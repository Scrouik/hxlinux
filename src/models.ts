import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  catalogPickerRowKey,
  findUsbAssignPickerLocation,
  findCabModelPickerLocation,
  getCatalogModelIdForCabSingleHex,
  getCatalogModelIdForCabDualCab2Hex,
  findIoSourceIdFromInputChainValues,
  findIoSourceIdByWireValue,
  findIoSourceRowById,
  findIoSourceRowByWireValue,
  findSplitSourceIdByCatalogModelId,
  findSplitSourceIdByWireValue,
  splitWireFromChainHex,
  splitChainHexFromWire,
  formatSubCategoryForHeader,
  getCatalogModelIdForHex,
  getCatalogModelImageForId,
  getCatalogModelNameForId,
  getUsbAssignPickerData,
  getPresetMetaForId,
  ioSourceMatchesConnectedDevice,
  modelsDefinitionFileBasesFromUsbAssign,
  cabHexFromAmpCabWire,
  cabDualWireParts,
  isCabDualWireHex,
  isAmpCabFamilySlotCategory,
  isAmpCabLegacySlotCategory,
  isLegacyCabChainHex,
  moduleHexForUsbVariant,
  ampCabHexPairFromAssignVariant,
  cabDualHexPairFromAssignVariant,
  pickBasedOn,
  pickSignal,
  usbAssignVariantForAmpCabSlot,
  usbAssignVariantFromPresetMeta,
  type CatalogPickerData,
  type CatalogPickerModelRow,
  type PresetMetaJson,
} from "./hxModelCatalogMeta";
import { hwUi } from "./hwUiRefresh";
import "./styles.css";

type SlotDebug = {
  category: string;
  name: string;
  gridX?: string;
  gridY?: string;
  /** Hex module preset (Rust), pour routage mono/stéréo via chainHex parallèles. */
  moduleHex?: string;
  /** Cab 2 lu dans la trame scroll (`c219`) — prioritaire sur le défaut assign `dual`. */
  cabHexHint?: string;
  /** ID catalogue imposé (jointure stricte par ID, sans fallback nom). */
  catalogModelId?: string;
  /** Type structurel Kempline (`00..03`) pour I/O, distinct d'un chainHex modèle. */
  slotTypeHex?: string;
};

type RoutingMarker = {
  category: string;
  name: string;
  moduleHex?: string;
};

type HardwareActiveSlotState = {
  slotIndex: number | null;
  slotBus: number | null;
  sequence: number;
};

/** Payload Rust `models:hardware-slot-changed` (camelCase) — slot actif hardware a changé de bus. */
type HardwareSlotChangedPayload = {
  sequence: number;
  slotIndex: number | null;
  slotBus: number | null;
};

/** Contenu du slot actif modifié (empreinte IN focus) — surveillance slot-only, sans preset_data. */
type SlotContentChangedPayload = {
  sequence: number;
  slotIndex: number;
  kind: "content";
  capsuleSig?: string;
};

/** Paramètre live modifié sur le hardware (IN `85:62…1c:PP:77`, parse passif). */
type SlotParamChangedPayload = {
  sequence: number;
  slotIndex: number;
  slotBus: number;
  paramIndex: number;
  valueType: "float" | "bool" | "discrete" | string;
  value: ChainParamValueJson;
};

/** Modèle changé sur le hardware (notif `1f` + pull `1b`/`19`/`19`, parse bulk IN). */
type SlotModelHwChangedPayload = {
  sequence: number;
  slotIndex: number;
  slotBus: number;
  moduleHex: string | null;
  /** Catégorie lue sur le fil (ex. Amp+Cab) — prioritaire sur le catalogue seul. */
  categoryHint?: string | null;
  /** Hex cab (bloc c219 ou fil combiné) — legacy IR vs hybrid au scroll. */
  cabHexHint?: string | null;
};

/** Scroll / echo Input Path 1 : `@input` wire (1 / 4 / 6 Stomp) depuis IN `82:62:00:33:XX`. */
type Path1InputSourceChangedPayload = {
  wireValue: number;
  fromScroll21: boolean;
};

/** Type Split Path 1 — select UI, scroll ed03 ou IN `21` (`split scroll.json`). */
type Path1SplitTypeChangedPayload = {
  wireValue: number;
  fromScroll21: boolean;
};

type SlotFocusSyncResponse = {
  slotIndex: number;
  contentChange?: SlotContentChangedPayload | null;
  inFrameCount?: number;
};

let currentPresetIndex = -1;
let loadedPresetIndex = -1;
let loading = false;
let pendingPresetIndex = -1;
let lastRequestedPresetIndex = -1;
const ENABLE_PRESET_CONTENT = true;
const DEBUG_MODEL_ID_JOIN_FALLBACK =
  localStorage.getItem("models_debug_id_join") === "1";
let debugRoutingMode = localStorage.getItem("models_debug_routing") === "1";
let connectedDeviceName: string | null = null;

const statusEl = document.getElementById("status");

function setStatus(text: string) {
  if (statusEl) statusEl.textContent = text;
}
const presetLabelEl = document.getElementById("preset-label") as HTMLElement;
const contentEl = document.getElementById("content") as HTMLElement;
const LIVE_WRITE_PROBE_FLAG = "models_live_write_probe";
const LIVE_WRITE_ENABLED_FLAG = "models_live_write_enabled";
const LIVE_WRITE_TRANSPORT_KEY = "models_live_write_transport";
const LIVE_WRITE_MIDI_CC_KEY = "models_live_midi_cc";
const LIVE_WRITE_MIDI_CHANNEL_KEY = "models_live_midi_channel";
const LIVE_WRITE_SYNC_PAUSE_MS = 1200;
const HW_SYNC_INTERVAL_MS_KEY = "models_hw_sync_interval_ms";
/**
 * Throttle optionnel entre deux appels **event-driven** de `runHardwareSyncSoftRefresh`
 * (ex. deux `models:hardware-slot-changed` rapprochés). **Pas de poll périodique** par défaut.
 * **Par défaut : 0** (pas de throttle) — voir `docs/models-hardware-sync.md`.
 * Throttle explicite : `localStorage.setItem("models_hw_sync_interval_ms", "200")`.
 * Re-dump preset USB optionnel (timer dédié) : `models_hw_usb_preset_poll_ms` (ex. `2500`, min 500).
 */
const HW_USB_PRESET_POLL_MS_KEY = "models_hw_usb_preset_poll_ms";
const HW_USB_PRESET_POLL_MIN_MS = 500;
const HW_USB_PRESET_POLL_MAX_MS = 120000;
/** `models_debug_hw_slot_sync=1` : logs `[HwSlotSync]` (dont succès `sync_hardware_slot_focus_usb`) ; avec **`models_debug_sync_trace`** ou ce flag : `console.info` sur `models:hardware-slot-changed`. */
const DEBUG_HW_SLOT_SYNC_FLAG = "models_debug_hw_slot_sync";
/**
 * Ancien comportement (debug / secours) : sur notif « slot actif hardware », forcer encore un
 * `request_preset_content` immédiat. Par défaut (clé absente ou ≠ `1`) : pas de dump preset sur
 * changement de slot — panneau / sélection depuis le snapshot ; grille seulement après relecture.
 */
const HW_FORCE_PRESET_DUMP_ON_SLOT_NOTIFY_KEY = "models_hw_force_preset_dump_on_slot_notify";
/**
 * `localStorage.setItem("models_hw_slot_focus_usb", "0")` désactive l’OUT « focus slot » HX Edit
 * (`sync_hardware_slot_focus_usb`) après notif slot. Défaut : actif.
 */
const HW_SLOT_FOCUS_USB_KEY = "models_hw_slot_focus_usb";
/**
 * Surveillance périodique du slot hardware actif (focus USB + diff empreinte).
 * Défaut 1200 ms ; `localStorage.setItem("models_hw_slot_content_watch_ms", "0")` pour désactiver.
 */
const HW_SLOT_CONTENT_WATCH_MS_KEY = "models_hw_slot_content_watch_ms";
const HW_SLOT_CONTENT_WATCH_MIN_MS = 400;
const HW_SLOT_CONTENT_WATCH_MAX_MS = 30000;
/** Log console lorsque le `moduleHex` d’un slot change après sync USB (jointure catalogue). */
const DEBUG_CATALOG_CHAINHEX_FLAG = "models_debug_catalog_chainhex";
/** `localStorage.setItem("models_debug_sync_trace", "1")` → logs `[ModelsSync]…` ; lignes répétitives throttlées (`emitModelsSyncTraceThrottled`). */
const MODELS_SYNC_TRACE_FLAG = "models_debug_sync_trace";
const HW_SYNC_MIN_MS = 100;
const HW_SYNC_MAX_MS = 5000;
const REQUEST_PRESET_MIN_GAP_MS = 320;
const REQUEST_PRESET_RECOVERY_DELAY_MS = 800;
const REQUEST_PRESET_SOFT_STALL_TRIES = 18;
const REQUEST_PRESET_HARD_RECOVERY_AFTER = 4;
let lastHardwareSyncAt = 0;
let lastSlotContentWatchAt = 0;
/** Dernier `request_preset_content` réussi déclenché par le soft-sync (pas les chargements utilisateur). */
let lastSoftUsbPresetReadAt = 0;
/** Dump preset USB immédiat hors chargement preset : sondage modèle slot, ou opt-in sur notif slot. */
let pendingForceUsbPresetContent = false;
let hardwareSyncBusy = false;
/** Pendant `probe_slot_model_usb` (MAJ optimiste) : pas de soft-sync qui relirait d’anciens slots. */
let slotModelUsbProbeInFlight: number | null = null;
/** Contexte assign picker (Amp+Cab) pendant `probe_slot_model_usb` + merge grace. */
let lastProbePickerAssignContext: {
  ki: number;
  catalogModelId: string;
  assignVariant: string;
  category: string;
  /** Cab 2 réel après probe / collage matrice (preset_data peut être en retard). */
  cabDualCab2ModelId?: string | null;
  /** Cab Amp+Cab réel après probe / collage matrice. */
  ampCabCabModelId?: string | null;
  ampCabCabAssignVariant?: string | null;
} | null = null;
/**
 * Après `probe_slot_model_usb` réussi sans re-dump preset : le parse `get_active_preset_slots` peut
 * encore refléter l’ancien `preset_data` Rust. On garde la ligne optimiste pour ce slot un court instant.
 */
let mergeProbeSlotModelUntil: {
  ki: number;
  /** Autres slots à préserver (ex. move drag & drop : source vide + destination remplie). */
  extraKis?: number[];
  deadline: number;
  /** Évite de spammer `emitModelsSyncTrace` sur soft-sync répétés. */
  mergeTraceEmitted?: boolean;
} | null = null;
const PROBE_SLOT_MERGE_GRACE_MS = 20_000;

function armProbeSlotMergeGrace(...kis: number[]): void {
  const unique = [...new Set(kis.filter((k) => k >= 0 && k <= 15))];
  if (unique.length === 0) return;
  mergeProbeSlotModelUntil = {
    ki: unique[0]!,
    extraKis: unique.length > 1 ? unique.slice(1) : undefined,
    deadline: Date.now() + PROBE_SLOT_MERGE_GRACE_MS,
  };
}
/** Évite un `request_preset_content` (poll) collé au trafic `probe_slot_model_usb` → preset_data vidé → chainFetch null / hardwareSyncBusy long. */
let suppressUsbPresetPollUntilMs = 0;
const USB_PRESET_POLL_SUPPRESS_AFTER_PROBE_MS = 10_000;
let hardwareSyncPausedForPresetLoad = false;
let lastLiveWriteAt = 0;
let liveWriteUiInteractionUntil = 0;
let lastRequestPresetInvokeAt = 0;
let queuedPresetLoadTimer: number | null = null;
let recoveryPresetLoadTimer: number | null = null;
let recoveryAttemptCount = 0;
let loadingHeartbeatTimer: number | null = null;
let loadingHeartbeatBase = "";
let loadingHeartbeatPhase = 0;
let autoSelectFallbackTimer: number | null = null;
type PendingLiveWrite = {
  slotIndex: number;
  paramIndex: number;
  symbolicId: string;
  displayType: string | null;
  /** Line 6 `.models` : 2 = booléen (route write `23` côté Rust). */
  valueType: number | null;
  /** Valeur chaîne / slider (ex. Ratio 0..5), avant normalisation USB. */
  rawValue: number;
  /** Si définis et max > min, `write_live_param` / MIDI reçoivent (raw-min)/(max-min) borné à 0..1. */
  rawMin: number | null;
  rawMax: number | null;
  /** Sous-bloc double : Amp+Cab (`amp`/`cab`) ou Cab dual (`cab1`/`cab2`). */
  dualPart?: "amp" | "cab" | "cab1" | "cab2" | null;
  ampCabAssignVariant?: string | null;
  /** Nombre de params visibles du panneau Amp (route legacy guitar vs compact). */
  ampCabAmpParamCount?: number | null;
  /** Cab dual IR (`dual`) vs hybrid legacy (`dual-legacy`) — route bloc modèle live write. */
  cabDualAssignVariant?: string | null;
};
const pendingLiveWrites = new Map<string, PendingLiveWrite>();
/**
 * Paramètres modifiés en live write alors que `preset_data` (RAM) n’est pas encore à jour :
 * fusion au prochain rendu / patch du panneau pour éviter l’affichage de vieilles valeurs au retour sur le slot.
 */
const liveChainParamOverridesByPresetSlot = new Map<string, Map<string, ChainParamValueJson>>();

/** Chaîne params par slot — hydratée **une fois** au chargement preset depuis `preset_data`, puis jamais relue. */
const slotChainSessionByKey = new Map<string, ChainParamValueJson[]>();

type DualSlotPartJson = {
  chainHex: string;
  category: string;
  name: string;
  modelId: string;
  values: ChainParamValueJson[];
};

type DualSlotPartsJson = {
  kind: "amp_cab" | "cab_dual";
  parts: DualSlotPartJson[];
};

/** Amp+Cab / Cab dual par slot — hydraté au chargement preset, puis session + trame HW (pas de relecture `preset_data`). */
const slotDualPartsSessionByKey = new Map<string, DualSlotPartsJson>();

function liveChainOverrideStorageKey(preset: number, kemplineSlotIndex: number): string {
  return `${preset}|${kemplineSlotIndex}`;
}

function recordLiveChainParamOverrideForKemplineSlot(
  preset: number,
  kemplineSlotIndex: number,
  symbolicId: string,
  value: ChainParamValueJson,
): void {
  if (preset < 0 || !Number.isInteger(kemplineSlotIndex) || kemplineSlotIndex < 0) return;
  const sid = symbolicId.trim();
  if (!sid) return;
  const key = liveChainOverrideStorageKey(preset, kemplineSlotIndex);
  let m = liveChainParamOverridesByPresetSlot.get(key);
  if (!m) {
    m = new Map();
    liveChainParamOverridesByPresetSlot.set(key, m);
  }
  m.set(sid, value);
}

function clearLiveChainOverridesForKemplineSlot(preset: number, kemplineSlotIndex: number): void {
  liveChainParamOverridesByPresetSlot.delete(liveChainOverrideStorageKey(preset, kemplineSlotIndex));
  slotChainSessionByKey.delete(liveChainOverrideStorageKey(preset, kemplineSlotIndex));
}

function clearAllLiveChainParamOverrides(): void {
  liveChainParamOverridesByPresetSlot.clear();
  slotChainSessionByKey.clear();
  slotDualPartsSessionByKey.clear();
}

function clearSlotChainSessionForPreset(preset: number): void {
  const prefix = `${preset}|`;
  for (const k of [...slotChainSessionByKey.keys()]) {
    if (k.startsWith(prefix)) slotChainSessionByKey.delete(k);
  }
}

function clearSlotDualPartsSessionForPreset(preset: number): void {
  const prefix = `${preset}|`;
  for (const k of [...slotDualPartsSessionByKey.keys()]) {
    if (k.startsWith(prefix)) slotDualPartsSessionByKey.delete(k);
  }
}

function clearSlotDualPartsSessionForKemplineSlot(
  preset: number,
  kemplineSlotIndex: number,
): void {
  if (preset < 0 || kemplineSlotIndex < 0 || kemplineSlotIndex > 15) return;
  slotDualPartsSessionByKey.delete(liveChainOverrideStorageKey(preset, kemplineSlotIndex));
}

function setSlotChainSessionValues(
  preset: number,
  kemplineSlotIndex: number,
  values: ChainParamValueJson[],
): void {
  if (preset < 0 || kemplineSlotIndex < 0 || kemplineSlotIndex > 15 || values.length === 0) return;
  slotChainSessionByKey.set(liveChainOverrideStorageKey(preset, kemplineSlotIndex), values.slice());
}

/** **Seul** appel autorisé à `get_active_preset_slot_chain_param_values` hors chargement preset. */
async function readChainValuesFromPresetDataOnce(slotIndex: number): Promise<ChainParamValueJson[] | null> {
  try {
    return await invoke<ChainParamValueJson[] | null>("get_active_preset_slot_chain_param_values", {
      slotIndex,
    });
  } catch {
    return null;
  }
}

/** Hydrate le cache session depuis `preset_data` — appelé une fois par chargement / changement de preset. */
async function hydrateSlotChainSessionFromPresetData(presetIndex: number): Promise<void> {
  if (presetIndex < 0) return;
  clearSlotChainSessionForPreset(presetIndex);
  let filled = 0;
  for (let ki = 0; ki < 16; ki += 1) {
    const vals = await readChainValuesFromPresetDataOnce(ki);
    if (vals && vals.length > 0) {
      slotChainSessionByKey.set(liveChainOverrideStorageKey(presetIndex, ki), vals);
      filled += 1;
    }
  }
  emitModelsSyncTrace(`hydrateSlotChainSession preset=${presetIndex} slots=${filled}/16`);
}

/** **Seul** appel autorisé à `get_active_preset_slot_dual_parts` (hydratation initiale preset). */
async function readDualPartsFromPresetDataOnce(
  slotIndex: number,
): Promise<DualSlotPartsJson | null> {
  try {
    return await invoke<DualSlotPartsJson | null>("get_active_preset_slot_dual_parts", {
      slotIndex,
    });
  } catch {
    return null;
  }
}

async function hydrateSlotDualPartsSessionFromPresetData(presetIndex: number): Promise<void> {
  if (presetIndex < 0) return;
  clearSlotDualPartsSessionForPreset(presetIndex);
  let filled = 0;
  for (let ki = 0; ki < 16; ki += 1) {
    const parts = await readDualPartsFromPresetDataOnce(ki);
    if (parts && parts.parts.length === 2) {
      slotDualPartsSessionByKey.set(liveChainOverrideStorageKey(presetIndex, ki), parts);
      filled += 1;
    }
  }
  emitModelsSyncTrace(`hydrateSlotDualPartsSession preset=${presetIndex} dualSlots=${filled}`);
}

function linkedCabHexFromSlot(slot: SlotDebug): string {
  return (slot.cabHexHint ?? "").trim() || cabHexFromAmpCabWire(slot.moduleHex) || "";
}

async function dualPartFromChainHex(
  chainHex: string,
  categoryHint: string,
  chainValues: ChainParamValueJson[] | null,
): Promise<DualSlotPartJson> {
  const hx = chainHex.trim();
  const modelId = (await getCatalogModelIdForHex(hx, categoryHint))?.trim() ?? "";
  const meta = modelId ? await getPresetMetaForId(modelId) : null;
  const name = ((modelId ? await getCatalogModelNameForId(modelId) : null) ?? "")?.trim() ?? "";
  const category = (meta?.categoryName ?? categoryHint).trim() || categoryHint;
  return { chainHex: hx, category, name, modelId, values: chainValues ?? [] };
}

async function buildAmpCabDualPartsFromWireAndSession(
  kemplineSlotIndex: number,
  slot: SlotDebug,
  ampCatalogModelId: string,
  assignVariant: string | null,
): Promise<DualSlotPartsJson | null> {
  const cabHexHint = linkedCabHexFromSlot(slot);
  const assign =
    (assignVariant ?? "").trim() ||
    (await usbAssignVariantForAmpCabSlot(
      await getPresetMetaForId(ampCatalogModelId),
      slot.moduleHex,
      slot.category,
      ampCatalogModelId,
      cabHexHint || null,
    ));
  if (assign !== "amp+cab" && assign !== "amp+cab-legacy") return null;

  const pair = await ampCabHexPairFromAssignVariant(ampCatalogModelId, assign);
  const wire = (slot.moduleHex ?? "").trim().toLowerCase();
  const sep = "1a";
  const si = wire.indexOf(sep);
  const ampHex = (si > 0 ? wire.slice(0, si) : pair?.ampHex ?? "").trim();
  const cabHex = (cabHexHint || (si > 0 ? wire.slice(si + sep.length) : "") || pair?.cabHex || "").trim();
  if (!ampHex || !cabHex) return null;

  const chainValues = await resolveChainValuesForKemplineSlot(
    kemplineSlotIndex,
    slot,
    ampCatalogModelId,
    slot.category,
    null,
  );
  const ampPart = await dualPartFromChainHex(ampHex, "Amp", chainValues);
  const cabModelId = (await getCatalogModelIdForHex(cabHex, "Cab"))?.trim() ?? "";
  const cabMeta = cabModelId ? await getPresetMetaForId(cabModelId) : null;
  const cabSignal = pickSignal(cabMeta, cabHex);
  const cabDef = cabModelId ? await findModelDefinitionBySymbolicId(cabModelId, "Cab") : null;
  const cabValues = cabDef
    ? buildDefaultChainValuesForSourceOrder(cabDef.entry.params ?? [], cabSignal)
    : [];
  const cabPart = await dualPartFromChainHex(cabHex, "Cab", cabValues);
  if (cabModelId) cabPart.modelId = cabModelId;

  return { kind: "amp_cab", parts: [ampPart, cabPart] };
}

async function buildCabDualPartsFromWireAndSession(
  kemplineSlotIndex: number,
  slot: SlotDebug,
  dualCatalogModelId: string,
): Promise<DualSlotPartsJson | null> {
  const wire = cabDualWireParts(slot.moduleHex);
  const pair = await cabDualHexPairFromAssignVariant(dualCatalogModelId, "dual");
  const cab1Hex = (wire?.cab1Hex ?? pair?.cab1Hex ?? "").trim();
  const cab2Hex = (
    await resolveCabDualCab2HexFromTrame(slot, kemplineSlotIndex)
  ).trim();
  if (!cab1Hex || !cab2Hex) return null;

  const chainValues = await resolveChainValuesForKemplineSlot(
    kemplineSlotIndex,
    slot,
    dualCatalogModelId,
    slot.category,
    null,
  );
  const cab1Part = await dualPartFromChainHex(cab1Hex, "Cab", chainValues);
  const cab2ModelId =
    (await getCatalogModelIdForCabDualCab2Hex(dualCatalogModelId, cab2Hex, cab1Hex))?.trim() ?? "";
  const cab2Meta = cab2ModelId ? await getPresetMetaForId(cab2ModelId) : null;
  const cab2Signal = pickSignal(cab2Meta, cab2Hex);
  const cab2Def = cab2ModelId
    ? await findModelDefinitionBySymbolicId(cab2ModelId, "Cab")
    : null;
  const cab2Values = cab2Def
    ? buildDefaultChainValuesForSourceOrder(cab2Def.entry.params ?? [], cab2Signal)
    : [];
  const cab2Part = await dualPartFromChainHex(cab2Hex, "Cab", cab2Values);
  if (cab2ModelId) cab2Part.modelId = cab2ModelId;

  return { kind: "cab_dual", parts: [cab1Part, cab2Part] };
}

type ResolveSlotDualPartsOpts = {
  slot?: SlotDebug | null;
  catalogModelId?: string | null;
  kind?: "amp_cab" | "cab_dual";
  assignVariant?: string | null;
};

async function slotDualPartsCacheStillValid(
  cached: DualSlotPartsJson,
  kemplineSlotIndex: number,
  opts: ResolveSlotDualPartsOpts,
): Promise<boolean> {
  const slot = opts.slot ?? lastHwSyncNormalizedSlots?.[kemplineSlotIndex] ?? null;
  if (!slot) return false;
  const catalogId =
    (opts.catalogModelId ?? slot.catalogModelId ?? "").trim() ||
    (slot.moduleHex
      ? (await getCatalogModelIdForHex(slot.moduleHex, slot.category))?.trim()
      : "") ||
    "";
  const meta = catalogId ? await getPresetMetaForId(catalogId) : null;
  if (cached.kind === "cab_dual") {
    if (!slotWantsCabDualTabs(slot, opts.assignVariant, meta)) return false;
    const cachedDualId = cached.parts[0]?.modelId?.trim().toLowerCase() ?? "";
    if (
      catalogId &&
      cachedDualId &&
      catalogId.toLowerCase() !== cachedDualId &&
      !isCabDualWireHex(slot.moduleHex)
    ) {
      return false;
    }
    return true;
  }
  if (cached.kind === "amp_cab") {
    if (!slotWantsAmpCabDualTabs(slot, opts.assignVariant)) return false;
    const cachedAmpId = cached.parts[0]?.modelId?.trim().toLowerCase() ?? "";
    if (catalogId && cachedAmpId && catalogId.toLowerCase() !== cachedAmpId) return false;
    return true;
  }
  return false;
}

/** Dual parts en session : cache preset (hydratation) puis trame HW + catalogue — jamais `preset_data` en runtime. */
async function resolveSlotDualParts(
  kemplineSlotIndex: number,
  opts: ResolveSlotDualPartsOpts = {},
): Promise<DualSlotPartsJson | null> {
  if (loadedPresetIndex !== currentPresetIndex || currentPresetIndex < 0) return null;
  if (kemplineSlotIndex < 0 || kemplineSlotIndex > 15) return null;

  const key = liveChainOverrideStorageKey(currentPresetIndex, kemplineSlotIndex);
  const cached = slotDualPartsSessionByKey.get(key);
  if (cached) {
    if (await slotDualPartsCacheStillValid(cached, kemplineSlotIndex, opts)) {
      return cached;
    }
    slotDualPartsSessionByKey.delete(key);
  }

  const slot = opts.slot ?? lastHwSyncNormalizedSlots?.[kemplineSlotIndex] ?? null;
  const catalogId =
    (opts.catalogModelId ?? slot?.catalogModelId ?? "").trim() ||
    (slot?.moduleHex
      ? (await getCatalogModelIdForHex(slot.moduleHex, slot.category))?.trim()
      : "") ||
    "";
  if (!slot || !catalogId) return null;

  let kind = opts.kind;
  if (!kind) {
    const meta = await getPresetMetaForId(catalogId);
    if (slotWantsAmpCabDualTabs(slot, opts.assignVariant)) kind = "amp_cab";
    else if (slotWantsCabDualTabs(slot, opts.assignVariant, meta)) kind = "cab_dual";
    else return null;
  }

  const built =
    kind === "amp_cab"
      ? await buildAmpCabDualPartsFromWireAndSession(
          kemplineSlotIndex,
          slot,
          catalogId,
          opts.assignVariant ?? null,
        )
      : await buildCabDualPartsFromWireAndSession(kemplineSlotIndex, slot, catalogId);

  if (built) slotDualPartsSessionByKey.set(key, built);
  return built;
}

function syncSlotDualPartsSessionFromTabPanes(
  kemplineSlotIndex: number,
  kind: "amp_cab" | "cab_dual",
  panes: DualTabPaneConfig[],
  slot: SlotDebug,
  linkedCabHex?: string | null,
): void {
  if (currentPresetIndex < 0 || panes.length !== 2) return;
  const wire = kind === "amp_cab" ? (slot.moduleHex ?? "").trim().toLowerCase() : "";
  const sep = "1a";
  const si = wire.indexOf(sep);
  const ampHex = kind === "amp_cab" && si > 0 ? wire.slice(0, si) : "";
  const cabFromWire =
    kind === "amp_cab"
      ? linkedCabHex?.trim() || (si > 0 ? wire.slice(si + sep.length) : "") || linkedCabHexFromSlot(slot)
      : "";
  const cabDualWire = kind === "cab_dual" ? cabDualWireParts(slot.moduleHex) : null;
  const parts: DualSlotPartJson[] = panes.map((p, i) => {
    let chainHex = "";
    if (kind === "amp_cab") {
      chainHex = i === 0 ? ampHex : cabFromWire;
    } else if (cabDualWire) {
      chainHex = i === 0 ? cabDualWire.cab1Hex : cabDualWire.cab2Hex;
    }
    return {
      chainHex,
      category: kind === "amp_cab" ? (i === 0 ? "Amp" : "Cab") : "Cab",
      name: p.modelTitle.trim() || "—",
      modelId: p.catalogModelId?.trim() ?? "",
      values: p.chainValues ?? [],
    };
  });
  slotDualPartsSessionByKey.set(liveChainOverrideStorageKey(currentPresetIndex, kemplineSlotIndex), {
    kind,
    parts,
  });
}

async function patchSlotDualPartsSessionAmpCabCab(
  kemplineSlotIndex: number,
  cabModelId: string,
  cabHex: string,
): Promise<void> {
  if (currentPresetIndex < 0) return;
  const key = liveChainOverrideStorageKey(currentPresetIndex, kemplineSlotIndex);
  const existing = slotDualPartsSessionByKey.get(key);
  const cabName = ((await getCatalogModelNameForId(cabModelId)) ?? "")?.trim() ?? "";
  if (existing?.kind === "amp_cab" && existing.parts.length === 2) {
    const cabPart = { ...existing.parts[1]! };
    cabPart.modelId = cabModelId.trim();
    cabPart.chainHex = cabHex.trim() || cabPart.chainHex;
    cabPart.name = cabName || cabPart.name;
    slotDualPartsSessionByKey.set(key, {
      kind: "amp_cab",
      parts: [existing.parts[0]!, cabPart],
    });
    return;
  }
  const slot = lastHwSyncNormalizedSlots?.[kemplineSlotIndex];
  if (!slot) return;
  const ampId =
    (slot.catalogModelId ?? "").trim() ||
    (await getCatalogModelIdForHex(slot.moduleHex, slot.category))?.trim() ||
    "";
  if (!ampId) return;
  const built = await buildAmpCabDualPartsFromWireAndSession(kemplineSlotIndex, slot, ampId, null);
  if (!built) return;
  const cabPart = { ...built.parts[1]! };
  cabPart.modelId = cabModelId.trim();
  cabPart.chainHex = cabHex.trim() || cabPart.chainHex;
  cabPart.name = cabName || cabPart.name;
  slotDualPartsSessionByKey.set(key, { kind: "amp_cab", parts: [built.parts[0]!, cabPart] });
}

async function cabDefaultChainValuesForCatalogModelId(
  modelId: string,
  chainHex?: string | null,
): Promise<ChainParamValueJson[]> {
  const id = modelId.trim();
  if (!id) return [];
  const def = await findModelDefinitionBySymbolicId(id, "Cab");
  const meta = await getPresetMetaForId(id);
  const signal = pickSignal(meta, chainHex ?? undefined);
  return def
    ? buildDefaultChainValuesForSourceOrder(def.entry.params ?? [], signal)
    : [];
}

async function patchSlotDualPartsSessionCabDualCab2(
  kemplineSlotIndex: number,
  cab2ModelId: string,
  cab2Hex: string,
): Promise<void> {
  if (currentPresetIndex < 0) return;
  const key = liveChainOverrideStorageKey(currentPresetIndex, kemplineSlotIndex);
  const existing = slotDualPartsSessionByKey.get(key);
  const cabName = ((await getCatalogModelNameForId(cab2ModelId)) ?? "")?.trim() ?? "";
  const cab2HexTrim = cab2Hex.trim();
  const cab2Defaults = await cabDefaultChainValuesForCatalogModelId(
    cab2ModelId,
    cab2HexTrim || null,
  );
  if (existing?.kind === "cab_dual" && existing.parts.length === 2) {
    const cab2Part = { ...existing.parts[1]! };
    cab2Part.modelId = cab2ModelId.trim();
    cab2Part.chainHex = cab2HexTrim || cab2Part.chainHex;
    cab2Part.name = cabName || cab2Part.name;
    cab2Part.values = cab2Defaults;
    slotDualPartsSessionByKey.set(key, {
      kind: "cab_dual",
      parts: [existing.parts[0]!, cab2Part],
    });
    return;
  }
  const slot = lastHwSyncNormalizedSlots?.[kemplineSlotIndex];
  if (!slot) return;
  const dualId =
    (slot.catalogModelId ?? "").trim() ||
    (await getCatalogModelIdForHex(slot.moduleHex, slot.category))?.trim() ||
    "";
  if (!dualId) return;
  const built = await buildCabDualPartsFromWireAndSession(kemplineSlotIndex, slot, dualId);
  if (!built) return;
  const cab2Part = { ...built.parts[1]! };
  cab2Part.modelId = cab2ModelId.trim();
  cab2Part.chainHex = cab2HexTrim || cab2Part.chainHex;
  cab2Part.name = cabName || cab2Part.name;
  cab2Part.values = cab2Defaults;
  slotDualPartsSessionByKey.set(key, { kind: "cab_dual", parts: [built.parts[0]!, cab2Part] });
}

async function resolveChainValuesForKemplineSlot(
  kemplineSlotIndex: number,
  slot: SlotDebug,
  catalogModelIdTrimmed: string,
  categoryName: string | null,
  catalogRoutingSignal: string | null,
): Promise<ChainParamValueJson[] | null> {
  const found = await findModelDefinitionForSlot(slot, catalogModelIdTrimmed, categoryName);
  const params = found?.entry.params ?? [];
  if (params.length === 0) return null;

  const sessionKey = liveChainOverrideStorageKey(currentPresetIndex, kemplineSlotIndex);
  let base = slotChainSessionByKey.get(sessionKey) ?? null;
  if (!base || base.length === 0) {
    base = buildDefaultChainValuesForSourceOrder(params, catalogRoutingSignal);
  }
  if (!base || base.length === 0) return null;

  const om = liveChainParamOverridesByPresetSlot.get(sessionKey);
  if (!om || om.size === 0) return base;

  const bySid = new Map<string, ChainParamValueJson>();
  const source = modelParamSourceOrderIds(params, catalogRoutingSignal, base.length);
  const n = Math.min(base.length, source.length);
  for (let i = 0; i < n; i += 1) {
    const sid = source[i];
    if (sid) bySid.set(sid, base[i]!);
  }
  for (const [sid, v] of om) bySid.set(sid, v);
  return chainValuesUsbOrderFromSymbolicMap(bySid, params, catalogRoutingSignal);
}

function mergeLiveChainOverridesIntoAligned(
  preset: number,
  kemplineSlotIndex: number | undefined,
  paramsForDisplay: ModelParamDefJson[],
  chainAligned: Array<ChainParamValueJson | undefined> | null | undefined,
): Array<ChainParamValueJson | undefined> | null {
  if (
    kemplineSlotIndex === undefined ||
    !Number.isInteger(kemplineSlotIndex) ||
    kemplineSlotIndex < 0 ||
    preset < 0
  ) {
    return chainAligned ?? null;
  }
  const om = liveChainParamOverridesByPresetSlot.get(
    liveChainOverrideStorageKey(preset, kemplineSlotIndex),
  );
  if (!om || om.size === 0) return chainAligned ?? null;
  const base = (chainAligned?.slice() as Array<ChainParamValueJson | undefined>) ?? [];
  const n = paramsForDisplay.length;
  while (base.length < n) base.push(undefined);
  for (let i = 0; i < n; i += 1) {
    const sid = (paramsForDisplay[i]?.symbolicID ?? "").trim();
    if (!sid || !om.has(sid)) continue;
    base[i] = om.get(sid);
  }
  return base;
}

function liveWriteUsbNormalized01(w: PendingLiveWrite): number {
  const lo = w.rawMin;
  const hi = w.rawMax;
  const v = w.rawValue;
  if (
    lo !== null &&
    hi !== null &&
    Number.isFinite(lo) &&
    Number.isFinite(hi) &&
    hi > lo
  ) {
    return Math.max(0, Math.min(1, (v - lo) / (hi - lo)));
  }
  return Math.max(0, Math.min(1, v));
}
/** Signature `category|name|moduleHex|…` par slot — stable si seuls les paramètres changent sur le HX. */
let lastHwSyncChainSignature: string | null = null;
/** Copie des 16 slots grille (soft-sync / chargement) pour MAJ optimiste pastille + signature sans re-parse. */
let lastHwSyncNormalizedSlots: SlotDebug[] | null = null;
/** Anti-flash: exige deux cycles consécutifs avant rerender complet si la signature change. */
let pendingHwLayoutSignature: string | null = null;
/** Snapshot liste presets + preset actif (device) pour éviter MAJ label inutiles. */
let lastPresetNamesSig: string | null = null;
/**
 * Dernier événement "slot actif hardware" consommé côté UI.
 * Valeur négative = "pas encore aligné après un chargement preset" : le premier soft-sync
 * synchronise sans forcer un `request_preset_content` (évite un dump USB en rafale après chaque load).
 */
let lastSeenHardwareSlotSequence = 0;
/** Deux constats consécutifs requis avant de traiter active_preset ≠ preset vue models (évite flash). */
let devicePresetMismatchStreak = 0;
let mainWindowPresetDriftStreak = 0;
/** Slot à appliquer après rendu / sync si un nouvel événement hardware est détecté. */
let pendingHardwareSelectedKemplineSlotIndex: number | null = null;
/** Bus slot_bus pour les blocs spéciaux (Input/Output/Split/Merge) sans kempline index. */
let pendingHardwareSelectedSlotBus: number | null = null;
/** Évite de renvoyer un ordre hardware lors des clics programmatiques (restore/sync). */
let suppressNextUiSlotHardwareSwitch = false;
let lastUserHwSlotSwitchAt = 0;
let lastUserHwSlotSwitchIndex: number | null = null;
let lastCatalogChainHexSnapshotPresetIndex = -1;
let lastCatalogChainHexBySlot: string[] | null = null;

function hwSlotDebugEnabled(): boolean {
  return localStorage.getItem(DEBUG_HW_SLOT_SYNC_FLAG) === "1";
}

function hwSlotDebugLog(message: string): void {
  if (!hwSlotDebugEnabled()) return;
  console.log(`[HwSlotSync] ${message}`);
}

function forcePresetDumpOnHardwareSlotNotify(): boolean {
  return localStorage.getItem(HW_FORCE_PRESET_DUMP_ON_SLOT_NOTIFY_KEY) === "1";
}

function slotFocusUsbSyncEnabled(): boolean {
  return localStorage.getItem(HW_SLOT_FOCUS_USB_KEY) !== "0";
}

function getSlotContentWatchIntervalMs(): number {
  const raw = (localStorage.getItem(HW_SLOT_CONTENT_WATCH_MS_KEY) ?? "").trim();
  if (raw === "0") return 0;
  if (!raw) return 1200;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed < 0) return 0;
  if (parsed === 0) return 0;
  return Math.min(HW_SLOT_CONTENT_WATCH_MAX_MS, Math.max(HW_SLOT_CONTENT_WATCH_MIN_MS, parsed));
}

function setModelsParamsBrowsingMode(browsing: boolean): void {
  const inner = getModelsParamsInner();
  if (inner) inner.classList.toggle("models-params-browsing", browsing);
  const pane = document.getElementById("models-params-pane");
  if (pane) pane.classList.toggle("models-params-browsing", browsing);
}

/** Soft-sync périodique : sync params si idle (pas de debounce qui repousse sans fin). */
function scheduleSoftRefreshParamsPaneFromSlots(slots: SlotDebug[]): void {
  hwUi.runParamsSyncWhenIdle("params", () => softRefreshParamsPaneFromSlots(slots));
}

/** Index Kempline du slot effet actif (UI ou pending sync hardware). */
function activeEffectKemplineSlotIndex(): number | null {
  const ki =
    selectedParamsKemplineSlotIndex ?? pendingHardwareSelectedKemplineSlotIndex;
  if (ki === null || ki < 0 || ki > 15) return null;
  return ki;
}

const hwModelHexCatalogCache = new Map<
  string,
  { catalogModelIdTrimmed: string; slot: SlotDebug }
>();

const MODELS_DEBUG_HW_MODEL_FAST_FLAG = "models_debug_hw_model_fast";

let hwModelSettleGeneration = 0;
let lastCompletedHwParamsHeavyKey = "";
/** `id` + variante assign (ex. amp+cab-legacy) — pas l’id seul. */
let lastHwPickerSyncKey: string | null = null;
/** Dernier event modèle HW pendant un geste (utilisé au settle). */
let pendingHwModelSettle: { payload: SlotModelHwChangedPayload; ki: number } | null = null;

function hwModelFastDebugEnabled(): boolean {
  return localStorage.getItem(MODELS_DEBUG_HW_MODEL_FAST_FLAG) === "1";
}

function scheduleHwModelSettleJob(job: () => void | Promise<void>): void {
  const gen = ++hwModelSettleGeneration;
  hwUi.scheduleAfterHwGesture("params", async () => {
    if (gen !== hwModelSettleGeneration) {
      emitModelsSyncTraceThrottled(
        "hw_model_settle_stale",
        `settle modèle ignoré (geste plus récent) gen=${gen} cur=${hwModelSettleGeneration}`,
        800,
      );
      return;
    }
    await job();
  });
}

/** Aperçu scroll : pas d’await catalogue ; cache hex si déjà résolu. */
function slotDebugPreviewFromHex(hex: string): SlotDebug {
  if (!hex) {
    return { category: "", name: "<empty>" };
  }
  const cached = hwModelHexCatalogCache.get(hex);
  if (cached) {
    return cached.slot;
  }
  const label = hex.length <= 6 ? hex.toUpperCase() : `…${hex.slice(-6).toUpperCase()}`;
  return { category: "…", name: label, moduleHex: hex };
}

function scrollLinkedCabHex(p: SlotModelHwChangedPayload, moduleHex: string): string {
  return (p.cabHexHint ?? "").trim() || cabHexFromAmpCabWire(moduleHex) || "";
}

/** Cab 2 depuis trame scroll : fil `c319` puis hint `c219` si suffixe usine encore sur le fil. */
function scrollCabDualCab2Hex(p: SlotModelHwChangedPayload, moduleHex: string): string {
  const wireCab2 = cabDualWireParts(moduleHex)?.cab2Hex ?? "";
  return cabDualEffectiveCab2Hex(wireCab2, p.cabHexHint);
}

async function slotDebugFromHwModelPayload(p: SlotModelHwChangedPayload): Promise<{
  slot: SlotDebug;
  catalogModelIdTrimmed: string;
  hex: string;
  cabHex: string;
}> {
  const hex = (p.moduleHex ?? "").trim();
  const categoryHint = (p.categoryHint ?? "").trim();
  const cabHex = scrollLinkedCabHex(p, hex);
  const cacheKey = [hex, categoryHint, cabHex].filter(Boolean).join("\0").toLowerCase();
  let catalogModelIdTrimmed = "";
  if (hex) {
    const cached = hwModelHexCatalogCache.get(cacheKey);
    if (cached) {
      return {
        hex,
        cabHex,
        catalogModelIdTrimmed: cached.catalogModelIdTrimmed,
        slot: cached.slot,
      };
    }
    const id = await getCatalogModelIdForHex(hex, categoryHint);
    catalogModelIdTrimmed = (id ?? "").trim();
    const meta = catalogModelIdTrimmed ? await getPresetMetaForId(catalogModelIdTrimmed) : null;
    const catalogName = catalogModelIdTrimmed
      ? await getCatalogModelNameForId(catalogModelIdTrimmed)
      : null;
    const displayName = (catalogName ?? "").trim() || hex;
    let categoryName =
      categoryHint || (meta?.categoryName ?? "").trim() || "?";
    const slotCatNorm = categoryName.trim().toLowerCase().replace(/\s+/g, "");
    if (
      (slotCatNorm === "amp+cab" || slotCatNorm === "ampcab") &&
      cabHex &&
      (await isLegacyCabChainHex(cabHex))
    ) {
      categoryName = "Amp+Cab Legacy";
    }
    const slot: SlotDebug = {
      category: categoryName,
      name: displayName,
      moduleHex: hex,
      catalogModelId: catalogModelIdTrimmed || undefined,
      cabHexHint: scrollCabDualCab2Hex(p, hex) || undefined,
    };
    hwModelHexCatalogCache.set(cacheKey, { catalogModelIdTrimmed, slot });
    return { hex, cabHex, catalogModelIdTrimmed, slot };
  }
  return {
    hex,
    cabHex,
    catalogModelIdTrimmed,
    slot: { category: "", name: "<empty>" },
  };
}

/** Scroll modèle : matrice + titre seulement (pas de picker / pas de catalogue). */
function applyHardwareSlotModelVisualLight(ki: number, slot: SlotDebug): void {
  if (shouldSkipHwSlotModelVisualOverwrite(ki)) {
    emitModelsSyncTraceThrottled(
      "hw_model_visual_skip_probe_grace",
      `visual light skip slot=${ki} (probe merge / picker)`,
      2_000,
    );
    return;
  }
  hwUi.runImmediate("grid", () => {
    if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
      const next = lastHwSyncNormalizedSlots.map((s, i) => (i === ki ? { ...slot } : { ...s }));
      lastHwSyncNormalizedSlots = next;
      lastHwSyncChainSignature = chainLayoutSignature(next);
    }
    patchMatrixSlotVisualFromSlot(ki, slot);
    setModelsParamsPaneModelNamePreview(slot.name?.trim() ?? "");
  });
}

/** Après settle : grille + picker + noms catalogue. */
function applyHardwareSlotModelVisualFast(
  ki: number,
  slot: SlotDebug,
  catalogModelIdTrimmed: string,
  cabHexHint?: string | null,
): void {
  if (shouldSkipHwSlotModelVisualOverwrite(ki)) {
    emitModelsSyncTraceThrottled(
      "hw_model_visual_fast_skip_probe_grace",
      `visual fast skip slot=${ki} (probe merge / picker)`,
      2_000,
    );
    return;
  }
  hwUi.runImmediate("grid", () => {
    if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
      const next = lastHwSyncNormalizedSlots.map((s, i) => (i === ki ? { ...slot } : { ...s }));
      lastHwSyncNormalizedSlots = next;
      lastHwSyncChainSignature = chainLayoutSignature(next);
    }
    patchMatrixSlotVisualFromSlot(ki, slot);
    setModelsParamsPaneModelNamePreview(slot.name?.trim() ?? "");
  });

  if (catalogModelIdTrimmed) {
    hwUi.runImmediate("picker", () => {
      void mountModelsSlotPicker().then(async () => {
        if (
          selectedParamsKemplineSlotIndex === ki &&
          (cabDualWireParts(slot.moduleHex) || isCabDualWireHex(slot.moduleHex))
        ) {
          await ensureCabDualPickerSynced(cabDualActiveTab);
          return;
        }
        const meta = await getPresetMetaForId(catalogModelIdTrimmed);
        const cabHex = cabHexHint ?? cabHexFromAmpCabWire(slot.moduleHex);
        const assignVariant = await usbAssignVariantForAmpCabSlot(
          meta,
          slot.moduleHex,
          slot.category,
          catalogModelIdTrimmed,
          cabHex,
        );
        const syncKey = `${catalogModelIdTrimmed}\0${assignVariant}`;
        if (syncKey === lastHwPickerSyncKey) return;
        lastHwPickerSyncKey = syncKey;
        await syncModelsSlotPickerFromLoadedModel(
          catalogModelIdTrimmed,
          meta,
          slot.moduleHex,
          slot.category,
          cabHex,
          undefined,
          0,
          slot.name,
        );
      });
    });
  } else {
    lastHwPickerSyncKey = null;
  }
}

async function applyHardwareSlotModelParamsHeavy(
  ki: number,
  slot: SlotDebug,
  catalogModelIdTrimmed: string,
  hex: string,
): Promise<void> {
  const probeMerge = mergeProbeSlotModelUntil;
  if (
    probeMerge &&
    Date.now() <= probeMerge.deadline &&
    (probeMerge.ki === ki || probeMerge.extraKis?.includes(ki))
  ) {
    emitModelsSyncTraceThrottled(
      "hw_params_heavy_skip_probe_grace",
      `params heavy skip slot=${ki} (merge grace probe picker)`,
      2_000,
    );
    return;
  }
  const optimistic = optimisticSlotDuringProbeMerge(ki);
  const effectiveSlot = optimistic ?? slot;
  const pickerCtx =
    lastProbePickerAssignContext && lastProbePickerAssignContext.ki === ki
      ? lastProbePickerAssignContext
      : null;
  if (currentPresetIndex >= 0) {
    clearLiveChainOverridesForKemplineSlot(currentPresetIndex, ki);
  }
  paramsPaneCatalogBySlotKey.delete(makeSlotSelectionKey(effectiveSlot, ki));
  const innerBefore = getModelsParamsInner();
  const alreadyDualTabs =
    innerBefore?.querySelector(".models-params-dual-tabs") !== null &&
    selectedParamsKemplineSlotIndex === ki;
  if (
    selectedParamsKemplineSlotIndex === ki &&
    selectedParamsPresetIndex === currentPresetIndex &&
    !alreadyDualTabs
  ) {
    selectedParamsValuesSig = null;
    selectedParamsInPlaceUpdater = null;
    selectedParamsInPlaceSlotKey = null;
    showModelsParamsLoading();
  }
  if (catalogModelIdTrimmed) {
    const meta = await getPresetMetaForId(catalogModelIdTrimmed);
    const signal = pickSignal(meta, effectiveSlot.moduleHex);
    let catalogOpts: { assignVariant: string } | undefined;
    if (pickerCtx) {
      catalogOpts = { assignVariant: pickerCtx.assignVariant };
    } else if (slotWantsAmpCabDualTabs(effectiveSlot, null)) {
      catalogOpts = {
        assignVariant: await usbAssignVariantForAmpCabSlot(
          meta,
          effectiveSlot.moduleHex,
          effectiveSlot.category,
          catalogModelIdTrimmed,
        ),
      };
    } else if (slotWantsCabDualTabs(effectiveSlot, null, meta)) {
      await loadAndShowModelsParamsForSlot(effectiveSlot, ki);
      const sessionVals = await resolveChainValuesForKemplineSlot(
        ki,
        effectiveSlot,
        catalogModelIdTrimmed,
        meta?.categoryName ?? null,
        signal,
      );
      if (sessionVals) setSlotChainSessionValues(currentPresetIndex, ki, sessionVals);
      return;
    }
    await loadAndShowModelsParamsFromCatalogDefaults(
      effectiveSlot,
      catalogModelIdTrimmed,
      ki,
      catalogOpts,
    );
    const sessionVals = await resolveChainValuesForKemplineSlot(
      ki,
      effectiveSlot,
      catalogModelIdTrimmed,
      meta?.categoryName ?? null,
      signal,
    );
    if (sessionVals) setSlotChainSessionValues(currentPresetIndex, ki, sessionVals);
    return;
  }
  if (!hex) {
    showModelsParamsNotFound(slot, null);
  } else {
    showModelsParamsError(
      `Jointure catalogue impossible pour chainHex « ${hex.toUpperCase()} ».`,
    );
  }
}

/**
 * Changement de modèle HW : aperçu léger à chaque event ; catalogue + picker + params au settle (~200 ms).
 * Bus Rust inchangé — debounce côté webview (voir logs `models_debug_hw_model_fast`).
 */
function applyHardwareSlotModelChanged(p: SlotModelHwChangedPayload): void {
  const activeKi = activeEffectKemplineSlotIndex();
  if (activeKi === null) {
    emitModelsSyncTraceThrottled(
      "evt_slot_model_hw_skip",
      "models:slot-model-changed ignoré : aucun slot effet actif UI",
      3_000,
    );
    return;
  }
  const ki = activeKi;
  const hex = (p.moduleHex ?? "").trim();
  pendingHwModelSettle = { payload: p, ki };

  applyHardwareSlotModelVisualLight(ki, slotDebugPreviewFromHex(hex));
  if (hwModelFastDebugEnabled()) {
    console.log(
      `[hwModelFast] preview seq=${p.sequence} hex=${hex || "(none)"} cache=${hwModelHexCatalogCache.has(hex)}`,
    );
  }

  scheduleHwModelSettleJob(async () => {
    const pending = pendingHwModelSettle;
    if (!pending || pending.ki !== ki) return;

    const t0 = hwModelFastDebugEnabled() ? performance.now() : 0;
    const { slot, catalogModelIdTrimmed, hex: settledHex, cabHex } =
      await slotDebugFromHwModelPayload(pending.payload);
    const tCatalog = hwModelFastDebugEnabled() ? performance.now() : 0;

    applyHardwareSlotModelVisualFast(ki, slot, catalogModelIdTrimmed, cabHex);
    const tVisual = hwModelFastDebugEnabled() ? performance.now() : 0;

    const heavyKey = `${currentPresetIndex}|${ki}|${catalogModelIdTrimmed}|${settledHex}`;
    if (
      heavyKey !== lastCompletedHwParamsHeavyKey ||
      selectedParamsKemplineSlotIndex !== ki ||
      !selectedParamsInPlaceUpdater
    ) {
      await applyHardwareSlotModelParamsHeavy(ki, slot, catalogModelIdTrimmed, settledHex);
      lastCompletedHwParamsHeavyKey = heavyKey;
    } else {
      emitModelsSyncTraceThrottled(
        "hw_params_heavy_skip_dup",
        `params heavy skip (déjà affiché) key=${heavyKey}`,
        1_500,
      );
    }

    if (hwModelFastDebugEnabled()) {
      const tEnd = performance.now();
      console.log(
        `[hwModelFast] settle seq=${pending.payload.sequence} hex=${settledHex || "(none)"} ` +
          `catalog=${Math.round(tCatalog - t0)}ms visual=${Math.round(tVisual - tCatalog)}ms ` +
          `params=${Math.round(tEnd - tVisual)}ms total=${Math.round(tEnd - t0)}ms`,
      );
    }
  });
}

async function applySlotContentWatchFromSync(
  snap: SlotFocusSyncResponse,
  ki: number,
): Promise<void> {
  const change = snap.contentChange;
  if (!change) return;
  emitModelsSyncTraceThrottled(
    "slot_content_usb",
    `slot watch slot=${ki} kind=${change.kind} capsule=${change.capsuleSig ?? "?"}`,
    3_000,
  );
  if (selectedParamsKemplineSlotIndex === ki && lastHwSyncNormalizedSlots) {
    scheduleSoftRefreshParamsPaneFromSlots(lastHwSyncNormalizedSlots);
  }
}

async function invokeSlotFocusWatch(ki: number): Promise<void> {
  if (!slotFocusUsbSyncEnabled()) return;
  if (ki < 0 || ki >= 16) return;
  try {
    const snap = await invoke<SlotFocusSyncResponse>("sync_hardware_slot_focus_usb", {
      slotIndex: ki,
    });
    await applySlotContentWatchFromSync(snap, ki);
  } catch (e) {
    emitModelsSyncTraceThrottled(
      "slot_content_watch_err",
      `slot content watch error slot=${ki} ${String(e)}`,
      5_000,
    );
  }
}

/** Focus USB périodique sur le slot actif (sans dump preset complet). */
async function maybeWatchActiveSlotContent(
  hw: HardwareActiveSlotState | null,
): Promise<void> {
  const interval = getSlotContentWatchIntervalMs();
  if (interval <= 0) return;
  if (!hw || !Number.isInteger(hw.slotIndex) || (hw.slotIndex as number) < 0) return;
  const ki = hw.slotIndex as number;
  if (ki >= 16) return;
  const now = Date.now();
  if (now - lastSlotContentWatchAt < interval) return;
  if (slotModelUsbProbeInFlight !== null) return;
  lastSlotContentWatchAt = now;
  await invokeSlotFocusWatch(ki);
}

/**
 * Applique l’état « slot actif » vu côté backend. Retourne true si la séquence a augmenté
 * (nouvelle sélection sur le hardware) → le soft-sync rafraîchit le slot depuis la RAM ;
 * un `request_preset_content` immédiat n’est déclenché que si `models_hw_force_preset_dump_on_slot_notify=1`.
 * L’événement `models:hardware-slot-changed` déclenche aussi un soft-sync sans attendre le tick.
 */
function applyHardwareSlotStateFromBackend(hw: HardwareActiveSlotState | null): boolean {
  if (!hw || !Number.isFinite(hw.sequence)) return false;
  if (lastSeenHardwareSlotSequence < 0) {
    lastSeenHardwareSlotSequence = hw.sequence;
    const nextIdx =
      Number.isInteger(hw.slotIndex) && (hw.slotIndex as number) >= 0
        ? (hw.slotIndex as number)
        : null;
    const nextBus = Number.isInteger(hw.slotBus) ? (hw.slotBus as number) : null;
    pendingHardwareSelectedKemplineSlotIndex = nextIdx;
    pendingHardwareSelectedSlotBus = nextBus;
    return false;
  }
  let forceUsb = false;
  if (hw.sequence < lastSeenHardwareSlotSequence) {
    hwSlotDebugLog(
      `reset sequence local=${lastSeenHardwareSlotSequence} backend=${hw.sequence}`,
    );
    lastSeenHardwareSlotSequence = hw.sequence;
  } else if (hw.sequence > lastSeenHardwareSlotSequence) {
    forceUsb = true;
    emitModelsSyncTraceThrottled(
      "hw_slot_seq",
      `hw_slot_notify seq ${lastSeenHardwareSlotSequence}->${hw.sequence} slotIdx=${hw.slotIndex} slotBus=${hw.slotBus}`,
      400,
    );
    const nextIdx =
      Number.isInteger(hw.slotIndex) && (hw.slotIndex as number) >= 0
        ? (hw.slotIndex as number)
        : null;
    const nextBus = Number.isInteger(hw.slotBus) ? (hw.slotBus as number) : null;
    hwSlotDebugLog(
      `event sequence ${lastSeenHardwareSlotSequence} -> ${hw.sequence}, slot=${nextIdx ?? "null"}, bus=${nextBus ?? "null"}`,
    );
    lastSeenHardwareSlotSequence = hw.sequence;
    pendingHardwareSelectedKemplineSlotIndex = nextIdx;
    pendingHardwareSelectedSlotBus = nextBus;
  }
  return forceUsb;
}

function catalogChainHexLogEnabled(): boolean {
  return localStorage.getItem(DEBUG_CATALOG_CHAINHEX_FLAG) === "1";
}

function modelsSyncTraceEnabled(): boolean {
  return localStorage.getItem(MODELS_SYNC_TRACE_FLAG) === "1";
}

/**
 * Trace sync UI : `console.info` (DevTools fenêtre Models) + `eprintln!` côté Rust (terminal `cargo tauri dev`).
 * Active avec `localStorage.setItem("models_debug_sync_trace", "1")` dans la **fenêtre Models**.
 * Pour les messages très répétitifs (soft-sync ~200 ms), utiliser **`emitModelsSyncTraceThrottled`**.
 */
function emitModelsSyncTrace(line: string): void {
  if (!modelsSyncTraceEnabled()) return;
  const ts = new Date().toISOString();
  const msg = `[ModelsSync][${ts}] ${line}`;
  console.info(msg);
  void invoke("log_frontend_message", { message: msg }).catch(() => {
    console.warn("[ModelsSync] log_frontend_message invoke failed — trace déjà ci-dessus (console)");
  });
}

/** Dernière émission par clé — évite spam console + `invoke(log_frontend_message)` à chaque tick. */
const modelsSyncTraceLastByKey = new Map<string, number>();
const MODELS_SYNC_TRACE_THROTTLE_DEFAULT_MS = 12_000;

function emitModelsSyncTraceThrottled(
  key: string,
  line: string,
  minIntervalMs: number = MODELS_SYNC_TRACE_THROTTLE_DEFAULT_MS,
): void {
  if (!modelsSyncTraceEnabled()) return;
  const now = Date.now();
  const prev = modelsSyncTraceLastByKey.get(key) ?? 0;
  if (now - prev < minIntervalMs) return;
  modelsSyncTraceLastByKey.set(key, now);
  emitModelsSyncTrace(line);
}

/** Même texte que l’ancien `console.log` : le backend le réaffiche via `eprintln!` (terminal `tauri dev`). */
async function emitCatalogChainHexToTerminal(line: string): Promise<void> {
  try {
    await invoke("log_frontend_message", { message: line });
  } catch {
    console.log(line);
  }
}

/**
 * Quand le matériel change de modèle dans un slot, le prochain `request_preset_content` + parse
 * met à jour `moduleHex`. On journalise `chainHex` (hardware) et `Name` (entrée HX_ModelCatalog.json via byHex).
 */
async function logCatalogChainHexDiffIfNeeded(slots: SlotDebug[], presetIndex: number): Promise<void> {
  if (!catalogChainHexLogEnabled()) return;
  const next = slots.map((s) => (s.moduleHex ?? "").trim().toLowerCase());
  if (presetIndex !== lastCatalogChainHexSnapshotPresetIndex) {
    lastCatalogChainHexSnapshotPresetIndex = presetIndex;
    lastCatalogChainHexBySlot = next;
    return;
  }
  const prev = lastCatalogChainHexBySlot;
  lastCatalogChainHexBySlot = next;
  if (prev === null) return;
  const max = Math.max(prev.length, next.length);
  for (let i = 0; i < max; i += 1) {
    const chainHex = next[i] ?? "";
    const was = prev[i] ?? "";
    if (chainHex === was) continue;
    let name = "";
    if (chainHex) {
      const id = await getCatalogModelIdForHex(chainHex);
      if (id) {
        const nm = await getCatalogModelNameForId(id);
        name = (nm ?? "").trim();
      }
    }
    const line = `[CatalogChainHex] preset=${presetIndex} kemplineSlot=${i} chainHex = "${chainHex}" - Name = "${name}"`;
    await emitCatalogChainHexToTerminal(line);
  }
}

function getHardwareSyncIntervalMs(): number {
  const raw = (localStorage.getItem(HW_SYNC_INTERVAL_MS_KEY) ?? "").trim();
  if (raw === "0") return 0;
  if (!raw) return 0;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed < 0) return 0;
  if (parsed === 0) return 0;
  return Math.max(HW_SYNC_MIN_MS, Math.min(HW_SYNC_MAX_MS, parsed));
}

function getHardwareUsbPresetPollMs(): number {
  const raw = (localStorage.getItem(HW_USB_PRESET_POLL_MS_KEY) ?? "").trim();
  if (!raw || raw === "0") return 0;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return 0;
  return Math.max(HW_USB_PRESET_POLL_MIN_MS, Math.min(HW_USB_PRESET_POLL_MAX_MS, parsed));
}

function delayMs(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

/** Évite deux `switch_active_hardware_slot` concurrents (clics rapides → embouteillage USB / preset). */
let hardwareSlotSwitchTail: Promise<void> = Promise.resolve();

function enqueueHardwareSlotSwitch(slotIndex: number): Promise<void> {
  const p = hardwareSlotSwitchTail.then(async () => {
    try {
      await invoke("switch_active_hardware_slot", { slotIndex });
    } catch (e) {
      console.warn("[HwSlotSync] switch_active_hardware_slot error", e);
    }
  });
  hardwareSlotSwitchTail = p.catch(() => {});
  return p;
}

/**
 * Pendant `runHardwareSyncSoftRefresh`, le thread JS attend souvent sur `request_preset_content`
 * puis une longue boucle : un clic slot peut alors envoyer un `switch_active_hardware_slot` **entre**
 * ces `await` alors que le backend est encore en `preset_content_only` — courses avec le dump.
 */
async function waitUntilHardwareSyncIdle(maxWaitMs: number): Promise<void> {
  const deadline = Date.now() + maxWaitMs;
  let logged = false;
  while (hardwareSyncBusy) {
    if (!logged) {
      logged = true;
      emitModelsSyncTrace(`waitHwSyncBusy (avant switch USB) maxWaitMs=${maxWaitMs}`);
    }
    if (Date.now() >= deadline) {
      emitModelsSyncTrace(`waitHwSyncBusy TIMEOUT encore busy=${hardwareSyncBusy}`);
      return;
    }
    await delayMs(40);
  }
}

function normalizeSlotsPayloadFromInvoke(
  slots: [string, string][] | [string, string, string, string][] | [string, string, string, string, string][],
): SlotDebug[] {
  if (debugRoutingMode) {
    return (slots as unknown as [string, string, string, string, string][]).map(
      ([category, name, gridX, gridY, moduleHex]) => ({
        category,
        name,
        gridX,
        gridY,
        moduleHex: moduleHex?.trim() || undefined,
      }),
    );
  }
  return (slots as unknown as [string, string, string][]).map(([category, name, moduleHex]) => ({
    category,
    name,
    moduleHex: moduleHex?.trim() || undefined,
  }));
}

function chainLayoutSignature(slots: SlotDebug[]): string {
  return slots
    .map((s, i) => {
      // Signature volontairement "structurelle" pour éviter les faux positifs de rerender:
      // on ignore le nom affiché (peut varier/bruiter selon parse) et on garde les clés slot.
      const cat = s.category.trim().toLowerCase();
      const hx = (s.moduleHex ?? "").trim().toLowerCase();
      const gx = (s.gridX ?? "").trim();
      const gy = (s.gridY ?? "").trim();
      const st = (s.slotTypeHex ?? "").trim().toLowerCase();
      return `${i}|${cat}|${hx}|${gx}|${gy}|${st}`;
    })
    .join("\x1e");
}

function rememberHwSyncChainLayout(slots: SlotDebug[]): void {
  lastHwSyncChainSignature = chainLayoutSignature(slots);
  // Snapshot pour le soft-sync sans re-parse : grille 16 ou flow (toute liste non vide).
  lastHwSyncNormalizedSlots = slots.length > 0 ? slots.map((s) => ({ ...s })) : null;
  pendingHwLayoutSignature = null;
}

/** Soft-sync déclenché par événement USB (`models:hardware-slot-changed`), pas par un tick fixe. */
function scheduleHardwareSyncFromEvent(): void {
  if (hardwareSyncPausedForPresetLoad) return;
  if (hwUi.gestureInProgress) return;
  void runHardwareSyncSoftRefresh();
}

/** Re-dump preset USB périodique uniquement si `models_hw_usb_preset_poll_ms` > 0 (voir doc). */
function startOptionalUsbPresetPollTimer(): void {
  const pollMs = getHardwareUsbPresetPollMs();
  if (pollMs <= 0) return;
  window.setInterval(() => {
    if (hardwareSyncPausedForPresetLoad) return;
    if (hwUi.gestureInProgress) return;
    void runHardwareSyncSoftRefresh();
  }, pollMs);
}

function requestPresetCooldownRemainingMs(now = Date.now()): number {
  const elapsed = now - lastRequestPresetInvokeAt;
  return Math.max(0, REQUEST_PRESET_MIN_GAP_MS - elapsed);
}

function stopLoadingHeartbeat(): void {
  if (loadingHeartbeatTimer !== null) {
    window.clearInterval(loadingHeartbeatTimer);
    loadingHeartbeatTimer = null;
  }
}

function startLoadingHeartbeat(baseText: string): void {
  loadingHeartbeatBase = baseText;
  loadingHeartbeatPhase = 0;
  stopLoadingHeartbeat();
  setStatus(`${loadingHeartbeatBase}.`);
  loadingHeartbeatTimer = window.setInterval(() => {
    const dots = ".".repeat((loadingHeartbeatPhase % 3) + 1);
    setStatus(`${loadingHeartbeatBase}${dots}`);
    loadingHeartbeatPhase += 1;
  }, 350);
}

function armQueuedPresetLoadAfterCooldown(): void {
  if (queuedPresetLoadTimer !== null) return;
  // Après report « scroll HW », attendre la fin de la fenêtre busy Rust (~700 ms).
  const waitMs = Math.max(requestPresetCooldownRemainingMs() + 5, 750);
  queuedPresetLoadTimer = window.setTimeout(() => {
    queuedPresetLoadTimer = null;
    if (loading) return;
    if (pendingPresetIndex < 0) return;
    const next = pendingPresetIndex;
    pendingPresetIndex = -1;
    void requestLoadForPreset(next);
  }, waitMs);
}

function armRecoveryPresetLoad(reason: string): void {
  if (recoveryPresetLoadTimer !== null) return;
  recoveryAttemptCount += 1;
  emitModelsSyncTrace(
    `armRecovery reason=${JSON.stringify(reason)} attempt=${recoveryAttemptCount}/${REQUEST_PRESET_HARD_RECOVERY_AFTER} pendingPreset=${pendingPresetIndex}`,
  );
  startLoadingHeartbeat(`Sablier: recuperation USB en cours (${reason})`);
  recoveryPresetLoadTimer = window.setTimeout(() => {
    recoveryPresetLoadTimer = null;
    if (loading) return;
    if (pendingPresetIndex < 0) return;
    const next = pendingPresetIndex;
    pendingPresetIndex = -1;
    if (recoveryAttemptCount >= REQUEST_PRESET_HARD_RECOVERY_AFTER) {
      setStatus("Sablier: reset communication USB en cours...");
      emitModelsSyncTrace(
        `force_recover_preset_reader invoke attempts=${recoveryAttemptCount} nextPreset=${next}`,
      );
      void invoke("force_recover_preset_reader")
        .catch(() => {
          // Best effort: même en cas d'échec on tente une relance de lecture.
        })
        .finally(() => {
          recoveryAttemptCount = 0;
          void requestLoadForPreset(next);
        });
      return;
    }
    void requestLoadForPreset(next);
  }, REQUEST_PRESET_RECOVERY_DELAY_MS);
}

function applyProbeSlotMergeToNormalized(normalized: SlotDebug[]): SlotDebug[] {
  const m = mergeProbeSlotModelUntil;
  if (!m || Date.now() > m.deadline) {
    mergeProbeSlotModelUntil = null;
    lastProbePickerAssignContext = null;
    return normalized;
  }
  if (normalized.length !== 16) return normalized;
  if (!lastHwSyncNormalizedSlots || lastHwSyncNormalizedSlots.length !== 16) return normalized;
  const indices = [m.ki, ...(m.extraKis ?? [])];
  if (!m.mergeTraceEmitted) {
    mergeProbeSlotModelUntil = { ...m, mergeTraceEmitted: true };
    emitModelsSyncTrace(
      `softSync merge probe slots=${indices.join(",")} (stale preset_data parse vs optimistic row; no post-probe dump)`,
    );
  }
  return normalized.map((s, i) => {
    if (!indices.includes(i)) return { ...s };
    const optimistic = lastHwSyncNormalizedSlots![i];
    return optimistic ? { ...optimistic } : { ...s };
  });
}

/**
 * Sync matériel : le **parse** `get_active_preset_slots` (RAM Rust `preset_data`) n’est utilisé
 * pour la grille **que** si ce cycle a fait un `request_preset_content` (RAM fraîchement relue).
 * Sinon on s’appuie sur le snapshot `lastHwSyncNormalizedSlots` (dernier chargement / dernière
 * relecture + MAJ optimistes probe/remove) pour le panneau params et la sélection HW — pas de
 * `renderSlots` depuis un dump périmé.
 */
async function runHardwareSyncSoftRefresh(): Promise<void> {
  const syncMs = getHardwareSyncIntervalMs();
  if (currentPresetIndex < 0) return;
  if (hwUi.gestureInProgress) return;
  if (loading || hardwareSyncBusy) return;
  if (slotModelUsbProbeInFlight !== null) return;
  if (loadedPresetIndex !== currentPresetIndex) return;

  const now = Date.now();
  const wroteThisCycle = await flushPendingLiveWrites();
  if (now < liveWriteUiInteractionUntil) return;
  // Pendant une écriture live (ou juste après), éviter de forcer RequestPreset,
  // sinon la machine d'état repasse en lecture et l'écriture peut être ignorée.
  if (!wroteThisCycle && now - lastLiveWriteAt < LIVE_WRITE_SYNC_PAUSE_MS) return;

  let hwSlotState: HardwareActiveSlotState | null = null;
  try {
    hwSlotState = await invoke<HardwareActiveSlotState>("get_active_hardware_slot_state");
  } catch {
    hwSlotState = null;
  }
  const hardwareSlotSequenceAdvanced = applyHardwareSlotStateFromBackend(hwSlotState);
  if (hardwareSlotSequenceAdvanced) {
    if (forcePresetDumpOnHardwareSlotNotify()) {
      pendingForceUsbPresetContent = true;
      emitModelsSyncTrace(
        "hw_slot_notify + models_hw_force_preset_dump_on_slot_notify=1 -> pending USB preset dump",
      );
    } else {
      hwSlotDebugLog(
        "hw slot seq advanced: refresh depuis RAM (pas de request_preset_content sur seule notif slot)",
      );
      emitModelsSyncTraceThrottled(
        "hw_slot_notify_tick",
        "hw_slot_notify -> slot-only snapshot (pas de request_preset_content sur seule notif slot) ; voir description.md 12 mai (suite)",
        15_000,
      );
    }
  }

  if (!hardwareSlotSequenceAdvanced && now - lastHardwareSyncAt < syncMs) return;

  const usbPresetPollMs = getHardwareUsbPresetPollMs();
  const forceUsbPending = pendingForceUsbPresetContent;
  const pollPresetDue =
    usbPresetPollMs > 0 &&
    now - lastSoftUsbPresetReadAt >= usbPresetPollMs &&
    now >= suppressUsbPresetPollUntilMs;
  const wantUsbPresetDump = pendingForceUsbPresetContent || pollPresetDue;
  if (wantUsbPresetDump && requestPresetCooldownRemainingMs(now) > 0 && !pendingForceUsbPresetContent) {
    emitModelsSyncTraceThrottled(
      "soft_sync_usb_cooldown",
      `softSync skip cooldown wantUsb pollMs=${usbPresetPollMs} lastUsbReadAge=${now - lastSoftUsbPresetReadAt}ms`,
      10_000,
    );
    return;
  }
  lastHardwareSyncAt = now;

  const presetIdx = currentPresetIndex;
  hardwareSyncBusy = true;
  try {
    let normalized: SlotDebug[] | null = null;
    let didUsbPresetDumpThisCycle = false;

    if (
      hardwareSlotSequenceAdvanced &&
      slotFocusUsbSyncEnabled() &&
      !forcePresetDumpOnHardwareSlotNotify() &&
      hwSlotState &&
      Number.isInteger(hwSlotState.slotIndex) &&
      (hwSlotState.slotIndex as number) >= 0 &&
      (hwSlotState.slotIndex as number) < 16
    ) {
      const focusIdx = hwSlotState.slotIndex as number;
      // Ne pas await : la synchro USB (~60 ms côté Rust) ne doit pas retarder grille / panneau (RAM).
      void invoke<SlotFocusSyncResponse>("sync_hardware_slot_focus_usb", { slotIndex: focusIdx })
        .then(async (snap) => {
          const n = snap?.inFrameCount ?? 0;
          hwSlotDebugLog(`sync_hardware_slot_focus_usb slot=${focusIdx} inFrames=${n}`);
          await applySlotContentWatchFromSync(snap, focusIdx);
        })
        .catch((e) => {
          console.warn("[HwSlotSync] sync_hardware_slot_focus_usb", e);
          emitModelsSyncTraceThrottled(
            "slot_focus_usb_err",
            `slot_focus_usb error slot=${focusIdx} ${String(e)}`,
            5_000,
          );
        });
    }

    if (wantUsbPresetDump) {
      const initSettling = await invoke<boolean>("is_helix_usb_init_settling").catch(() => false);
      if (initSettling) {
        emitModelsSyncTraceThrottled(
          "softSync_skip_init_settle",
          "request_preset_content reporté : init USB ~700 ms (ACK seulement)",
          2_000,
        );
        return;
      }
      pendingForceUsbPresetContent = false;
      didUsbPresetDumpThisCycle = true;
      lastRequestPresetInvokeAt = Date.now();
      emitModelsSyncTrace(
        `request_preset_content (softSync) reason=${forceUsbPending ? "hw_notify_force" : "poll_interval"} pollMs=${usbPresetPollMs} preset=${presetIdx}`,
      );
      await invoke("request_preset_content");
      // Le parse preset peut livrer `[]` une ou deux lectures trop tôt → ne jamais traiter comme « ok ».
      await delayMs(120);

      for (let tries = 0; tries < 45; tries += 1) {
        if (currentPresetIndex !== presetIdx) return;
        try {
          const slots = debugRoutingMode
            ? await invoke<[string, string, string, string][] | null>("get_active_preset_slots_debug")
            : await invoke<[string, string][] | null>("get_active_preset_slots");
          if (slots !== null) {
            const candidate = normalizeSlotsPayloadFromInvoke(slots as never);
            if (candidate.length > 0) {
              normalized = candidate;
              break;
            }
          }
        } catch {
          // transient
        }
        await delayMs(200);
      }
      if (normalized && normalized.length > 0) {
        lastSoftUsbPresetReadAt = Date.now();
        emitModelsSyncTrace(
          `softSync usbDump ok preset=${presetIdx} slots=${normalized.length} (wait loop)`,
        );
      } else {
        normalized = null;
        emitModelsSyncTrace(`softSync usbDump NO_SLOTS after wait preset=${presetIdx}`);
      }
    } else {
      // Pas de relecture preset ce cycle : ne pas re-parser `preset_data` (évite grille fantôme).
      if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length > 0) {
        normalized = lastHwSyncNormalizedSlots.map((s) => ({ ...s }));
      } else {
        normalized = null;
        emitModelsSyncTraceThrottled(
          "soft_sync_no_snapshot",
          "softSync sans request_preset_content et sans snapshot grille — skip",
          20_000,
        );
      }
    }

    if (!normalized || normalized.length === 0 || currentPresetIndex !== presetIdx) {
      if (
        lastHwSyncNormalizedSlots &&
        lastHwSyncNormalizedSlots.length > 0 &&
        currentPresetIndex === presetIdx
      ) {
        emitModelsSyncTraceThrottled(
          "soft_sync_empty_parse",
          `softSync abort emptyParse keepExistingGrid preset=${presetIdx} snapSlots=${lastHwSyncNormalizedSlots.length}`,
          8_000,
        );
        scheduleSoftRefreshParamsPaneFromSlots(lastHwSyncNormalizedSlots);
      } else {
        emitModelsSyncTraceThrottled(
          "soft_sync_abort_no_norm",
          `softSync abort noNormalized=${!normalized || normalized.length === 0} presetChanged=${currentPresetIndex !== presetIdx} cur=${currentPresetIndex} snap=${presetIdx}`,
          8_000,
        );
      }
      return;
    }

    if (normalized.length === 16) {
      normalized = applyProbeSlotMergeToNormalized(normalized);
    }

    const deviceActive = await invoke<number>("get_active_preset");
    if (deviceActive !== presetIdx) {
      devicePresetMismatchStreak += 1;
      emitModelsSyncTraceThrottled(
        "device_preset_mismatch_streak",
        `deviceActive mismatch backend=${deviceActive} modelsUi=${presetIdx} streak=${devicePresetMismatchStreak}/2`,
        5_000,
      );
      if (devicePresetMismatchStreak < 2) {
        return;
      }
      devicePresetMismatchStreak = 0;
      emitModelsSyncTrace(`deviceActive mismatch CONFIRMED -> full reload preset ${deviceActive}`);
      const names = await invoke<string[]>("get_preset_names");
      if (deviceActive >= 0 && deviceActive < names.length) {
        currentPresetIndex = deviceActive;
        loadedPresetIndex = -1;
        clearSelectedParamsContext();
        renderEmpty("Chargement des modeles...");
        scheduleLoadForPreset(deviceActive, true);
      }
      return;
    }
    devicePresetMismatchStreak = 0;

    const names = await invoke<string[]>("get_preset_names");
    const nameSig = `${deviceActive}\n${names.join("\n")}`;
    if (nameSig !== lastPresetNamesSig) {
      lastPresetNamesSig = nameSig;
      if (deviceActive >= 0 && deviceActive < names.length) {
        const displayName = isEmpty(names[deviceActive]) ? "empty" : names[deviceActive];
        presetLabelEl.textContent = `${padNum(deviceActive)} ${displayName}`;
      }
    }

    await logCatalogChainHexDiffIfNeeded(normalized, presetIdx);

    const sig = chainLayoutSignature(normalized);
    // Layout inchangé => on évite `renderSlots` (flash) et on met à jour uniquement le panneau sélectionné.
    if (lastHwSyncChainSignature !== null && sig === lastHwSyncChainSignature) {
      pendingHwLayoutSignature = null;
      consumePendingHardwareSlotSelection();
      scheduleSoftRefreshParamsPaneFromSlots(normalized);
      return;
    }
    // Anti-flash / debounce layout : sans dump USB ce cycle, la signature ne vient pas d’un parse
    // frais — le chemin `!didUsbPresetDumpThisCycle` ci-dessous évite tout `renderSlots`.
    const usbDumpIsPollOnly = didUsbPresetDumpThisCycle && !forceUsbPending;
    if (lastHwSyncChainSignature !== null && sig !== lastHwSyncChainSignature) {
      const allowLayoutDebounce = !didUsbPresetDumpThisCycle || usbDumpIsPollOnly;
      if (allowLayoutDebounce && pendingHwLayoutSignature !== sig) {
        pendingHwLayoutSignature = sig;
        scheduleSoftRefreshParamsPaneFromSlots(normalized);
        emitModelsSyncTraceThrottled(
          "soft_sync_layout_debounce",
          `softSync layout debounce pass1 -> paramsPane only (sig changed usbPollOnly=${usbDumpIsPollOnly})`,
          8_000,
        );
        return;
      }
      pendingHwLayoutSignature = null;
    }

    if (!didUsbPresetDumpThisCycle) {
      pendingHwLayoutSignature = null;
      consumePendingHardwareSlotSelection();
      scheduleSoftRefreshParamsPaneFromSlots(normalized);
      await maybeWatchActiveSlotContent(hwSlotState);
      emitModelsSyncTraceThrottled(
        "soft_sync_no_dump_grid",
        "softSync sans dump USB ce cycle : pas de renderSlots (grille = dernier chargement / MAJ optimistes)",
        20_000,
      );
      return;
    }

    rememberHwSyncChainLayout(normalized);
    emitModelsSyncTrace(
      `softSync -> renderSlots FULL preset=${presetIdx} didUsb=${didUsbPresetDumpThisCycle} usbPollOnly=${usbDumpIsPollOnly} sigLen=${sig.length}`,
    );
    let routingFlow: RoutingMarker[] = [];
    let stompLayout: ActivePresetStompLayout | null = null;
    if (isKemplineGrid16(normalized)) {
      try {
        const r = await invoke<[string, string, string][] | null>("get_active_preset_routing_markers");
        routingFlow =
          r?.map(([category, name, moduleHex]) => ({
            category,
            name,
            moduleHex: moduleHex?.trim() || undefined,
          })) ?? [];
      } catch {
        console.warn("[PresetDebug][models] get_active_preset_routing_markers error (hw sync)");
      }
      try {
        stompLayout = await invoke<ActivePresetStompLayout | null>("get_active_preset_stomp_layout");
      } catch {
        console.warn("[PresetDebug][models] get_active_preset_stomp_layout error (hw sync)");
      }
    }
    if (currentPresetIndex !== presetIdx) return;
    await renderSlots(normalized, routingFlow, stompLayout);
    const realBlocks = countRealBlocks(normalized);
    const singleDsp = isSingleDspDevice(connectedDeviceName);
    const dspSuffix = singleDsp ? ` (${realBlocks}/8 max)` : "";
    const overLimit =
      singleDsp && realBlocks > 8 ? " - warning: parsed blocks exceed Stomp DSP budget" : "";
    setStatus(
      debugRoutingMode
        ? `${realBlocks} blocks detected${dspSuffix} (debug routing ON)${overLimit}`
        : `${realBlocks} blocks detected${dspSuffix}${overLimit}`,
    );
  } catch (e) {
    console.warn("[PresetDebug][models] hardware sync soft refresh error", e);
  } finally {
    hardwareSyncBusy = false;
  }
}

async function softRefreshParamsPaneFromSlots(slots: SlotDebug[]): Promise<void> {
  if (!hasSelectedParamsContextForCurrentPreset()) return;
  const idx = selectedParamsKemplineSlotIndex;
  if (idx === null || idx < 0 || idx >= slots.length) return;
  const slot = slots[idx];
  if (!slot) return;
  if (isEmptyGridCell(slot)) {
    const nextSig = `${currentPresetIndex}|${idx}|empty`;
    if (selectedParamsValuesSig === nextSig) return;
    selectedParamsInPlaceUpdater = null;
    selectedParamsInPlaceSlotKey = null;
    selectedParamsHwWireContext = null;
    clearModelsParamsPaneContent();
    selectedParamsValuesSig = nextSig;
    return;
  }
  const idTrim = await resolveSlotCatalogModelId(slot);
  const meta = idTrim ? await getPresetMetaForId(idTrim) : null;
  const chainValues = idTrim
    ? await resolveChainValuesForKemplineSlot(
        idx,
        slot,
        idTrim,
        meta?.categoryName ?? null,
        pickSignal(meta, slot.moduleHex),
      )
    : null;
  const nextSig = `${currentPresetIndex}|${idx}|${chainValuesSignature(chainValues)}`;
  if (selectedParamsValuesSig === nextSig) return;
  const slotKey = makeSlotSelectionKey(slot, idx);
  const wantsDualTabs =
    slotWantsAmpCabDualTabs(slot, probePickerAssignVariantHint(idx)) ||
    slotWantsCabDualTabs(slot, probePickerAssignVariantHint(idx), meta);
  const inner = getModelsParamsInner();
  const hasDualTabs = inner?.querySelector(".models-params-dual-tabs") !== null;
  if (
    selectedParamsInPlaceUpdater &&
    selectedParamsInPlaceSlotKey &&
    selectedParamsInPlaceSlotKey === slotKey &&
    chainValues !== null &&
    chainValues.length > 0 &&
    wantsDualTabs === hasDualTabs
  ) {
    selectedParamsInPlaceUpdater(chainValues);
    selectedParamsValuesSig = nextSig;
    return;
  }
  await loadAndShowModelsParamsForSlot(slot, idx);
  selectedParamsValuesSig = nextSig;
}

function liveWriteProbeEnabled(): boolean {
  return localStorage.getItem(LIVE_WRITE_PROBE_FLAG) === "1";
}

function liveWriteEnabled(): boolean {
  return localStorage.getItem(LIVE_WRITE_ENABLED_FLAG) === "1";
}

/** File d’attente + flush : sonde seule (`probe`) ou écriture USB/MIDI réelle (`enabled`). */
function liveWriteQueueEnabled(): boolean {
  return liveWriteProbeEnabled() || liveWriteEnabled();
}

let liveWriteFlushTimer: number | null = null;

function scheduleLiveWriteFlushDebounced(): void {
  if (liveWriteFlushTimer !== null) {
    window.clearTimeout(liveWriteFlushTimer);
  }
  liveWriteFlushTimer = window.setTimeout(() => {
    liveWriteFlushTimer = null;
    void flushPendingLiveWrites();
  }, 100);
}

function markLiveWriteUiInteraction(): void {
  const now = Date.now();
  // Pause immédiate pour empêcher un poll hardware de partir entre deux events slider.
  liveWriteUiInteractionUntil = now + 900;
  lastLiveWriteAt = now;
}

function liveWriteTransport(): "usb_raw" | "midi_cc" {
  return localStorage.getItem(LIVE_WRITE_TRANSPORT_KEY) === "midi_cc" ? "midi_cc" : "usb_raw";
}

function liveWriteMidiCcNumber(): number {
  const raw = (localStorage.getItem(LIVE_WRITE_MIDI_CC_KEY) ?? "").trim();
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) return 0;
  return Math.max(0, Math.min(127, parsed));
}

function liveWriteMidiChannel(): number {
  const raw = (localStorage.getItem(LIVE_WRITE_MIDI_CHANNEL_KEY) ?? "").trim();
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) return 0;
  return Math.max(0, Math.min(15, parsed));
}

function scheduleLiveParamWriteProbe(
  slotIndex: number | undefined,
  paramIndex: number,
  p: ModelParamDefJson,
  rawValue: number,
  dualPart: "amp" | "cab" | "cab1" | "cab2" | null = null,
  ampCabAssignVariant: string | null = null,
  cabDualAssignVariant: string | null = null,
  ampCabAmpParamCount: number | null = null,
): void {
  if (!liveWriteQueueEnabled()) return;
  if (slotIndex === undefined || !Number.isInteger(slotIndex)) return;
  if (!Number.isFinite(rawValue)) return;
  const symbolicId = (p.symbolicID ?? "").trim();
  if (!symbolicId) return;
  const key = `${slotIndex}:${symbolicId}:${paramIndex}`;
  const vtRaw = p.valueType;
  const valueType =
    vtRaw !== undefined && vtRaw !== null && Number.isFinite(Number(vtRaw))
      ? Number(vtRaw)
      : null;
  const rawMin = typeof p.min === "number" && Number.isFinite(p.min) ? p.min : null;
  const rawMax = typeof p.max === "number" && Number.isFinite(p.max) ? p.max : null;
  if (currentPresetIndex >= 0) {
    recordLiveChainParamOverrideForKemplineSlot(
      currentPresetIndex,
      slotIndex,
      symbolicId,
      rawValue,
    );
  }
  pendingLiveWrites.set(key, {
    slotIndex,
    paramIndex,
    symbolicId,
    displayType: (p.displayType ?? "").trim() || null,
    valueType,
    rawValue,
    rawMin,
    rawMax,
    dualPart,
    ampCabAssignVariant,
    cabDualAssignVariant,
    ampCabAmpParamCount,
  });
  scheduleLiveWriteFlushDebounced();
}

/** Témoin : sélecteur local par type d'onde (défaut ON). `="0"` -> ancien index global. */
function wireLocalParamSelectorEnabled(): boolean {
  return localStorage.getItem("models_wire_local_param_selector") !== "0";
}

/**
 * Param écrit en trame `23`/`c2` (discret/bool) vs `27`/`c3` (float) — miroir de `wire_23`
 * côté Rust (build_live_write_frames_from_state). valueType 0 = entier/discret, 2 = bool,
 * 1 = float. off_on / polarity = bool même sans valueType 2.
 */
function isWire23Param(p: ModelParamDefJson): boolean {
  const vt = p.valueType;
  if (vt === 2 || vt === 0) return true;
  if (isOffOnDisplayType(p.displayType)) return true;
  if (isPolarityDisplayType(p.displayType)) return true;
  return false;
}

function liveWriteParamIndexForRow(
  paramsForDisplay: ModelParamDefJson[],
  rowIndex: number,
  catalogSignal: string | null | undefined,
  paramIndexBase = 0,
  wireLocal = false,
): number {
  const target = paramsForDisplay[rowIndex];
  if (!target) return paramIndexBase + rowIndex;
  const writeOrder = paramsVisibleForSignal(paramsForDisplay, catalogSignal);
  const idxByRef = writeOrder.indexOf(target);
  if (idxByRef < 0) return paramIndexBase + rowIndex;

  // Sélecteur LOCAL par type d'onde : UNIQUEMENT pour les cab single legacy.
  // Les cab modern (single et dual) utilisent l'index global (mic compté) — vérifié sur le Stomp.
  if (wireLocal && paramIndexBase === 0 && wireLocalParamSelectorEnabled()) {
    const targetWire23 = isWire23Param(target);
    let local = 0;
    for (let i = 0; i < idxByRef; i += 1) {
      if (isWire23Param(writeOrder[i]!) === targetWire23) local += 1;
    }
    return local;
  }
  return paramIndexBase + idxByRef;
}

/**
 * Décalage d'index wire pour le 2ᵉ panneau Amp+Cab (cab après l'ampli).
 * Cab dual : toujours 0 — `dualPart` (`cab1`/`cab2`) cible le sous-modèle, l'index param est local.
 */
function dualPaneLiveWriteParamIndexBase(
  dualSlotKind: "amp_cab" | "cab_dual",
  paneIndex: number,
  tabPanes: DualTabPaneConfig[],
): number {
  if (paneIndex <= 0) return 0;
  if (dualSlotKind === "cab_dual") return 0;
  const primary = tabPanes[0];
  if (!primary) return 0;
  const chainLen = primary.chainValues?.length ?? 0;
  if (chainLen > 0) return chainLen;
  return paramsVisibleForSignal(primary.params, primary.catalogRoutingSignal).length;
}

function dualPaneHwWireContext(
  dualSlotKind: "amp_cab" | "cab_dual",
  paneIndex: number,
  tabPanes: DualTabPaneConfig[],
): {
  paramsForDisplay: ModelParamDefJson[];
  catalogSignal: string | null | undefined;
  wireParamIndexBase: number;
} {
  const pane = tabPanes[paneIndex] ?? tabPanes[0]!;
  return {
    paramsForDisplay: pane.params,
    catalogSignal: pane.catalogRoutingSignal,
    wireParamIndexBase: dualPaneLiveWriteParamIndexBase(dualSlotKind, paneIndex, tabPanes),
  };
}

/** Inverse de `liveWriteParamIndexForRow` : index wire `PP` → `symbolicID` catalogue. */
function symbolicIdForWireParamIndex(
  paramsForDisplay: ModelParamDefJson[],
  wireParamIndex: number,
  catalogSignal: string | null | undefined,
  wireParamIndexBase = 0,
): string | null {
  const writeOrder = paramsVisibleForSignal(paramsForDisplay, catalogSignal);
  const local = wireParamIndex - wireParamIndexBase;
  if (local < 0 || local >= writeOrder.length) return null;
  const p = writeOrder[local];
  const sid = (p?.symbolicID ?? "").trim();
  return sid || null;
}

function chainValueFromHwSlotParam(p: SlotParamChangedPayload): ChainParamValueJson | null {
  if (p.valueType === "bool") {
    if (typeof p.value === "boolean") return p.value;
    if (p.value === 0 || p.value === 1) return p.value === 1;
    return null;
  }
  if (p.valueType === "discrete") {
    const n = typeof p.value === "number" ? p.value : Number(p.value);
    if (!Number.isFinite(n)) return null;
    return Math.round(n);
  }
  if (p.valueType === "float") {
    const n = typeof p.value === "number" ? p.value : Number(p.value);
    if (!Number.isFinite(n)) return null;
    return n;
  }
  return null;
}

function applyHardwareSlotParamChanged(p: SlotParamChangedPayload): void {
  if (currentPresetIndex < 0) return;
  if (Date.now() < liveWriteUiInteractionUntil) return;
  const cv = chainValueFromHwSlotParam(p);
  if (cv === null) return;

  const ctx = selectedParamsHwWireContext;
  const sid =
    ctx !== null
      ? symbolicIdForWireParamIndex(
          ctx.paramsForDisplay,
          p.paramIndex,
          ctx.catalogSignal,
          ctx.wireParamIndexBase ?? 0,
        )
      : null;
  if (!sid) return;

  recordLiveChainParamOverrideForKemplineSlot(currentPresetIndex, p.slotIndex, sid, cv);

  if (
    selectedParamsKemplineSlotIndex !== p.slotIndex ||
    selectedParamsPresetIndex !== currentPresetIndex ||
    !selectedParamsInPlaceUpdater
  ) {
    return;
  }

  const slotIndex = p.slotIndex;
  const paramIndex = p.paramIndex;
  const sequence = p.sequence;
  hwUi.scheduleAfterHwGesture("params", () => {
    if (
      selectedParamsKemplineSlotIndex !== slotIndex ||
      selectedParamsPresetIndex !== currentPresetIndex
    ) {
      return;
    }
    selectedParamsValuesSig = `${currentPresetIndex}|${slotIndex}|hw:${sequence}:${paramIndex}:${String(cv)}`;
    emitModelsSyncTraceThrottled(
      "evt_slot_param_changed",
      `hw param slot=${slotIndex} pp=${paramIndex} ${sid}=${String(cv)}`,
      400,
    );
    if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
      scheduleSoftRefreshParamsPaneFromSlots(lastHwSyncNormalizedSlots);
    }
  });
}

function discreteSliderTickCount(
  valueType: number | undefined,
  minN: number,
  maxN: number,
): number | null {
  if (valueType !== 0) return null;
  if (!Number.isFinite(minN) || !Number.isFinite(maxN) || maxN <= minN) return null;
  const lo = Math.round(minN);
  const hi = Math.round(maxN);
  if (Math.abs(minN - lo) > 1e-6 || Math.abs(maxN - hi) > 1e-6) return null;
  const n = hi - lo + 1;
  // Au-delà, les repères se chevauchent visuellement et n'apportent plus grand-chose.
  if (n < 2 || n > 16) return null;
  return n;
}

function setSliderFillVisual(
  input: HTMLInputElement,
  value: number,
  minN: number,
  maxN: number,
): void {
  if (!Number.isFinite(value) || !Number.isFinite(minN) || !Number.isFinite(maxN) || maxN <= minN) {
    return;
  }
  const pct = ((value - minN) / (maxN - minN)) * 100;
  const clamped = Math.max(0, Math.min(100, pct));
  input.style.setProperty("--slider-fill-pct", `${clamped}%`);
}

async function flushPendingLiveWrites(): Promise<boolean> {
  if (pendingLiveWrites.size === 0) return false;
  const batch = [...pendingLiveWrites.values()];
  pendingLiveWrites.clear();
  const mode = liveWriteEnabled() ? liveWriteTransport() : "probe";
  lastLiveWriteAt = Date.now();
  for (const w of batch) {
    try {
      if (mode === "probe") {
        await invoke("probe_live_param_write", {
          slotIndex: w.slotIndex,
          paramIndex: w.paramIndex,
          symbolicId: w.symbolicId,
          displayType: w.displayType,
          rawValue: w.rawValue,
        });
      } else if (mode === "midi_cc") {
        await invoke("write_live_param_midi_cc", {
          slotIndex: w.slotIndex,
          paramIndex: w.paramIndex,
          symbolicId: w.symbolicId,
          displayType: w.displayType,
          rawValue: liveWriteUsbNormalized01(w),
          midiChannel: liveWriteMidiChannel(),
          ccNumber: liveWriteMidiCcNumber(),
        });
      } else {
        await invoke("write_live_param", {
          slotIndex: w.slotIndex,
          paramIndex: w.paramIndex,
          symbolicId: w.symbolicId,
          displayType: w.displayType,
          valueType: w.valueType,
          rawValue: liveWriteUsbNormalized01(w),
          chainMin: w.rawMin ?? undefined,
          chainMax: w.rawMax ?? undefined,
          dualPart: w.dualPart ?? undefined,
          ampCabAssignVariant: w.ampCabAssignVariant ?? undefined,
          cabDualAssignVariant: w.cabDualAssignVariant ?? undefined,
          ampCabAmpParamCount: w.ampCabAmpParamCount ?? undefined,
        });
      }
    } catch (e) {
      // Mode expérimental : ne pas casser l'UI ; journaliser refus sécurité (ex. valueType non géré USB).
      console.warn("[LiveWrite]", e);
    }
  }
  return true;
}

const MODELS_PARAMS_IDLE_PLACEHOLDER =
  "Les paramètres du bloc sélectionné s'afficheront ici.";

function getModelsParamsInner(): HTMLElement | null {
  return document.getElementById("models-params-inner");
}

// --- Sélecteur catalogue (gauche) : catégorie → sous-catégorie → liste modèles (aperçu uniquement) ---

let catalogPickerDataCache: CatalogPickerData | null = null;
let slotPickerCategoryEl: HTMLSelectElement | null = null;
let slotPickerSubEl: HTMLSelectElement | null = null;
let slotPickerListEl: HTMLUListElement | null = null;
let slotPickerMountPromise: Promise<void> | null = null;
/**
 * Picker figé : I/O Path 1 (Input/Output) ou jonctions routage (Split/Merge uniquement).
 */
type SlotPickerLock =
  | { kind: "io"; category: "Input" | "Output"; parentModelId: string }
  | { kind: "routing"; category: "Split" | "Merge" }
  /** Onglet Cab d’un slot Amp+Cab : picker figé sur Cab / Single ou Single Legacy. */
  | { kind: "amp_cab_cab"; highlightModelId: string; lockedSub: string };
let slotPickerIoLock: SlotPickerLock | null = null;
/** Contexte picker Amp+Cab (onglets Amp / Cab). */
let ampCabDualPickerSync: {
  ampCatalogModelId: string;
  meta: PresetMetaJson | null;
  moduleHex?: string;
  slotCategory: string;
  linkedCabHex: string | null;
  cabCatalogModelId: string;
} | null = null;
let ampCabDualActiveTab: 0 | 1 = 0;

/** Contexte picker Cab dual (onglets Cab 1 / Cab 2).
 *
 * **Règle picker (ne pas régresser)** — `syncPickerForCabDualTab` :
 * - **Cab 1** (`tabIndex === 0`) : **pas de lock** (`slotPickerIoLock` / `applySlotPickerCabDualCabLock`).
 *   Catégorie et sous-catégorie restent libres (comme onglet Amp sur Amp+Cab). Surbrillance = entrée dual parente.
 * - **Cab 2** (`tabIndex === 1`) : lock `amp_cab_cab` sur Cab / **Single IR** (affichage utilisateur).
 *   **Surbrillance liste = `dualTabPanes[1].catalogModelId`** (id **single**, même source que le titre).
 *   **USB replace cab2** : `resolveCabDualCab2UsbWireFromPicker` → entrée assign **dual** / WithPan (hint `c319`), pas le bulk single.
 */
let cabDualPickerSync: {
  dualCatalogModelId: string;
  /** Modèle réel du panneau Cab 1 (peut être single ou dual). */
  cab1CatalogModelId: string;
  /** Surlignage picker onglet Cab 1 (= entrée dual parente). */
  cab1PickerModelId: string;
  cab2CatalogModelId: string;
  cab1PickerSub: string;
  cab2PickerSub: string;
  meta: PresetMetaJson | null;
  moduleHex?: string;
  slotCategory: string;
} | null = null;
let cabDualActiveTab: 0 | 1 = 0;
/** Cab 2 depuis le dump preset actif (vérité matériel après lecture USB). */
async function resolveCabDualCab2IdFromPreset(
  kemplineSlotIndex: number,
  dualCatalogModelId?: string | null,
): Promise<string | null> {
  const dualId =
    (dualCatalogModelId ?? "").trim() ||
    lastCabDualTabPanesContext?.dualCatalogModelId?.trim() ||
    cabDualPickerSync?.dualCatalogModelId?.trim() ||
    "";
  const hwSlot = lastHwSyncNormalizedSlots?.[kemplineSlotIndex];
  if (hwSlot && dualId) {
    const cab2HexTrame = await resolveCabDualCab2HexFromTrame(hwSlot, kemplineSlotIndex);
    const cab1Hex = cabDualWireParts(hwSlot.moduleHex)?.cab1Hex;
    if (cab2HexTrame) {
      const fromTrame = await getCatalogModelIdForCabDualCab2Hex(
        dualId,
        cab2HexTrame,
        cab1Hex,
      );
      if (fromTrame) return fromTrame;
    }
  }
  try {
    const dualParts = await resolveSlotDualParts(kemplineSlotIndex, {
      slot: hwSlot ?? undefined,
      catalogModelId: dualId,
      kind: "cab_dual",
    });
    const part = dualParts?.kind === "cab_dual" ? dualParts.parts[1] : null;
    if (!part || dualParts?.kind !== "cab_dual") return null;
    const cab1Hex = dualParts.parts[0]?.chainHex?.trim();
    const cab2Hex = (part.chainHex ?? "").trim();
    if (dualId && cab2Hex) {
      const fromDualCtx = await getCatalogModelIdForCabDualCab2Hex(
        dualId,
        cab2Hex,
        cab1Hex,
      );
      if (fromDualCtx) return fromDualCtx;
    }
    const fromId = (part.modelId ?? "").trim();
    if (fromId) {
      const legacy = isCabLegacyFromMeta(await getPresetMetaForId(fromId));
      const wire = await resolveCabDualWireCabTarget(fromId, legacy);
      return wire.cabCatalogModelId;
    }
    if (cab2Hex) {
      return (await getCatalogModelIdForCabSingleHex(cab2Hex))?.trim() ?? null;
    }
  } catch {
    /* repli sync UI */
  }
  return null;
}

async function refreshCabDualContextAfterProbe(
  ki: number,
  cabIdForUsb: string,
  tab: 0 | 1,
): Promise<void> {
  const mount = lastCabDualTabPanesContext;
  if (!mount || (mount.kemplineSlotIndex ?? ki) !== ki) return;
  if (tab === 1 && cabIdForUsb.trim()) {
    const panes = await applyCabDualPane2ModelOverride(
      mount.dualTabPanes,
      cabIdForUsb,
      { forceDefaults: true },
    );
    lastCabDualTabPanesContext = { ...mount, dualTabPanes: panes };
    if (cabDualActiveTab === 1) {
      applyModelsParamsDualHeader(mount.slot, panes[1]!);
    }
  }
}
/** Dernier slot Cab dual affiché — réarme le picker si le sync HW arrive avant le mount. */
let lastCabDualTabPanesContext: {
  dualTabPanes: DualTabPaneConfig[];
  dualCatalogModelId: string;
  meta: PresetMetaJson | null;
  slot: SlotDebug;
  kemplineSlotIndex: number | undefined;
} | null = null;
/** Bus USB du slot structurel sélectionné (Input 0, Output 9, Split 10, Merge 19). */
let selectedSpecialHwSlotBus: number | null = null;

/** Bus Path 1 observés USB (`switch_active_hardware_special_slot` / `82:62:SS:1a`). */
const HW_SLOT_BUS_INPUT = 0;
const HW_SLOT_BUS_OUTPUT = 9;
const HW_SLOT_BUS_SPLIT = 0x0a;
const HW_SLOT_BUS_MERGE = 0x13;

function hwSlotBusFromDataset(raw: string | undefined): number | null {
  if (raw === undefined || raw === "") return null;
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) ? n : null;
}

function hwSlotBusFromSelectedParamsEl(): number | null {
  return hwSlotBusFromDataset(selectedParamsSlotEl?.dataset.hwSlotBus);
}

function pickerCategoryForHwSlotBus(bus: number): "Split" | "Merge" | null {
  if (bus === HW_SLOT_BUS_SPLIT) return "Split";
  if (bus === HW_SLOT_BUS_MERGE) return "Merge";
  return null;
}

function lockPickerCategoryFromHwSlotBus(
  bus: number,
  highlightModelId?: string | null,
): void {
  const cat = pickerCategoryForHwSlotBus(bus);
  if (!cat || !catalogPickerDataCache || !slotPickerCategoryEl) return;
  applySlotPickerRoutingLock(cat, highlightModelId ?? null);
}

/** Positionne le picker Split/Merge dès que le bus structurel est connu (sans attendre le catalogue FX). */
function applyPickerForStructuralSlot(
  slot: SlotDebug,
  hwSlotBus?: number | null,
): void {
  const fromDom = hwSlotBus ?? hwSlotBusFromSelectedParamsEl();
  let bus = fromDom;
  if (bus == null) {
    const nk = normalizeCategory(slot.category);
    if (nk === "split") bus = HW_SLOT_BUS_SPLIT;
    else if (nk === "merge") bus = HW_SLOT_BUS_MERGE;
  }
  if (bus == null || !pickerCategoryForHwSlotBus(bus)) return;
  selectedSpecialHwSlotBus = bus;
  if (catalogPickerDataCache) {
    if (bus === HW_SLOT_BUS_SPLIT) {
      applySlotPickerRoutingLock("Split");
    } else {
      lockPickerCategoryFromHwSlotBus(
        bus,
        (slot.catalogModelId ?? "HD2_AppDSPFlowJoin").trim() || null,
      );
    }
  }
  void mountModelsSlotPicker().then(() => {
    if (bus === HW_SLOT_BUS_SPLIT) {
      void syncSplitPickerHighlightAsync(
        (slot.catalogModelId ?? "").trim(),
        slot.moduleHex,
      );
    } else {
      lockPickerCategoryFromHwSlotBus(
        bus,
        (slot.catalogModelId ?? "HD2_AppDSPFlowJoin").trim() || null,
      );
    }
  });
}
/** Dernière source Input écrite sur le preset actif (preset_data pas encore à jour). */
let path1InputSourceHighlightOverride: string | null = null;
/** Wire `@input` Path 1 mémorisé pour l’icône matrice (scroll / live write). */
let path1InputMatrixWire: number | null = null;
/** Dernier type Split écrit (wire live / preset pas encore à jour). */
let path1SplitTypeHighlightOverride: string | null = null;
let splitScrollParamsReloadTimer: number | null = null;

function scheduleSplitScrollParamsReload(slot: SlotDebug): void {
  if (splitScrollParamsReloadTimer !== null) {
    window.clearTimeout(splitScrollParamsReloadTimer);
  }
  splitScrollParamsReloadTimer = window.setTimeout(() => {
    splitScrollParamsReloadTimer = null;
    if (
      selectedParamsSlotEl &&
      hwSlotBusFromSelectedParamsEl() === HW_SLOT_BUS_SPLIT
    ) {
      void loadAndShowModelsParamsForSlot(slot, undefined);
    }
  }, 150);
}

function clearPath1InputSourceHighlightOverride(): void {
  path1InputSourceHighlightOverride = null;
}

function clearPath1InputMatrixWire(): void {
  path1InputMatrixWire = null;
}

function clearPath1SplitTypeHighlightOverride(): void {
  path1SplitTypeHighlightOverride = null;
}

function syncInputPickerHighlight(
  catalogModelId: string,
  chainValues: readonly (ChainParamValueJson | undefined)[] | null | undefined,
  inputParamChainIndex: number,
): void {
  void syncInputPickerHighlightAsync(catalogModelId, chainValues, inputParamChainIndex);
}

async function syncInputPickerHighlightAsync(
  catalogModelId: string,
  chainValues: readonly (ChainParamValueJson | undefined)[] | null | undefined,
  inputParamChainIndex: number,
): Promise<void> {
  if (!catalogPickerDataCache) return;
  const idTrim = catalogModelId.trim();
  if (!idTrim) return;
  let highlight = path1InputSourceHighlightOverride;
  if (!highlight) {
    try {
      const liveWire = await invoke<number | null>("get_path1_input_source_wire_value");
      if (liveWire != null && Number.isFinite(liveWire)) {
        highlight = findIoSourceIdByWireValue(
          catalogPickerDataCache,
          idTrim,
          liveWire,
          connectedDeviceName,
        );
      }
    } catch {
      /* wire live optionnel */
    }
  }
  if (!highlight) {
    highlight = findIoSourceIdFromInputChainValues(
      catalogPickerDataCache,
      idTrim,
      chainValues,
      inputParamChainIndex,
      connectedDeviceName,
    );
  }
  applySlotPickerIoLock("Input", idTrim, highlight);
  if (highlight) {
    const row = findIoSourceRowById(catalogPickerDataCache, highlight);
    if (typeof row?.wireValue === "number") path1InputMatrixWire = row.wireValue;
    void refreshPath1InputMatrixIcon();
  }
}

async function syncSplitPickerHighlightAsync(
  catalogModelId: string,
  moduleHex?: string | null,
): Promise<void> {
  if (!catalogPickerDataCache) return;
  const idTrim = catalogModelId.trim();
  let highlight = path1SplitTypeHighlightOverride;
  if (!highlight) {
    try {
      const liveWire = await invoke<number | null>("get_path1_split_type_wire_value");
      if (liveWire != null && Number.isFinite(liveWire)) {
        highlight = findSplitSourceIdByWireValue(
          catalogPickerDataCache,
          liveWire,
          connectedDeviceName,
        );
      }
    } catch {
      /* wire live optionnel */
    }
  }
  if (!highlight && idTrim) {
    highlight = findSplitSourceIdByCatalogModelId(
      catalogPickerDataCache,
      idTrim,
      connectedDeviceName,
    );
  }
  if (!highlight) {
    const wire = splitWireFromChainHex(moduleHex);
    if (wire != null) {
      highlight = findSplitSourceIdByWireValue(
        catalogPickerDataCache,
        wire,
        connectedDeviceName,
      );
    }
  }
  if (highlight) {
    applySlotPickerRoutingLock("Split", highlight);
  }
}

async function refreshSplitPickerFromLiveWireDelayed(): Promise<void> {
  await new Promise((r) => window.setTimeout(r, 180));
  if (!catalogPickerDataCache || slotPickerIoLock?.kind !== "routing" || slotPickerIoLock.category !== "Split") {
    return;
  }
  try {
    const liveWire = await invoke<number | null>("get_path1_split_type_wire_value");
    if (liveWire == null || !Number.isFinite(liveWire)) return;
    const id = findSplitSourceIdByWireValue(
      catalogPickerDataCache,
      liveWire,
      connectedDeviceName,
    );
    if (id) applySlotPickerFromCatalogSelection("Split", "Mono", id);
  } catch {
    /* ignore */
  }
}

async function refreshInputPickerFromLiveWireDelayed(): Promise<void> {
  await new Promise((r) => window.setTimeout(r, 180));
  if (
    !catalogPickerDataCache ||
    slotPickerIoLock?.kind !== "io" ||
    slotPickerIoLock.category !== "Input"
  ) {
    return;
  }
  const parentId = slotPickerIoLock.parentModelId;
  try {
    const liveWire = await invoke<number | null>("get_path1_input_source_wire_value");
    if (liveWire == null || !Number.isFinite(liveWire)) return;
    const id = findIoSourceIdByWireValue(
      catalogPickerDataCache,
      parentId,
      liveWire,
      connectedDeviceName,
    );
    if (id) applySlotPickerFromCatalogSelection("Input", "Source", id);
  } catch {
    /* ignore */
  }
}

function clearSlotPickerIoLock(): void {
  slotPickerIoLock = null;
  clearPath1InputSourceHighlightOverride();
  clearPath1SplitTypeHighlightOverride();
  if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = false;
  if (slotPickerSubEl) slotPickerSubEl.disabled = false;
}

function clearSpecialSlotPickerContext(): void {
  selectedSpecialHwSlotBus = null;
  releaseCabPickerLockFromDualSlots();
  clearSlotPickerIoLock();
}

function applySlotPickerIoLock(
  category: "Input" | "Output",
  catalogModelId: string,
  highlightIoSourceId?: string | null,
): void {
  const id = catalogModelId.trim();
  if (!id) return;
  slotPickerIoLock = { kind: "io", category, parentModelId: id };
  if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = true;
  if (slotPickerSubEl) slotPickerSubEl.disabled = true;
  const subs = catalogPickerDataCache?.subcategoriesByCategory.get(category) ?? [];
  const sub =
    category === "Input" && subs.includes("Source") ? "Source" : subs.includes("Mono") ? "Mono" : subs[0] ?? "Mono";
  applySlotPickerFromCatalogSelection(category, sub, highlightIoSourceId ?? null);
}

/** Split ou Merge : une seule catégorie dans le picker (tous les splits ou le mixer). */
function applySlotPickerRoutingLock(
  category: "Split" | "Merge",
  highlightModelId?: string | null,
): void {
  slotPickerIoLock = { kind: "routing", category };
  if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = true;
  if (slotPickerSubEl) slotPickerSubEl.disabled = true;
  const subs = catalogPickerDataCache?.subcategoriesByCategory.get(category) ?? [];
  const sub = subs.includes("Mono") ? "Mono" : subs[0] ?? "Mono";
  applySlotPickerFromCatalogSelection(category, sub, highlightModelId ?? null);
}

/** Onglet Cab (Amp+Cab) : liste limitée à Cab / Single (IR) ou Single Legacy (hybrid). */
function applySlotPickerAmpCabCabLock(
  highlightCabModelId: string,
  lockedSub: string = "Single",
): void {
  const id = highlightCabModelId.trim();
  const sub = lockedSub.trim() || "Single";
  slotPickerIoLock = { kind: "amp_cab_cab", highlightModelId: id, lockedSub: sub };
  if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = true;
  if (slotPickerSubEl) slotPickerSubEl.disabled = true;
  applySlotPickerFromCatalogSelection("Cab", sub, id || null);
}

/** Libère le verrou Cab / Single (Amp+Cab onglet Cab, Cab dual) et ré-ouvre le picker. */
function releaseCabPickerLockFromDualSlots(): void {
  ampCabDualPickerSync = null;
  ampCabDualActiveTab = 0;
  cabDualPickerSync = null;
  cabDualActiveTab = 0;
  if (slotPickerIoLock?.kind !== "amp_cab_cab") {
    if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = false;
    if (slotPickerSubEl) slotPickerSubEl.disabled = false;
    return;
  }
  slotPickerIoLock = null;
  if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = false;
  if (slotPickerSubEl) slotPickerSubEl.disabled = false;
  refillSlotPickerSubcategories();
  refillSlotPickerModelList(null);
}

function clearAmpCabDualPickerContext(): void {
  ampCabDualPickerSync = null;
  ampCabDualActiveTab = 0;
  if (slotPickerIoLock?.kind === "amp_cab_cab") {
    slotPickerIoLock = null;
    if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = false;
    if (slotPickerSubEl) slotPickerSubEl.disabled = false;
    refillSlotPickerSubcategories();
    refillSlotPickerModelList(null);
  }
}

function clearCabDualPickerContext(): void {
  const ki =
    lastCabDualTabPanesContext?.kemplineSlotIndex ??
    selectedParamsKemplineSlotIndex ??
    -1;
  cabDualPickerSync = null;
  cabDualActiveTab = 0;
  lastCabDualTabPanesContext = null;
  if (currentPresetIndex >= 0 && ki >= 0 && ki <= 15) {
    clearSlotDualPartsSessionForKemplineSlot(currentPresetIndex, ki);
  }
}

/** Quitte le mode Cab dual (onglets + verrou picker) avant un assign slot entier. */
function exitCabDualPickerModeForFullSlotReplace(): void {
  clearCabDualPickerContext();
  if (slotPickerIoLock?.kind === "amp_cab_cab") {
    slotPickerIoLock = null;
    if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = false;
    if (slotPickerSubEl) slotPickerSubEl.disabled = false;
    refillSlotPickerSubcategories();
  }
}

/** Onglet Cab 2 : picker figé sur Cab / sous-cat single IR + surbrillance. Cab 1 = listes libérées. */
function applySlotPickerCabDualCabLock(pickerSub: string, highlightCabModelId: string): void {
  const id = highlightCabModelId.trim();
  const lockedSub = pickerSub.trim() || "Single";
  slotPickerIoLock = { kind: "amp_cab_cab", highlightModelId: id, lockedSub };
  if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = true;
  if (slotPickerSubEl) slotPickerSubEl.disabled = true;
  applySlotPickerFromCatalogSelection("Cab", lockedSub, id || null);
}

/** ID catalogue Cab 2 pour le picker — pane affiché (titre) avant hint probe / cache. */
async function resolveCabDualCab2CatalogModelId(
  dualCatalogModelId: string,
  dualTabPanes: DualTabPaneConfig[],
  kemplineSlotIndex: number | undefined,
  cab2IdOverride?: string | null,
): Promise<string> {
  const fromPane = dualTabPanes[1]?.catalogModelId?.trim();
  if (fromPane) return fromPane;
  const fromOverride = (cab2IdOverride ?? "").trim();
  if (fromOverride) return fromOverride;
  if (kemplineSlotIndex !== undefined && Number.isInteger(kemplineSlotIndex)) {
    const hwSlot = lastHwSyncNormalizedSlots?.[kemplineSlotIndex];
    const cab2HexTrame = hwSlot
      ? await resolveCabDualCab2HexFromTrame(hwSlot, kemplineSlotIndex)
      : "";
    if (cab2HexTrame) {
      const cab1Hex = cabDualWireParts(hwSlot?.moduleHex)?.cab1Hex;
      const fromTrame = await getCatalogModelIdForCabDualCab2Hex(
        dualCatalogModelId,
        cab2HexTrame,
        cab1Hex,
      );
      if (fromTrame) return fromTrame;
    }
    try {
      const dualParts = await resolveSlotDualParts(kemplineSlotIndex, {
        slot: hwSlot ?? undefined,
        catalogModelId: dualCatalogModelId,
        kind: "cab_dual",
      });
      const part = dualParts?.kind === "cab_dual" ? dualParts.parts[1] : null;
      const cab1Hex =
        dualParts?.kind === "cab_dual" ? dualParts.parts[0]?.chainHex?.trim() : undefined;
      const hex = part?.chainHex?.trim();
      if (hex) {
        const fromDualCtx = await getCatalogModelIdForCabDualCab2Hex(
          dualCatalogModelId,
          hex,
          cab1Hex,
        );
        if (fromDualCtx) return fromDualCtx;
      }
      const fromPartId = part?.modelId?.trim();
      if (fromPartId) return fromPartId;
      if (hex) {
        const fromHex = (await getCatalogModelIdForCabSingleHex(hex))?.trim();
        if (fromHex) return fromHex;
      }
    } catch {
      /* repli assign ci-dessous */
    }
  }
  const wire = cabDualWireParts(
    lastHwSyncNormalizedSlots?.[kemplineSlotIndex ?? -1]?.moduleHex,
  );
  const cab2FromWire = wire?.cab2Hex;
  if (cab2FromWire) {
    const fromWire = await getCatalogModelIdForCabDualCab2Hex(
      dualCatalogModelId,
      cab2FromWire,
      wire.cab1Hex,
    );
    if (fromWire) return fromWire;
  }
  const pair = await cabDualHexPairFromAssignVariant(dualCatalogModelId, "dual");
  if (pair) {
    return (
      (await getCatalogModelIdForCabDualCab2Hex(
        dualCatalogModelId,
        pair.cab2Hex,
        pair.cab1Hex,
      ))?.trim() ?? ""
    );
  }
  return "";
}

/** Id catalogue **single** pour affichage / surbrillance (preset, titre, picker). */
async function resolveCabDualCab2PickerCatalogId(
  cab2CatalogModelId: string,
  legacyFallback: boolean,
): Promise<string> {
  const wire = await resolveCabDualWireCabTarget(cab2CatalogModelId, legacyFallback);
  return wire.cabCatalogModelId;
}

/**
 * Entrée `HX_ModelUsbAssign` pour replace **cab2** : hint dual wire (`c319`), pas single `c219`.
 * IR Mic : `Foo` → `FooWithPan` variant `dual`. Legacy : même id, variant `dual`.
 */
function resolveCabDualCab2UsbWireFromPicker(
  pickedCatalogModelId: string,
): { cabCatalogModelId: string; cabAssignVariant: string } {
  const id = pickedCatalogModelId.trim();
  if (!id) {
    return { cabCatalogModelId: id, cabAssignVariant: "dual" };
  }
  if (catalogPickerDataCache) {
    if (findUsbAssignPickerLocation(catalogPickerDataCache, id, "dual", "Cab")) {
      return { cabCatalogModelId: id, cabAssignVariant: "dual" };
    }
    if (!id.endsWith("WithPan")) {
      const withPan = `${id}WithPan`;
      if (findUsbAssignPickerLocation(catalogPickerDataCache, withPan, "dual", "Cab")) {
        return { cabCatalogModelId: withPan, cabAssignVariant: "dual" };
      }
    }
  }
  if (!id.endsWith("WithPan")) {
    return { cabCatalogModelId: `${id}WithPan`, cabAssignVariant: "dual" };
  }
  return { cabCatalogModelId: id, cabAssignVariant: "dual" };
}

function resolveCabDualCab2PickerSub(
  cab2CatalogModelId: string,
  slotCategory: string,
  legacyFallback: boolean,
): string {
  if (!catalogPickerDataCache) {
    return cabDualCab2PickerSub(legacyFallback);
  }
  const loc = findCabModelPickerLocation(
    catalogPickerDataCache,
    cab2CatalogModelId,
    slotCategory,
  );
  // Cab 2 = fil single IR : ne jamais verrouiller le picker sur Dual / WithPan.
  if (loc && (loc.assignVariant === "single" || loc.assignVariant === "legacy")) {
    return loc.subKey;
  }
  return cabDualCab2PickerSub(legacyFallback);
}

/** Cab 2 affiché (preset / picker) : id catalogue **single** ou legacy. */
async function resolveCabDualWireCabTarget(
  pickedCatalogModelId: string,
  legacyFallback: boolean,
): Promise<{ cabCatalogModelId: string; cabAssignVariant: string }> {
  const id = pickedCatalogModelId.trim();
  const fallbackVariant = legacyFallback ? "legacy" : "single";
  if (catalogPickerDataCache) {
    if (findUsbAssignPickerLocation(catalogPickerDataCache, id, "single", "Cab")) {
      return { cabCatalogModelId: id, cabAssignVariant: "single" };
    }
    if (findUsbAssignPickerLocation(catalogPickerDataCache, id, "legacy", "Cab")) {
      return { cabCatalogModelId: id, cabAssignVariant: "legacy" };
    }
    if (id.endsWith("WithPan")) {
      const singleId = id.slice(0, -"WithPan".length);
      if (
        findUsbAssignPickerLocation(catalogPickerDataCache, singleId, "single", "Cab")
      ) {
        return { cabCatalogModelId: singleId, cabAssignVariant: "single" };
      }
    }
  }
  const meta = await getPresetMetaForId(id);
  const hexRaw = meta?.chainHex;
  const hex = (Array.isArray(hexRaw) ? hexRaw[0] : hexRaw ?? "").trim();
  if (hex) {
    const fromHex = (await getCatalogModelIdForCabSingleHex(hex))?.trim();
    if (fromHex) {
      return { cabCatalogModelId: fromHex, cabAssignVariant: fallbackVariant };
    }
  }
  return { cabCatalogModelId: id, cabAssignVariant: fallbackVariant };
}

async function armCabDualPickerSync(
  dualTabPanes: DualTabPaneConfig[],
  dualCatalogModelId: string,
  meta: PresetMetaJson | null,
  slot: SlotDebug,
  kemplineSlotIndex: number | undefined,
  cab2IdOverride?: string | null,
): Promise<void> {
  if (!catalogPickerDataCache) return;
  const dualIdTrim = dualCatalogModelId.trim();
  const cab1Id = dualTabPanes[0]?.catalogModelId?.trim() ?? dualIdTrim;
  const cab2Id = await resolveCabDualCab2CatalogModelId(
    dualIdTrim,
    dualTabPanes,
    kemplineSlotIndex,
    cab2IdOverride,
  );
  if (!cab2Id) {
    cabDualPickerSync = null;
    logCabDualTrace(
      `armCabDualPickerSync: cab2Id introuvable dual=${dualIdTrim} slot=${kemplineSlotIndex ?? "?"}`,
    );
    return;
  }
  const dualMeta = await getPresetMetaForId(dualIdTrim);
  const legacy = isCabLegacyFromMeta(dualMeta) || isCabLegacyFromMeta(meta);
  const cab1PickerSub = cabDualCab1PickerSub(legacy);
  const cab2PickerSub = resolveCabDualCab2PickerSub(
    cab2Id,
    slot.category,
    legacy,
  );
  cabDualPickerSync = {
    dualCatalogModelId: dualIdTrim,
    cab1CatalogModelId: cab1Id,
    cab1PickerModelId: dualIdTrim,
    cab2CatalogModelId: cab2Id,
    cab1PickerSub,
    cab2PickerSub,
    meta,
    moduleHex: slot.moduleHex,
    slotCategory: slot.category,
  };
}

async function syncPickerForCabDualTab(tabIndex: 0 | 1): Promise<void> {
  if (!cabDualPickerSync || !catalogPickerDataCache) return;
  cabDualActiveTab = tabIndex;
  const ctx = cabDualPickerSync;
  if (tabIndex === 0) {
    // INVARIANT : Cab 1 = pas de lock liste (voir commentaire `cabDualPickerSync`).
    if (slotPickerIoLock?.kind === "amp_cab_cab") {
      slotPickerIoLock = null;
      if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = false;
      if (slotPickerSubEl) slotPickerSubEl.disabled = false;
      refillSlotPickerSubcategories();
    }
    const legacy =
      isCabLegacyFromMeta(ctx.meta) ||
      isCabLegacyFromMeta(await getPresetMetaForId(ctx.dualCatalogModelId));
    applySlotPickerFromCatalogSelection(
      "Cab",
      cabDualCab1PickerSub(legacy),
      ctx.dualCatalogModelId,
    );
    return;
  }
  const ki = lastCabDualTabPanesContext?.kemplineSlotIndex;
  const pane2Id =
    lastCabDualTabPanesContext?.dualTabPanes[1]?.catalogModelId?.trim() ?? "";
  let cab2Id = pane2Id || ctx.cab2CatalogModelId.trim();
  if (!pane2Id && ki !== undefined && Number.isInteger(ki)) {
    const fromPreset = await resolveCabDualCab2IdFromPreset(ki, ctx.dualCatalogModelId);
    if (fromPreset) cab2Id = fromPreset;
  }
  if (!cab2Id) {
    cab2Id = ctx.cab2CatalogModelId.trim();
  }
  ctx.cab2CatalogModelId = cab2Id;
  const legacy =
    isCabLegacyFromMeta(ctx.meta) ||
    isCabLegacyFromMeta(await getPresetMetaForId(ctx.dualCatalogModelId));
  ctx.cab2PickerSub = resolveCabDualCab2PickerSub(
    cab2Id,
    ctx.slotCategory,
    legacy,
  );
  const cab2Highlight = await resolveCabDualCab2PickerCatalogId(cab2Id, legacy);
  ctx.cab2CatalogModelId = cab2Highlight;
  applySlotPickerCabDualCabLock(ctx.cab2PickerSub, cab2Highlight);
  logCabDualTrace(`picker sync cab2 highlight=${cab2Highlight}`);
}

/** Réarme le picker Cab dual (mount HW / clic onglet Cab 2). */
async function ensureCabDualPickerSynced(tabIndex: 0 | 1): Promise<void> {
  await mountModelsSlotPicker();
  const mountCtx = lastCabDualTabPanesContext;
  if (mountCtx) {
    await mountCabDualPickerSyncForSlot(
      mountCtx.dualTabPanes,
      mountCtx.dualCatalogModelId,
      mountCtx.meta,
      mountCtx.slot,
      mountCtx.kemplineSlotIndex,
      tabIndex,
    );
    return;
  }
  if (cabDualPickerSync) {
    await syncPickerForCabDualTab(tabIndex);
  }
}

async function isAmpCabSlotLegacy(
  slotCategory: string,
  linkedCabHex: string | null | undefined,
  cabCatalogModelId: string,
): Promise<boolean> {
  if (isAmpCabLegacySlotCategory(slotCategory)) return true;
  const cab = (linkedCabHex ?? "").trim();
  if (cab && (await isLegacyCabChainHex(cab))) return true;
  const id = cabCatalogModelId.trim();
  if (id && isCabLegacyFromMeta(await getPresetMetaForId(id))) return true;
  return false;
}

async function syncPickerForAmpCabDualTab(tabIndex: 0 | 1): Promise<void> {
  if (!ampCabDualPickerSync || !catalogPickerDataCache) return;
  ampCabDualActiveTab = tabIndex;
  const ctx = ampCabDualPickerSync;
  if (tabIndex === 1) {
    const legacy = await isAmpCabSlotLegacy(
      ctx.slotCategory,
      ctx.linkedCabHex,
      ctx.cabCatalogModelId,
    );
    const lockedSub = resolveCabDualCab2PickerSub(
      ctx.cabCatalogModelId,
      ctx.slotCategory,
      legacy,
    );
    applySlotPickerAmpCabCabLock(ctx.cabCatalogModelId, lockedSub);
    return;
  }
  if (slotPickerIoLock?.kind === "amp_cab_cab") {
    slotPickerIoLock = null;
    if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = false;
    if (slotPickerSubEl) slotPickerSubEl.disabled = false;
    refillSlotPickerSubcategories();
  }
  await syncModelsSlotPickerFromLoadedModel(
    ctx.ampCatalogModelId,
    ctx.meta,
    ctx.moduleHex,
    ctx.slotCategory,
    ctx.linkedCabHex,
  );
}

function refillSlotPickerSubcategories(): void {
  if (!slotPickerCategoryEl || !slotPickerSubEl || !catalogPickerDataCache) return;
  const cat = slotPickerCategoryEl.value.trim();
  slotPickerSubEl.replaceChildren();
  if (slotPickerIoLock?.kind === "amp_cab_cab") {
    const lockedSub = slotPickerIoLock.lockedSub;
    const o = document.createElement("option");
    o.value = lockedSub;
    o.textContent = lockedSub;
    slotPickerSubEl.appendChild(o);
    slotPickerSubEl.value = lockedSub;
    slotPickerSubEl.disabled = true;
    return;
  }
  const emptyOpt = document.createElement("option");
  emptyOpt.value = "";
  emptyOpt.textContent = "—";
  slotPickerSubEl.appendChild(emptyOpt);
  if (!cat) {
    slotPickerSubEl.disabled = true;
    slotPickerSubEl.value = "";
    return;
  }
  const subs = catalogPickerDataCache.subcategoriesByCategory.get(cat) ?? [];
  for (const s of subs) {
    const o = document.createElement("option");
    o.value = s;
    o.textContent = s;
    slotPickerSubEl.appendChild(o);
  }
  slotPickerSubEl.disabled = subs.length === 0;
  slotPickerSubEl.value = subs.length > 0 ? subs[0]! : "";
}

function refillSlotPickerModelList(highlightModelId: string | null | undefined): void {
  if (!slotPickerListEl || !catalogPickerDataCache || !slotPickerCategoryEl || !slotPickerSubEl) return;
  const cat = slotPickerCategoryEl.value.trim();
  const sub = slotPickerSubEl.value;
  slotPickerListEl.replaceChildren();
  if (!cat || !sub) return;
  const key = catalogPickerRowKey(cat, sub);
  const wantVariant = usbAssignVariantFromPickerSub(sub, cat);
  const bucket = catalogPickerDataCache.modelsByCategoryAndSub.get(key) ?? [];
  // Send/Return : variante USB unique `sendReturn` ; le regroupement picker reste Mono/Stereo.
  let rows =
    wantVariant === "sendreturn"
      ? bucket
      : bucket.filter(
          (row) => (row.assignVariant ?? "mono").toLowerCase() === wantVariant,
        );
  if (slotPickerIoLock?.kind === "io" && slotPickerIoLock.category === cat) {
    const parentId = slotPickerIoLock.parentModelId;
    if (cat === "Input") {
      const sources = rows.filter(
        (row) =>
          row.ioSource &&
          row.parentModelId === parentId &&
          ioSourceMatchesConnectedDevice(row.devices, connectedDeviceName),
      );
      rows = sources.length > 0 ? sources : rows.filter((row) => row.id === parentId);
    } else {
      rows = rows.filter((row) => row.id === parentId);
    }
  } else if (
    slotPickerIoLock?.kind === "routing" &&
    slotPickerIoLock.category === "Split" &&
    cat === "Split"
  ) {
    const sources = rows.filter(
      (row) =>
        row.splitSource &&
        ioSourceMatchesConnectedDevice(row.devices, connectedDeviceName),
    );
    if (sources.length > 0) rows = sources;
  } else if (
    slotPickerIoLock?.kind === "amp_cab_cab" &&
    cabDualActiveTab === 1 &&
    (cabDualPickerSync || lastCabDualTabPanesContext)
  ) {
    // Cab 2 : liste Single IR (pas Dual / WithPan dans le picker).
    rows = rows.filter((row) => {
      const v = (row.assignVariant ?? "").trim().toLowerCase();
      if (v === "dual") return false;
      if (row.id.trim().endsWith("WithPan")) return false;
      return true;
    });
  }
  if (rows.length === 0) {
    const hint = document.createElement("li");
    hint.className = "models-slot-picker-model-item models-slot-picker-model-item--hint";
    hint.textContent = !sub
      ? "Choisir Mono ou Stereo ci-dessus"
      : "Aucun modèle pour cette combinaison";
    slotPickerListEl.appendChild(hint);
    return;
  }
  const hi = (highlightModelId ?? "").trim();
  for (const row of rows) {
    const li = document.createElement("li");
    li.className = "models-slot-picker-model-item";
    li.textContent = row.name;
    li.title = row.ioSource
      ? `${row.id} · ioSource → ${row.parentModelId ?? ""}`
      : row.splitSource
        ? `${row.id} · splitSource → ${row.catalogModelId ?? ""}`
        : row.assignVariant !== undefined
          ? `${row.id} · ${row.assignVariant}`
          : row.id;
    li.dataset.modelId = row.id;
    if (row.ioSource) li.dataset.ioSource = "1";
    if (row.splitSource) li.dataset.splitSource = "1";
    if (hi && row.id === hi) li.classList.add("models-slot-picker-model-item--active");
    li.addEventListener("click", () => {
      slotPickerListEl
        ?.querySelectorAll(".models-slot-picker-model-item")
        .forEach((n) => n.classList.remove("models-slot-picker-model-item--active"));
      li.classList.add("models-slot-picker-model-item--active");
      void applySlotModelFromPickerListClick(row);
    });
    slotPickerListEl.appendChild(li);
  }
  if (hi) {
    slotPickerListEl
      .querySelector(".models-slot-picker-model-item--active")
      ?.scrollIntoView({ block: "nearest", behavior: "auto" });
  }
}

function resetSlotPickerToIdle(): void {
  if (!slotPickerCategoryEl || !slotPickerSubEl || !slotPickerListEl) return;
  clearSpecialSlotPickerContext();
  slotPickerCategoryEl.value = "";
  refillSlotPickerSubcategories();
  slotPickerSubEl.value = "";
  refillSlotPickerModelList(null);
}

/** Variante USB pour `HX_ModelUsbAssign.json` : alignée sur catégorie picker + sous-catégorie. */
function usbAssignVariantFromPickerSub(sub: string, pickerCategory?: string): string {
  const cat = (pickerCategory ?? "").trim().toLowerCase();
  if (cat === "send/return") return "sendReturn";
  if (cat === "amp") return "amp";
  if (cat === "preamp") return "preamp";
  if (cat.includes("amp") && cat.includes("legacy")) return "amp+cab-legacy";
  if (cat === "amp+cab") return "amp+cab";
  const t = sub.trim().toLowerCase();
  if (cat === "cab") {
    // HX_ModelUsbAssign : Cab hybrid = subCategory Legacy + variant single|dual (pas variant legacy).
    if (t === "single legacy") return "single";
    if (t === "single") return "single";
    if (t === "dual legacy") return "dual";
    if (t === "dual") return "dual";
  }
  if ((t.includes("guitar") || t.includes("bass")) && t.includes("legacy")) {
    return "amp+cab-legacy";
  }
  if (t.includes("legacy")) return "legacy";
  if (t.includes("stereo") || t.includes("stéréo")) return "stereo";
  if (t.includes("mono")) return "mono";
  if (t === "single") return "single";
  if (t === "dual") return "dual";
  return "mono";
}

function patchMatrixSlotVisualFromSlot(ki: number, slot: SlotDebug): void {
  const nodes = contentEl.querySelectorAll<HTMLElement>(`[data-kempline-slot-index="${ki}"]`);
  const preserveSelection =
    selectedParamsKemplineSlotIndex === ki &&
    selectedParamsPresetIndex === currentPresetIndex;
  let slotsCtx = lastHwSyncNormalizedSlots;
  if (slotsCtx && slotsCtx.length === 16) {
    slotsCtx = slotsCtx.map((s, i) => (i === ki ? { ...slot } : { ...s }));
  }
  for (const old of nodes) {
    const inner = gridSlotNode(slot, ki, slotsCtx ?? undefined);
    const wasSelected =
      preserveSelection &&
      (old === selectedParamsSlotEl || old.classList.contains("node--selected"));
    old.replaceWith(inner);
    if (wasSelected) {
      selectedParamsSlotEl = inner;
      inner.classList.add("node--selected");
      if (!isEmptyGridCell(slot)) {
        attachSelectedSlotRemoveButton(inner, ki);
      }
      selectedParamsSlotKey = makeSlotSelectionKey(slot, ki);
      if (isEmptyGridCell(slot)) {
        releaseCabPickerLockFromDualSlots();
      }
    }
  }
  if (slotsCtx && slotsCtx.length === 16) {
    refreshColumnPairedEmptySlotVisual(slotsCtx, ki);
  }
}

function refreshColumnPairedEmptySlotVisual(slots: SlotDebug[], kemplineSlotIndex: number): void {
  const paired = pairedKemplineSlotIndex(kemplineSlotIndex);
  if (paired === null) return;
  const pairedSlot = slots[paired];
  if (!pairedSlot || !isEmptyGridCell(pairedSlot)) return;
  const nodes = contentEl.querySelectorAll<HTMLElement>(`[data-kempline-slot-index="${paired}"]`);
  for (const old of nodes) {
    old.replaceWith(gridSlotNode(pairedSlot, paired, slots));
  }
}

/**
 * Clic sur une ligne du picker : MAJ immédiate pastille + paramètres (catalogue / défauts `.models`),
 * puis ordre USB `probe_slot_model_usb`.
 */
async function applyPath1InputSourceFromPicker(row: CatalogPickerModelRow): Promise<void> {
  const slotKey = selectedParamsSlotKey ?? "";
  if (!slotKey.startsWith("input|")) {
    console.warn(
      "[Path1Input] cliquez d’abord le bloc Input Path 1 sur la matrice.",
    );
    return;
  }
  if (selectedParamsPresetIndex !== currentPresetIndex) return;
  markLiveWriteUiInteraction();
  try {
    path1InputSourceHighlightOverride = row.id;
    if (typeof row.wireValue === "number") path1InputMatrixWire = row.wireValue;
    const out = await invoke<string>("write_path1_input_source", { ioSourceId: row.id });
    console.info("[Path1Input]", row.name, out);
    void refreshPath1InputMatrixIcon();
    await syncInputPickerHighlightAsync(
      flowIoCatalogIdsForConnectedDevice(connectedDeviceName).input,
      null,
      0,
    );
    if (selectedParamsSlotEl) {
      const slot = path1IoSlot("input");
      await loadAndShowModelsParamsForSlot(slot, undefined);
    }
  } catch (e) {
    console.warn("[Path1Input]", e);
  }
}

async function applyPath1SplitTypeFromPicker(row: CatalogPickerModelRow): Promise<void> {
  if (slotPickerIoLock?.kind !== "routing" || slotPickerIoLock.category !== "Split") {
    console.warn("[Path1Split] cliquez d’abord le Split Path 1 sur la matrice.");
    return;
  }
  if (selectedParamsPresetIndex !== currentPresetIndex) return;
  markLiveWriteUiInteraction();
  try {
    path1SplitTypeHighlightOverride = row.id;
    const out = await invoke<string>("write_path1_split_type", { splitSourceId: row.id });
    console.info("[Path1Split]", row.name, out);
    const catId = (row.catalogModelId ?? "").trim();
    const wire = row.wireValue ?? 0;
    await syncSplitPickerHighlightAsync(catId, splitChainHexFromWire(wire));
    if (selectedParamsSlotEl && catId) {
      const slot: SlotDebug = {
        category: "Split",
        name: row.name,
        moduleHex: splitChainHexFromWire(wire),
        catalogModelId: catId,
      };
      await loadAndShowModelsParamsForSlot(slot, undefined);
    }
  } catch (e) {
    console.warn("[Path1Split]", e);
  }
}

async function applyAmpCabCabFromPickerListClick(
  row: CatalogPickerModelRow,
  ki: number,
): Promise<void> {
  const ctx = ampCabDualPickerSync;
  if (!ctx) return;

  const cabIdTrim = row.id.trim();
  const legacy = await isAmpCabSlotLegacy(
    ctx.slotCategory,
    ctx.linkedCabHex,
    cabIdTrim,
  );
  const cabUsb = await resolveCabDualWireCabTarget(cabIdTrim, legacy);
  const cabAssignVariant = cabUsb.cabAssignVariant;
  const cabIdForUsb = cabUsb.cabCatalogModelId;
  const ampIdTrim = ctx.ampCatalogModelId.trim();
  const ampAssignVariant = await usbAssignVariantForAmpCabSlot(
    ctx.meta,
    ctx.moduleHex,
    ctx.slotCategory,
    ampIdTrim,
    ctx.linkedCabHex,
  );
  const cabMeta = await getPresetMetaForId(cabIdForUsb);
  const cabHex =
    (await moduleHexForUsbVariant(cabIdForUsb, cabAssignVariant, cabMeta))?.trim() || "";

  const prevSnapshot =
    lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16
      ? lastHwSyncNormalizedSlots.map((s) => ({ ...s }))
      : null;
  const prevSlot = prevSnapshot?.[ki];
  const optimisticSlot: SlotDebug = {
    category: ctx.slotCategory,
    name: (prevSlot?.name ?? "").trim() || ampIdTrim,
    catalogModelId: ampIdTrim,
    moduleHex: ctx.moduleHex ?? prevSlot?.moduleHex,
  };

  ctx.cabCatalogModelId = cabIdTrim;
  if (cabHex) ctx.linkedCabHex = cabHex;

  if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
    const next = lastHwSyncNormalizedSlots.map((s, i) =>
      i === ki ? { ...optimisticSlot } : { ...s },
    );
    lastHwSyncNormalizedSlots = next;
    lastHwSyncChainSignature = chainLayoutSignature(next);
  }

  patchMatrixSlotVisualFromSlot(ki, optimisticSlot);
  if (currentPresetIndex >= 0) {
    clearLiveChainOverridesForKemplineSlot(currentPresetIndex, ki);
  }
  markLiveWriteUiInteraction();
  slotModelUsbProbeInFlight = ki;
  lastProbePickerAssignContext = {
    ki,
    catalogModelId: ampIdTrim,
    assignVariant: ampAssignVariant,
    category: ctx.slotCategory,
    ampCabCabModelId: cabIdTrim,
    ampCabCabAssignVariant: cabAssignVariant,
  };
  ampCabDualActiveTab = 1;

  try {
    await loadAndShowModelsParamsFromCatalogDefaults(optimisticSlot, ampIdTrim, ki, {
      assignVariant: ampAssignVariant,
    });

    const out = await invoke<string>("probe_slot_model_usb", {
      op: "replace",
      slotIndex: ki,
      catalogModelId: ampIdTrim,
      assignVariant: ampAssignVariant,
      cabCatalogModelId: cabIdForUsb,
      cabAssignVariant,
    });
    console.info("[SlotModelProbe] amp+cab cab", row.name, cabIdTrim, out);
    mergeProbeSlotModelUntil = {
      ki,
      deadline: Date.now() + PROBE_SLOT_MERGE_GRACE_MS,
      mergeTraceEmitted: false,
    };
    suppressUsbPresetPollUntilMs = Date.now() + USB_PRESET_POLL_SUPPRESS_AFTER_PROBE_MS;
    lastSoftUsbPresetReadAt = Date.now();
    emitModelsSyncTrace(
      `slot_model_probe amp+cab cab ok slot=${ki} cab=${cabIdTrim}`,
    );
    await patchSlotDualPartsSessionAmpCabCab(ki, cabIdTrim, cabHex);
    await syncPickerForAmpCabDualTab(1);
  } catch (e) {
    console.warn("[SlotModelProbe] amp+cab cab", e);
    lastProbePickerAssignContext = null;
    if (prevSnapshot) {
      const old = prevSnapshot[ki]!;
      patchMatrixSlotVisualFromSlot(ki, old);
      lastHwSyncNormalizedSlots = prevSnapshot.map((s) => ({ ...s }));
      lastHwSyncChainSignature = chainLayoutSignature(lastHwSyncNormalizedSlots);
      if (selectedParamsKemplineSlotIndex === ki) {
        await loadAndShowModelsParamsForSlot(old, ki);
      }
    }
  } finally {
    slotModelUsbProbeInFlight = null;
  }
}

function logCabDualTrace(line: string): void {
  const msg = `[CabDual] ${line}`;
  console.info(msg);
  void invoke("log_frontend_message", { message: msg }).catch(() => {});
}

function cabDualPickerApplyContextForSlot(ki: number): boolean {
  const mount = lastCabDualTabPanesContext;
  if (mount) {
    const mKi = mount.kemplineSlotIndex;
    if (mKi === undefined || mKi === ki) return true;
  }
  if (cabDualPickerSync) return true;
  return slotPickerIoLock?.kind === "amp_cab_cab";
}

function isCabDualPickerApplyRoute(ki: number): boolean {
  if (!cabDualPickerApplyContextForSlot(ki)) return false;
  const slot = lastHwSyncNormalizedSlots?.[ki];
  // Ne casser le contexte cab dual QUE si le slot est devenu explicitement un fil cab dual modern
  // d'un AUTRE modèle. Le dual legacy (hints 1 octet) n'est pas reconnu par isCabDualWireHex /
  // cabDualWireParts ; dans ce cas on fait confiance au contexte d'onglets déjà monté.
  if (
    slot &&
    isCabDualWireHex(slot.moduleHex) === false &&
    cabDualWireParts(slot.moduleHex) === null &&
    !lastCabDualTabPanesContext &&
    !cabDualPickerSync
  ) {
    clearCabDualPickerContext();
    return false;
  }
  return Boolean(lastCabDualTabPanesContext || cabDualPickerSync);
}

/**
 * Clic picker = remplacement **cab1/cab2** dans le dual existant (pas un autre type de slot).
 * Cab 2 verrouillé Single IR → toujours sous-cab. Cab 1 libre → Cab single/legacy seulement.
 */
function isCabDualSubCabPickerPick(
  row: CatalogPickerModelRow,
  pickerCategory: string,
): boolean {
  if (normalizeCategory(pickerCategory) !== "cab") {
    return false;
  }
  if (cabDualActiveTab === 1) {
    return true;
  }
  const pickerSub = (slotPickerSubEl?.value ?? "").trim().toLowerCase();
  if (
    pickerSub === "single" ||
    pickerSub === "single legacy" ||
    pickerSub === "legacy"
  ) {
    return false;
  }
  const rowVariant = (row.assignVariant ?? "").trim().toLowerCase();
  if (rowVariant === "dual") {
    return false;
  }
  if (rowVariant === "single" || rowVariant === "legacy") {
    return true;
  }
  if (catalogPickerDataCache) {
    const id = row.id.trim();
    if (
      findUsbAssignPickerLocation(catalogPickerDataCache, id, "single", "Cab") ||
      findUsbAssignPickerLocation(catalogPickerDataCache, id, "legacy", "Cab")
    ) {
      return true;
    }
  }
  return false;
}

async function ensureCabDualPickerCtxForApply(
  ki: number,
): Promise<NonNullable<typeof cabDualPickerSync> | null> {
  if (cabDualPickerSync) return cabDualPickerSync;
  const mount = lastCabDualTabPanesContext;
  if (!mount) return null;
  if (mount.kemplineSlotIndex !== undefined && mount.kemplineSlotIndex !== ki) {
    return null;
  }
  await mountModelsSlotPicker();
  await armCabDualPickerSync(
    mount.dualTabPanes,
    mount.dualCatalogModelId,
    mount.meta,
    mount.slot,
    mount.kemplineSlotIndex,
  );
  return cabDualPickerSync;
}

async function applyCabDualCabFromPickerListClick(
  row: CatalogPickerModelRow,
  ki: number,
): Promise<void> {
  const ctx = await ensureCabDualPickerCtxForApply(ki);
  if (!ctx) {
    logCabDualTrace(
      `cab dual cab apply bloqué slot=${ki} tab=${cabDualActiveTab} (contexte picker indisponible)`,
    );
    return;
  }

  const tab = cabDualActiveTab;
  const newCabIdTrim = row.id.trim();
  const rowVariant = (row.assignVariant ?? "").trim().toLowerCase();
  const legacy =
    isCabLegacyFromMeta(ctx.meta) ||
    isCabLegacyFromMeta(await getPresetMetaForId(ctx.dualCatalogModelId));
  let cabIdForUsb = newCabIdTrim;
  let cabAssignVariant =
    tab === 1
      ? legacy
        ? "legacy"
        : "single"
      : rowVariant || "dual";
  /** Id single pour UI (titre, surbrillance, panneau params). */
  let cab2PickerCatalogId = newCabIdTrim;
  if (tab === 1) {
    const usb = resolveCabDualCab2UsbWireFromPicker(newCabIdTrim);
    cabIdForUsb = usb.cabCatalogModelId;
    cabAssignVariant = usb.cabAssignVariant;
    cab2PickerCatalogId = newCabIdTrim;
  }
  const dualIdTrim = ctx.dualCatalogModelId.trim();

  const prevSnapshot =
    lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16
      ? lastHwSyncNormalizedSlots.map((s) => ({ ...s }))
      : null;
  const prevSlot = prevSnapshot?.[ki];
  const optimisticSlot: SlotDebug = {
    category: ctx.slotCategory,
    name: (prevSlot?.name ?? "").trim() || dualIdTrim,
    catalogModelId: dualIdTrim,
    moduleHex: ctx.moduleHex ?? prevSlot?.moduleHex,
  };

  if (tab === 0) {
    ctx.cab1CatalogModelId = newCabIdTrim;
  } else {
    ctx.cab2CatalogModelId = cab2PickerCatalogId;
  }

  if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
    const next = lastHwSyncNormalizedSlots.map((s, i) =>
      i === ki ? { ...optimisticSlot } : { ...s },
    );
    lastHwSyncNormalizedSlots = next;
    lastHwSyncChainSignature = chainLayoutSignature(next);
  }

  patchMatrixSlotVisualFromSlot(ki, optimisticSlot);
  if (currentPresetIndex >= 0) {
    clearLiveChainOverridesForKemplineSlot(currentPresetIndex, ki);
  }
  markLiveWriteUiInteraction();
  slotModelUsbProbeInFlight = ki;
  lastProbePickerAssignContext = {
    ki,
    catalogModelId: dualIdTrim,
    assignVariant: "dual",
    category: ctx.slotCategory,
    cabDualCab2ModelId: tab === 1 ? cab2PickerCatalogId : ctx.cab2CatalogModelId,
  };

  try {
    logCabDualTrace(
      `probe cab${tab + 1} replace cab=${cabIdForUsb} variant=${cabAssignVariant} dual=${dualIdTrim} (picker=${newCabIdTrim})`,
    );
    const out = await invoke<string>("probe_slot_model_usb", {
      op: "replace",
      slotIndex: ki,
      catalogModelId: dualIdTrim,
      assignVariant: "dual",
      cabCatalogModelId: cabIdForUsb,
      cabAssignVariant,
      cabDualCabIndex: tab,
    });
    console.info("[SlotModelProbe] cab dual cab", tab + 1, row.name, cabIdForUsb, out);
    logCabDualTrace(`probe cab${tab + 1} ok: ${out.slice(0, 120)}`);

    await refreshCabDualContextAfterProbe(
      ki,
      tab === 1 ? cab2PickerCatalogId : cabIdForUsb,
      tab,
    );
    mergeProbeSlotModelUntil = {
      ki,
      deadline: Date.now() + PROBE_SLOT_MERGE_GRACE_MS,
      mergeTraceEmitted: false,
    };
    suppressUsbPresetPollUntilMs = Date.now() + USB_PRESET_POLL_SUPPRESS_AFTER_PROBE_MS;
    lastSoftUsbPresetReadAt = Date.now();
    emitModelsSyncTrace(
      `slot_model_probe cab dual cab${tab + 1} ok slot=${ki} cab=${newCabIdTrim}`,
    );
    if (tab === 1) {
      // Le fil optimiste (optimisticSlot.moduleHex) porte encore l'ANCIEN cab2 tant qu'il n'y a
      // pas eu de re-dump preset. Depuis que cabDualWireParts parse le legacy, le lire ici réinjecte
      // l'ancien cab2 dans la session -> l'UI affiche le cab2 précédent. On dérive le hex du cab
      // RÉELLEMENT choisi (cab2PickerCatalogId), comme source de vérité optimiste.
      const pickedMeta = await getPresetMetaForId(cab2PickerCatalogId);
      const cab2HexPicked =
        (await moduleHexForUsbVariant(cab2PickerCatalogId, "single", pickedMeta))?.trim() || "";
      await patchSlotDualPartsSessionCabDualCab2(ki, cab2PickerCatalogId, cab2HexPicked);
    }
  } catch (e) {
    const errMsg = e instanceof Error ? e.message : String(e);
    logCabDualTrace(`probe cab dual ERREUR: ${errMsg}`);
    console.warn("[SlotModelProbe] cab dual cab", e);
    lastProbePickerAssignContext = null;
    if (prevSnapshot) {
      const old = prevSnapshot[ki]!;
      patchMatrixSlotVisualFromSlot(ki, old);
      lastHwSyncNormalizedSlots = prevSnapshot.map((s) => ({ ...s }));
      lastHwSyncChainSignature = chainLayoutSignature(lastHwSyncNormalizedSlots);
      if (selectedParamsKemplineSlotIndex === ki) {
        await loadAndShowModelsParamsForSlot(old, ki);
      }
    }
  } finally {
    slotModelUsbProbeInFlight = null;
  }
  // Cab 2 changé : refreshCabDualContextAfterProbe ne met à jour que l'en-tête, et le chemin
  // "patch valeurs seules" de loadAndShowModelsParamsForSlot ne touche que le cab1. On force donc
  // une reconstruction COMPLÈTE pour afficher les sliders du NOUVEAU cab2.
  // (probePickerCabDualCab2Hint = lastProbePickerAssignContext.cabDualCab2ModelId route le rebuild
  //  vers le bon cab2 ; sur erreur ce contexte est null -> pas de rebuild ici.)
  if (
    tab === 1 &&
    selectedParamsKemplineSlotIndex === ki &&
    lastProbePickerAssignContext?.ki === ki
  ) {
    selectedParamsInPlaceUpdater = null;   // invalide le chemin "patch valeurs seules"
    selectedParamsInPlaceSlotKey = null;
    selectedParamsValuesSig = null;
    const slotNow = lastHwSyncNormalizedSlots?.[ki] ?? optimisticSlot;
    await loadAndShowModelsParamsForSlot(slotNow, ki);
  }
}

async function applySlotModelFromPickerListClick(row: CatalogPickerModelRow): Promise<void> {
  if (row.ioSource) {
    await applyPath1InputSourceFromPicker(row);
    return;
  }
  if (row.splitSource) {
    await applyPath1SplitTypeFromPicker(row);
    return;
  }
  const catalogModelId = row.id;
  const displayName = row.name;
  const assignVariantFromRow = row.assignVariant;
  const ki = selectedParamsKemplineSlotIndex;
  if (ki === null || ki < 0 || ki > 15) {
    console.warn(
      "[SlotModelProbe] aucun slot grille sélectionné — cliquez d’abord un slot sur la matrice.",
    );
    return;
  }
  if (selectedParamsPresetIndex !== currentPresetIndex) {
    logCabDualTrace(
      `picker ignoré: preset UI=${selectedParamsPresetIndex} actif=${currentPresetIndex}`,
    );
    return;
  }
  logCabDualTrace(
    `picker click id=${catalogModelId} tab=${cabDualActiveTab} slot=${ki} ctx=${Boolean(lastCabDualTabPanesContext)} sync=${Boolean(cabDualPickerSync)} lock=${slotPickerIoLock?.kind ?? "none"}`,
  );
  if (isCabDualPickerApplyRoute(ki)) {
    const rowVariant = (row.assignVariant ?? "").trim().toLowerCase();
    const rowId = row.id.trim().toLowerCase();
    const dualId = (
      cabDualPickerSync?.dualCatalogModelId ??
      lastCabDualTabPanesContext?.dualCatalogModelId ??
      ""
    )
      .trim()
      .toLowerCase();
    const pickerCat = (slotPickerCategoryEl?.value ?? "").trim();

    if (cabDualActiveTab === 0 && rowVariant === "dual" && rowId === dualId) {
      return;
    }

    if (isCabDualSubCabPickerPick(row, pickerCat)) {
      await applyCabDualCabFromPickerListClick(row, ki);
      return;
    }

    logCabDualTrace(
      `picker sortie cab dual → assign slot entier id=${catalogModelId} cat=${pickerCat}`,
    );
    exitCabDualPickerModeForFullSlotReplace();
  }
  if (
    slotPickerIoLock?.kind === "amp_cab_cab" &&
    ampCabDualPickerSync &&
    ampCabDualActiveTab === 1
  ) {
    await applyAmpCabCabFromPickerListClick(row, ki);
    return;
  }
  const selectedKey = selectedParamsSlotKey ?? "";
  const selectedEmptyKey = `empty|${ki}`;
  const isExplicitEmptySelection = selectedKey === selectedEmptyKey;
  const occupied =
    !isExplicitEmptySelection &&
    selectedParamsSlotEl !== null &&
    !selectedParamsSlotEl.classList.contains("node-empty");
  const op = occupied ? "replace" : "add";
  const assignVariant =
    (assignVariantFromRow ?? "").trim().toLowerCase() ||
    usbAssignVariantFromPickerSub(
      slotPickerSubEl?.value ?? "",
      slotPickerCategoryEl?.value ?? "",
    );
  const categoryName = (slotPickerCategoryEl?.value ?? "").trim();
  if (
    normalizeCategory(categoryName) === "cab" &&
    assignVariant !== "dual" &&
    currentPresetIndex >= 0
  ) {
    clearCabDualPickerContext();
    clearSlotDualPartsSessionForKemplineSlot(currentPresetIndex, ki);
  }
  if (!categoryName) {
    console.warn("[SlotModelProbe] catégorie picker vide — impossible MAJ optimiste.");
    try {
      const out = await invoke<string>("probe_slot_model_usb", {
        op,
        slotIndex: ki,
        catalogModelId,
        assignVariant,
      });
      console.info("[SlotModelProbe]", op, displayName, catalogModelId, out);
    } catch (e) {
      console.warn("[SlotModelProbe]", e);
    }
    return;
  }

  const idTrim = catalogModelId.trim();
  const metaEarly = await getPresetMetaForId(idTrim);
  const moduleHexOpt =
    (await moduleHexForUsbVariant(idTrim, assignVariant, metaEarly))?.trim() || undefined;
  const optimisticSlot: SlotDebug = {
    category: categoryName,
    name: displayName.trim(),
    catalogModelId: idTrim,
    moduleHex: moduleHexOpt,
  };

  const prevSnapshot =
    lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16
      ? lastHwSyncNormalizedSlots.map((s) => ({ ...s }))
      : null;

  if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
    const next = lastHwSyncNormalizedSlots.map((s, i) =>
      i === ki ? { ...optimisticSlot } : { ...s },
    );
    lastHwSyncNormalizedSlots = next;
    lastHwSyncChainSignature = chainLayoutSignature(next);
  }

  patchMatrixSlotVisualFromSlot(ki, optimisticSlot);
  if (currentPresetIndex >= 0) {
    clearLiveChainOverridesForKemplineSlot(currentPresetIndex, ki);
  }
  markLiveWriteUiInteraction();
  slotModelUsbProbeInFlight = ki;
  if (assignVariant === "amp+cab" || assignVariant === "amp+cab-legacy") {
    lastProbePickerAssignContext = {
      ki,
      catalogModelId: idTrim,
      assignVariant,
      category: categoryName,
    };
    ampCabDualActiveTab = 0;
  } else if (assignVariant === "dual" && normalizeCategory(categoryName) === "cab") {
    lastProbePickerAssignContext = {
      ki,
      catalogModelId: idTrim,
      assignVariant,
      category: categoryName,
    };
    cabDualActiveTab = 0;
  } else {
    lastProbePickerAssignContext = null;
  }
  if (selectedParamsKemplineSlotIndex === ki) {
    selectParamsPaneByKemplineIndex(ki);
  }
  try {
    await loadAndShowModelsParamsFromCatalogDefaults(optimisticSlot, idTrim, ki, {
      assignVariant,
    });

    const out = await invoke<string>("probe_slot_model_usb", {
      op,
      slotIndex: ki,
      catalogModelId: idTrim,
      assignVariant,
    });
    console.info("[SlotModelProbe]", op, displayName, catalogModelId, out);
    mergeProbeSlotModelUntil = {
      ki,
      deadline: Date.now() + PROBE_SLOT_MERGE_GRACE_MS,
      mergeTraceEmitted: false,
    };
    suppressUsbPresetPollUntilMs = Date.now() + USB_PRESET_POLL_SUPPRESS_AFTER_PROBE_MS;
    lastSoftUsbPresetReadAt = Date.now();
    emitModelsSyncTrace(
      `slot_model_probe ok slot=${ki} — no pendingForceUsbPresetContent (pas de relecture preset complète)`,
    );
    const sessionVals = await resolveChainValuesForKemplineSlot(
      ki,
      optimisticSlot,
      idTrim,
      categoryName,
      pickSignal(metaEarly, moduleHexOpt),
    );
    if (sessionVals) {
      setSlotChainSessionValues(currentPresetIndex, ki, sessionVals);
      if (assignVariant === "amp+cab" || assignVariant === "amp+cab-legacy") {
        await loadAndShowModelsParamsFromCatalogDefaults(optimisticSlot, idTrim, ki, {
          assignVariant,
          ampChainValues: sessionVals,
        });
      }
    }
  } catch (e) {
    console.warn("[SlotModelProbe]", e);
    lastProbePickerAssignContext = null;
    if (prevSnapshot) {
      const old = prevSnapshot[ki]!;
      patchMatrixSlotVisualFromSlot(ki, old);
      lastHwSyncNormalizedSlots = prevSnapshot.map((s) => ({ ...s }));
      lastHwSyncChainSignature = chainLayoutSignature(lastHwSyncNormalizedSlots);
      if (selectedParamsKemplineSlotIndex === ki) {
        selectParamsPaneByKemplineIndex(ki);
      }
      await loadAndShowModelsParamsForSlot(old, ki);
    } else {
      scheduleLoadForPreset(currentPresetIndex, true);
    }
  } finally {
    slotModelUsbProbeInFlight = null;
  }
}

/**
 * Aligne les combos + liste sur le modèle catalogue courant (slot chargé).
 * Repli si catégorie / sous-clé absentes du jeu construit depuis le JSON.
 */
function ensureSlotPickerCategoryOption(category: string): void {
  if (!slotPickerCategoryEl) return;
  const cat = category.trim();
  if (!cat) return;
  for (const opt of slotPickerCategoryEl.options) {
    if (opt.value === cat) return;
  }
  const o = document.createElement("option");
  o.value = cat;
  o.textContent = cat;
  slotPickerCategoryEl.appendChild(o);
}

function applySlotPickerFromCatalogSelection(
  categoryName: string,
  subKey: string,
  highlightModelId: string | null,
): void {
  if (!slotPickerCategoryEl || !slotPickerSubEl || !catalogPickerDataCache) return;
  const cats = catalogPickerDataCache.categories;
  if (cats.length === 0) return;
  let cat = categoryName.trim();
  const lockCat =
    slotPickerIoLock?.kind === "io" || slotPickerIoLock?.kind === "routing"
      ? slotPickerIoLock.category
      : slotPickerIoLock?.kind === "amp_cab_cab"
        ? "Cab"
        : null;
  if (!cat || !cats.includes(cat)) {
    if (lockCat && lockCat === categoryName.trim()) {
      ensureSlotPickerCategoryOption(lockCat);
      cat = lockCat;
    } else {
      cat = cats[0] ?? "";
    }
  }
  slotPickerCategoryEl.value = cat;
  refillSlotPickerSubcategories();
  const subs = catalogPickerDataCache.subcategoriesByCategory.get(cat) ?? [];
  let sub = subKey;
  if (slotPickerIoLock?.kind === "amp_cab_cab") {
    sub = slotPickerIoLock.lockedSub;
    slotPickerSubEl.value = sub;
    slotPickerSubEl.disabled = true;
    if (slotPickerCategoryEl) slotPickerCategoryEl.disabled = true;
  } else if (!subs.includes(sub)) {
    sub = subs[0] ?? "";
    slotPickerSubEl.value = sub;
  } else {
    slotPickerSubEl.value = sub;
  }
  refillSlotPickerModelList(highlightModelId);
}

async function syncModelsSlotPickerFromLoadedModel(
  catalogModelId: string,
  meta: PresetMetaJson | null,
  moduleHex?: string,
  slotCategory?: string,
  /** Cab lié (trame HW / session) — legacy hybrid vs IR. */
  linkedCabHex?: string | null,
  /** Valeurs chaîne lues (Path 1 Input : surligner la source `@input` courante). */
  chainValues?: readonly (ChainParamValueJson | undefined)[] | null,
  /** Index param `@input` dans `chainValues` alignées (défaut 0). */
  inputParamChainIndex?: number,
  slotName?: string,
  hwSlotBus?: number | null,
): Promise<void> {
  if (!catalogPickerDataCache) return;
  const slotCat = (slotCategory ?? "").trim();
  const slotCatNorm = normalizeCategory(slotCat);
  const idTrim = catalogModelId.trim();
  const bus =
    hwSlotBus ??
    selectedSpecialHwSlotBus ??
    hwSlotBusFromSelectedParamsEl();

  if (bus === HW_SLOT_BUS_SPLIT) {
    await syncSplitPickerHighlightAsync(idTrim, moduleHex);
    return;
  }
  if (bus === HW_SLOT_BUS_MERGE) {
    applySlotPickerRoutingLock("Merge", idTrim || "HD2_AppDSPFlowJoin");
    return;
  }
  if (bus === HW_SLOT_BUS_INPUT && idTrim) {
    syncInputPickerHighlight(idTrim, chainValues, inputParamChainIndex ?? 0);
    return;
  }
  if (bus === HW_SLOT_BUS_OUTPUT && idTrim) {
    applySlotPickerIoLock("Output", idTrim);
    return;
  }

  const routingCat = resolveRoutingPickerCategory(slotCat, slotName ?? "", idTrim, meta);
  if (routingCat === "Split") {
    await syncSplitPickerHighlightAsync(idTrim, moduleHex);
    return;
  }
  if (routingCat === "Merge") {
    applySlotPickerRoutingLock("Merge", idTrim || "HD2_AppDSPFlowJoin");
    return;
  }
  if (slotCatNorm === "input" && idTrim) {
    syncInputPickerHighlight(idTrim, chainValues, inputParamChainIndex ?? 0);
    return;
  }
  if (slotCatNorm === "output" && idTrim) {
    applySlotPickerIoLock("Output", idTrim);
    return;
  }
  if (selectedSpecialHwSlotBus !== null && pickerCategoryForHwSlotBus(selectedSpecialHwSlotBus)) {
    return;
  }
  if (
    ampCabDualPickerSync &&
    ampCabDualActiveTab === 1 &&
    isAmpCabFamilySlotCategory(slotCategory)
  ) {
    const legacy = await isAmpCabSlotLegacy(
      ampCabDualPickerSync.slotCategory,
      ampCabDualPickerSync.linkedCabHex,
      ampCabDualPickerSync.cabCatalogModelId,
    );
    const lockedSub = resolveCabDualCab2PickerSub(
      ampCabDualPickerSync.cabCatalogModelId,
      ampCabDualPickerSync.slotCategory,
      legacy,
    );
    applySlotPickerAmpCabCabLock(ampCabDualPickerSync.cabCatalogModelId, lockedSub);
    return;
  }
  if (
    (cabDualPickerSync || lastCabDualTabPanesContext) &&
    normalizeCategory(slotCategory ?? "") === "cab" &&
    cabDualActiveTab === 1
  ) {
    await ensureCabDualPickerSynced(1);
    return;
  }
  clearSlotPickerIoLock();
  const pickerKi = selectedParamsKemplineSlotIndex;
  const probePickerCtx =
    pickerKi !== null &&
    lastProbePickerAssignContext &&
    lastProbePickerAssignContext.ki === pickerKi
      ? lastProbePickerAssignContext
      : null;
  const assignVariant =
    probePickerCtx?.assignVariant?.trim().toLowerCase() ||
    (await usbAssignVariantForAmpCabSlot(
      meta,
      moduleHex,
      slotCategory,
      catalogModelId,
      linkedCabHex,
    ));
  const preferPickerCategory = probePickerCtx?.category?.trim() || slotCategory;
  const loc = findUsbAssignPickerLocation(
    catalogPickerDataCache,
    catalogModelId,
    assignVariant,
    preferPickerCategory,
  );
  if (!loc) {
    const prefer = (slotCategory ?? "").trim();
    // Split / merge / connected devices : hors picker FX.
    if (prefer && !catalogPickerDataCache.categories.includes(prefer)) return;
    const catFallback =
      prefer && catalogPickerDataCache.categories.includes(prefer)
        ? prefer
        : (catalogPickerDataCache.categories[0] ?? "");
    const subFallback =
      catalogPickerDataCache.subcategoriesByCategory.get(catFallback)?.[0] ?? "";
    applySlotPickerFromCatalogSelection(catFallback, subFallback, catalogModelId.trim() || null);
    return;
  }
  const cat = loc.category;
  const subKey = loc.subKey;
  applySlotPickerFromCatalogSelection(cat, subKey, catalogModelId);
}

async function mountModelsSlotPicker(): Promise<void> {
  if (slotPickerMountPromise) return slotPickerMountPromise;
  const root = document.getElementById("models-params-slot-picker");
  if (!root) return;
  slotPickerMountPromise = (async () => {
    if (slotPickerCategoryEl) return;
    try {
      catalogPickerDataCache = await getUsbAssignPickerData();
    } catch (e) {
      console.warn("[models] getUsbAssignPickerData", e);
      catalogPickerDataCache = {
        categories: [],
        subcategoriesByCategory: new Map(),
        modelsByCategoryAndSub: new Map(),
      };
    }
    root.replaceChildren();

    const catSel = document.createElement("select");
    catSel.id = "models-slot-picker-category";
    catSel.className = "models-params-slot-picker-select";
    catSel.setAttribute("aria-label", "Catégorie");
    const catEmpty = document.createElement("option");
    catEmpty.value = "";
    catEmpty.textContent = "—";
    catSel.appendChild(catEmpty);
    for (const c of catalogPickerDataCache.categories) {
      const o = document.createElement("option");
      o.value = c;
      o.textContent = c;
      catSel.appendChild(o);
    }
    root.appendChild(catSel);
    slotPickerCategoryEl = catSel;

    const subSel = document.createElement("select");
    subSel.id = "models-slot-picker-subcategory";
    subSel.className = "models-params-slot-picker-select";
    subSel.setAttribute("aria-label", "Sous-catégorie");
    subSel.disabled = true;
    root.appendChild(subSel);
    slotPickerSubEl = subSel;

    const listWrap = document.createElement("div");
    listWrap.className = "models-params-slot-picker-list-wrap";
    const ul = document.createElement("ul");
    ul.className = "models-slot-picker-model-list";
    ul.id = "models-slot-picker-model-list";
    ul.setAttribute("aria-label", "Modèles");
    listWrap.appendChild(ul);
    root.appendChild(listWrap);
    slotPickerListEl = ul;

    catSel.addEventListener("change", () => {
      if (slotPickerIoLock?.kind === "amp_cab_cab") return;
      refillSlotPickerSubcategories();
      refillSlotPickerModelList(null);
    });
    subSel.addEventListener("change", () => {
      if (slotPickerIoLock?.kind === "amp_cab_cab") {
        subSel.value = slotPickerIoLock.lockedSub;
        return;
      }
      refillSlotPickerModelList(null);
    });

    if (selectedSpecialHwSlotBus !== null && pickerCategoryForHwSlotBus(selectedSpecialHwSlotBus)) {
      lockPickerCategoryFromHwSlotBus(selectedSpecialHwSlotBus);
    } else {
      resetSlotPickerToIdle();
    }
  })();
  return slotPickerMountPromise;
}

function getModelsParamsSubheadEl(): HTMLElement | null {
  return document.getElementById("models-params-pane-subhead");
}

function getModelsParamsBasedOnEl(): HTMLElement | null {
  return document.getElementById("models-params-pane-basedon");
}

function getModelsParamsModelIconWrapEl(): HTMLElement | null {
  return document.getElementById("models-params-pane-model-icon-wrap");
}

/** Nom modèle provisoire (scroll HW) dans l’en-tête du panneau. */
function setModelsParamsPaneModelNamePreview(usbName: string): void {
  const subhead = getModelsParamsSubheadEl();
  if (!subhead) return;
  subhead.replaceChildren();
  const label = usbName.trim();
  const title = document.createElement("h2");
  title.className = "models-params-model-title";
  title.textContent = label ? `${label} …` : "Modèle…";
  subhead.appendChild(title);
}

/** Vide le sous-titre modèle et l’icône sous le bandeau titre. */
function clearModelsParamsSubheadAndIcon(): void {
  getModelsParamsSubheadEl()?.replaceChildren();
  const basedOn = getModelsParamsBasedOnEl();
  if (basedOn) {
    basedOn.textContent = "";
    basedOn.title = "";
  }
  disposeModelsParamsIconLivePreview();
  hideModelsParamsIconPreviewPopover();
  getModelsParamsModelIconWrapEl()?.replaceChildren();
}

let selectedParamsSlotEl: HTMLElement | null = null;
let selectedParamsSlotKey: string | null = null;
let selectedParamsPresetIndex = -1;
let selectedParamsKemplineSlotIndex: number | null = null;
let selectedParamsValuesSig: string | null = null;
// Callback de patch in-place pour le panneau courant (même modèle, valeurs différentes).
let selectedParamsInPlaceUpdater: ((rawChainValues: ChainParamValueJson[] | null) => void) | null = null;
let selectedParamsInPlaceSlotKey: string | null = null;
/** Contexte pour mapper l’index wire `PP` → `symbolicID` (panneau params ouvert). */
let selectedParamsHwWireContext: {
  paramsForDisplay: ModelParamDefJson[];
  catalogSignal: string | null | undefined;
  /** Décalage wire Amp+Cab / Cab dual (onglet Cab 2) : index catalogue + base → PP USB. */
  wireParamIndexBase?: number;
} | null = null;
/** Id catalogue du dernier rendu par clé slot — détecte un changement de modèle au même index sans tout reconstruire inutilement. */
const paramsPaneCatalogBySlotKey = new Map<string, string>();

/** Index grille Kempline pour la clé de sélection (rejette non-entiers). */
function kemplineIndexForSlotKey(kemplineSlotIndex?: number): number | undefined {
  if (kemplineSlotIndex === undefined) return undefined;
  const n = Number(kemplineSlotIndex);
  if (!Number.isFinite(n)) return undefined;
  const t = Math.trunc(n);
  if (Math.abs(n - t) > 1e-9) return undefined;
  return t;
}

/**
 * Identité slot pour le panneau paramètres / sync in-place.
 * Avec index Kempline (grille 0–15) : on n’inclut pas `name` ni `moduleHex` — le parseur peut les
 * stabiliser après coup et provoquait des rechargements complets + `chainValues` encore `null` (UI sans sliders).
 */
function makeSlotSelectionKey(slot: SlotDebug, kemplineSlotIndex?: number): string {
  const cat = normalizeCategory(slot.category);
  const ki = kemplineIndexForSlotKey(kemplineSlotIndex);
  if (ki !== undefined) {
    return `k|${ki}|${cat}`;
  }
  return [
    cat,
    slot.name.trim().toLowerCase(),
    (slot.moduleHex ?? "").trim().toLowerCase(),
    (slot.slotTypeHex ?? "").trim().toLowerCase(),
    "",
  ].join("|");
}

function clearSelectedParamsContext(): void {
  selectedParamsSlotKey = null;
  selectedParamsPresetIndex = -1;
  selectedParamsKemplineSlotIndex = null;
  selectedParamsValuesSig = null;
  selectedParamsInPlaceUpdater = null;
  selectedParamsInPlaceSlotKey = null;
  selectedParamsHwWireContext = null;
  paramsPaneCatalogBySlotKey.clear();
  clearAllLiveChainParamOverrides();
  pendingHardwareSelectedKemplineSlotIndex = null;
  pendingHardwareSelectedSlotBus = null;
  if (autoSelectFallbackTimer !== null) {
    window.clearTimeout(autoSelectFallbackTimer);
    autoSelectFallbackTimer = null;
  }
}

function hasSelectedParamsContextForCurrentPreset(): boolean {
  return (
    selectedParamsPresetIndex === currentPresetIndex &&
    (selectedParamsKemplineSlotIndex !== null || !!selectedParamsSlotKey)
  );
}

function chainValuesSignature(values: ChainParamValueJson[] | null): string {
  if (!values) return "null";
  return JSON.stringify(values);
}

function clearSlotSelectionVisual() {
  if (selectedParamsSlotEl) {
    selectedParamsSlotEl
      .querySelectorAll(".models-slot-remove-btn")
      .forEach((n) => n.remove());
    selectedParamsSlotEl.classList.remove("node--selected");
    selectedParamsSlotEl = null;
  }
}

function attachSelectedSlotRemoveButton(el: HTMLElement, slotIndex: number): void {
  if (!Number.isInteger(slotIndex) || slotIndex < 0 || slotIndex > 15) return;
  el.querySelectorAll(".models-slot-remove-btn").forEach((n) => n.remove());
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "models-slot-remove-btn";
  btn.textContent = "×";
  btn.title = "Supprimer le modèle du slot";
  btn.setAttribute("aria-label", "Supprimer le modèle du slot");
  btn.addEventListener("click", (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    if (isMatrixUsbInteractionLocked()) return;
    void removeMatrixSlotFromCell(slotIndex);
  });
  el.appendChild(btn);
}

/** Vide le slot sur le HX (`probe_slot_model_usb` remove) + MAJ optimiste grille. */
async function removeMatrixSlotFromCell(
  slotIndex: number,
  options?: { reselect?: boolean; skipInteractionLock?: boolean },
): Promise<boolean> {
  const run = async (): Promise<boolean> => {
  if (!Number.isInteger(slotIndex) || slotIndex < 0 || slotIndex > 15) return false;
  const emptySlot: SlotDebug = { category: "", name: "<empty>" };
  slotModelUsbProbeInFlight = slotIndex;
  try {
    // Le remove doit cibler le slot source : focus USB explicite (après un add le HX est
    // souvent sur le slot destination — sans switch, le bulk remove peut être ignoré).
    await waitUntilHardwareSyncIdle(15_000);
    await enqueueHardwareSlotSwitch(slotIndex);
    await delayMs(100);
    const out = await invoke<string>("probe_slot_model_usb", {
      op: "remove",
      slotIndex,
    });
    console.info("[SlotModelProbe]", "remove", `slot=${slotIndex}`, out);
    selectedParamsValuesSig = null;
    if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
      const next = lastHwSyncNormalizedSlots.map((s, i) =>
        i === slotIndex ? { ...emptySlot } : { ...s },
      );
      lastHwSyncNormalizedSlots = next;
      lastHwSyncChainSignature = chainLayoutSignature(next);
      patchMatrixSlotVisualFromSlot(slotIndex, emptySlot);
      mergeProbeSlotModelUntil = {
        ki: slotIndex,
        deadline: Date.now() + PROBE_SLOT_MERGE_GRACE_MS,
        mergeTraceEmitted: false,
      };
      suppressUsbPresetPollUntilMs = Date.now() + USB_PRESET_POLL_SUPPRESS_AFTER_PROBE_MS;
      lastSoftUsbPresetReadAt = Date.now();
      emitModelsSyncTrace(
        `slot_model_probe remove ok slot=${slotIndex} — optimistic empty + merge grace (preset_data inchangé côté Rust)`,
      );
    }
    markLiveWriteUiInteraction();
    if (currentPresetIndex >= 0) {
      clearLiveChainOverridesForKemplineSlot(currentPresetIndex, slotIndex);
    }
    releaseCabPickerLockFromDualSlots();
    if (options?.reselect !== false && selectedParamsKemplineSlotIndex === slotIndex) {
      suppressNextUiSlotHardwareSwitch = true;
      selectParamsPaneByKemplineIndex(slotIndex);
    }
    return true;
  } catch (e) {
    console.warn("[SlotModelProbe][remove]", e);
    return false;
  } finally {
    slotModelUsbProbeInFlight = null;
  }
  };
  if (options?.skipInteractionLock) return run();
  return withMatrixUsbInteractionLock(`Suppression slot ${slotIndex + 1}…`, run);
}

function tryRestoreSelectedParamsPaneAfterRender(): boolean {
  if (selectedParamsPresetIndex !== currentPresetIndex) return false;
  if (selectedParamsKemplineSlotIndex !== null) {
    const byIndex = contentEl.querySelector<HTMLElement>(
      `[data-kempline-slot-index="${selectedParamsKemplineSlotIndex}"]`,
    );
    if (byIndex) {
      suppressNextUiSlotHardwareSwitch = true;
      byIndex.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      return true;
    }
  }
  if (!selectedParamsSlotKey) return false;
  const nodes = contentEl.querySelectorAll<HTMLElement>("[data-slot-selection-key]");
  for (const node of nodes) {
    if (node.dataset.slotSelectionKey !== selectedParamsSlotKey) continue;
    suppressNextUiSlotHardwareSwitch = true;
    node.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    return true;
  }
  return false;
}

function selectParamsPaneByKemplineIndex(kemplineSlotIndex: number): boolean {
  if (!Number.isInteger(kemplineSlotIndex) || kemplineSlotIndex < 0 || kemplineSlotIndex > 15) {
    return false;
  }
  const node = contentEl.querySelector<HTMLElement>(
    `.node--params-clickable[data-kempline-slot-index="${kemplineSlotIndex}"]`,
  );
  if (!node) return false;
  suppressNextUiSlotHardwareSwitch = true;
  node.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  return true;
}

/** Focus UI + panneau params sur un slot (sans commande HW — déjà positionné). */
async function focusMatrixSlotParamsPane(kemplineSlotIndex: number): Promise<void> {
  if (!Number.isInteger(kemplineSlotIndex) || kemplineSlotIndex < 0 || kemplineSlotIndex > 15) {
    return;
  }
  suppressNextUiSlotHardwareSwitch = true;
  lastUserHwSlotSwitchIndex = kemplineSlotIndex;
  lastUserHwSlotSwitchAt = Date.now();
  selectParamsPaneByKemplineIndex(kemplineSlotIndex);
  const slot = lastHwSyncNormalizedSlots?.[kemplineSlotIndex];
  if (!slot || slot.name === "<empty>") return;

  selectedParamsInPlaceUpdater = null;
  selectedParamsInPlaceSlotKey = null;
  selectedParamsValuesSig = null;

  const cb = matrixSlotClipboard;
  if (
    cb &&
    cb.presetIndex === currentPresetIndex &&
    cb.sourceKemplineIndex === kemplineSlotIndex &&
    cb.chainParamsBySymbolicId.size > 0
  ) {
    const hint = await buildChainValuesFromMatrixClipboard(cb, slot);
    if (hint) setSlotChainSessionValues(currentPresetIndex, kemplineSlotIndex, hint);
  }

  await loadAndShowModelsParamsForSlot(slot, kemplineSlotIndex);

  if (selectedParamsKemplineSlotIndex === kemplineSlotIndex) {
    const idTrim = (slot.catalogModelId ?? "").trim();
    if (idTrim) {
      const meta = await getPresetMetaForId(idTrim);
      const resolved = await resolveChainValuesForKemplineSlot(
        kemplineSlotIndex,
        slot,
        idTrim,
        meta?.categoryName ?? null,
        pickSignal(meta, slot.moduleHex),
      );
      if (resolved) {
        selectedParamsValuesSig = `${currentPresetIndex}|${kemplineSlotIndex}|${chainValuesSignature(resolved)}`;
      }
    }
  }
}

function selectParamsPaneByHwSlotBus(slotBus: number): boolean {
  const node = contentEl.querySelector<HTMLElement>(`[data-hw-slot-bus="${slotBus}"]`);
  if (!node) return false;
  suppressNextUiSlotHardwareSwitch = true;
  node.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  return true;
}

function consumePendingHardwareSlotSelection(): void {
  if (pendingHardwareSelectedKemplineSlotIndex !== null) {
    const idx = pendingHardwareSelectedKemplineSlotIndex;
    if (selectedParamsKemplineSlotIndex === idx) {
      hwSlotDebugLog(`selection déjà active idx=${idx}`);
      pendingHardwareSelectedKemplineSlotIndex = null;
      pendingHardwareSelectedSlotBus = null;
      return;
    }
    if (selectParamsPaneByKemplineIndex(idx)) {
      hwSlotDebugLog(`selection appliquée idx=${idx}`);
      pendingHardwareSelectedKemplineSlotIndex = null;
      pendingHardwareSelectedSlotBus = null;
      return;
    }
    hwSlotDebugLog(`node introuvable pour idx=${idx} (nouvel essai prochain cycle)`);
    return;
  }
  if (pendingHardwareSelectedSlotBus !== null) {
    const bus = pendingHardwareSelectedSlotBus;
    if (selectParamsPaneByHwSlotBus(bus)) {
      hwSlotDebugLog(`selection appliquée slot_bus=${bus}`);
      pendingHardwareSelectedSlotBus = null;
      return;
    }
    hwSlotDebugLog(`node introuvable pour slot_bus=${bus} (nouvel essai prochain cycle)`);
  }
}

function tryAutoSelectFallbackParamsPaneAfterRender(): boolean {
  if (selectedParamsPresetIndex === currentPresetIndex) return false;
  if (pendingHardwareSelectedKemplineSlotIndex !== null) return false;
  if (pendingHardwareSelectedSlotBus !== null) return false;
  const nodes = contentEl.querySelectorAll<HTMLElement>(
    '.node--params-clickable[data-kempline-slot-index]:not(.node--empty)',
  );
  if (nodes.length === 0) return false;
  const first = nodes[0];
  suppressNextUiSlotHardwareSwitch = true;
  first.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  return true;
}

function armAutoSelectFallbackParamsPaneAfterRender(): void {
  if (autoSelectFallbackTimer !== null) return;
  autoSelectFallbackTimer = window.setTimeout(() => {
    autoSelectFallbackTimer = null;
    if (selectedParamsPresetIndex === currentPresetIndex) return;
    if (pendingHardwareSelectedKemplineSlotIndex !== null) return;
    if (pendingHardwareSelectedSlotBus !== null) return;
    tryAutoSelectFallbackParamsPaneAfterRender();
  }, 240);
}

function resetModelsParamsIdleHint() {
  clearModelsParamsSubheadAndIcon();
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  resetSlotPickerToIdle();
  const p = document.createElement("p");
  p.className = "models-params-placeholder";
  p.textContent = MODELS_PARAMS_IDLE_PLACEHOLDER;
  inner.appendChild(p);
}

/** Panneau Paramètres Models : aucun contenu (ex. clic sur un slot vide). */
function clearModelsParamsPaneContent() {
  clearModelsParamsSubheadAndIcon();
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  releaseCabPickerLockFromDualSlots();
  // Ne pas resetSlotPickerToIdle : garder catégorie picker pour assigner sur ce slot vide FX.
}

// --- Copier / coller matrice (snapshot RAM → add + replay params) ---

type MatrixSlotClipboard = {
  presetIndex: number;
  /** Libellé preset au moment de la copie (affichage inter-preset). */
  sourcePresetLabel: string;
  sourceKemplineIndex: number;
  path: 0 | 1;
  catalogModelId: string;
  displayName: string;
  categoryName: string;
  assignVariant: string;
  moduleHex?: string;
  catalogSignal: string | null;
  chainParamsBySymbolicId: Map<string, ChainParamValueJson>;
  /** Cab dual : 2ᵉ cab (id catalogue + variante assign). */
  cabDualCab2ModelId?: string | null;
  cabDualCab2AssignVariant?: string | null;
  /** Amp+Cab : cab lié (id catalogue + variante assign). */
  ampCabCabModelId?: string | null;
  ampCabCabAssignVariant?: string | null;
};

/** Délai entre purge source et coller destination (move) — laisser digérer le remove USB. */
const MATRIX_USB_OP_SETTLE_MS = 150;
/** Pause avant `request_preset_content` après probe matrice (évite collision avec dump). */
const MATRIX_USB_BEFORE_PRESET_LOAD_MS = 400;

async function waitForMatrixUsbIdle(maxWaitMs = 30_000): Promise<void> {
  const deadline = Date.now() + maxWaitMs;
  while (isMatrixUsbInteractionLocked()) {
    if (Date.now() >= deadline) {
      emitModelsSyncTrace("waitMatrixUsbIdle TIMEOUT");
      return;
    }
    await delayMs(40);
  }
}

async function settleUsbAfterMatrixProbe(): Promise<void> {
  const sinceProbe = Date.now() - lastSoftUsbPresetReadAt;
  if (sinceProbe < MATRIX_USB_BEFORE_PRESET_LOAD_MS) {
    await delayMs(MATRIX_USB_BEFORE_PRESET_LOAD_MS - sinceProbe);
  }
}

let matrixUsbInteractionLockDepth = 0;
/** Overlay « sablier » pendant lecture preset (USB + rendu grille). */
let presetLoadUiLockDepth = 0;

function isPresetLoadUiLocked(): boolean {
  return presetLoadUiLockDepth > 0;
}

function isModelsContentBusy(): boolean {
  return loading || isMatrixUsbInteractionLocked() || isPresetLoadUiLocked();
}

function pushPresetLoadUiLock(): void {
  if (presetLoadUiLockDepth === 0) {
    hideMatrixContextMenu();
    document.body.classList.add("models-preset-load-busy");
  }
  presetLoadUiLockDepth += 1;
}

function popPresetLoadUiLock(): void {
  presetLoadUiLockDepth = Math.max(0, presetLoadUiLockDepth - 1);
  if (presetLoadUiLockDepth === 0) {
    document.body.classList.remove("models-preset-load-busy");
  }
}

function isMatrixUsbInteractionLocked(): boolean {
  return matrixUsbInteractionLockDepth > 0;
}

function pushMatrixUsbInteractionLock(statusHint?: string): void {
  if (matrixUsbInteractionLockDepth === 0) {
    hideMatrixContextMenu();
    document.body.classList.add("models-matrix-usb-busy");
    if (statusHint) setStatus(statusHint);
  }
  matrixUsbInteractionLockDepth += 1;
}

function popMatrixUsbInteractionLock(): void {
  matrixUsbInteractionLockDepth = Math.max(0, matrixUsbInteractionLockDepth - 1);
  if (matrixUsbInteractionLockDepth === 0) {
    document.body.classList.remove("models-matrix-usb-busy");
  }
}

async function withMatrixUsbInteractionLock<T>(
  statusHint: string,
  fn: () => Promise<T>,
): Promise<T> {
  pushMatrixUsbInteractionLock(statusHint);
  try {
    return await fn();
  } finally {
    popMatrixUsbInteractionLock();
  }
}

let matrixSlotClipboard: MatrixSlotClipboard | null = null;
let matrixCtxTargetKemplineIndex: number | null = null;
/** Index source pendant un drag & drop matrice (move). */
let matrixDragSourceKi: number | null = null;

function clearMatrixSlotClipboard(): void {
  matrixSlotClipboard = null;
}

function matrixSlotPath(kemplineSlotIndex: number): 0 | 1 {
  return kemplineSlotIndex >= 8 ? 1 : 0;
}

function isStructuralMatrixCategory(category: string): boolean {
  const nk = normalizeCategory(category);
  return (
    nk === "input" ||
    nk === "output" ||
    nk === "split" ||
    nk === "merge" ||
    nk === "routing"
  );
}

function canCopyMatrixSlot(slot: SlotDebug): boolean {
  if (isEmptyGridCell(slot)) return false;
  if (isStructuralMatrixCategory(slot.category)) return false;
  if ((slot.catalogModelId ?? "").trim().length > 0) return true;
  // Grille preset : category + name + moduleHex (pas toujours catalogModelId).
  return (slot.moduleHex ?? "").trim().length > 0;
}

function canPasteMatrixSlotToEmpty(kemplineSlotIndex: number, slots: SlotDebug[]): boolean {
  const cb = matrixSlotClipboard;
  if (!cb || currentPresetIndex < 0) return false;
  const slot = slots[kemplineSlotIndex];
  if (!slot || !isEmptyGridCell(slot)) return false;
  return !isColumnPairedSlotBlocked(slots, kemplineSlotIndex);
}

/** Même règles que coller, sans clipboard (validation drag & drop move). */
function canMoveMatrixSlotToEmpty(
  sourceKi: number,
  destKi: number,
  slots: SlotDebug[],
): boolean {
  if (sourceKi === destKi) return false;
  if (currentPresetIndex < 0 || isModelsContentBusy()) return false;
  const sourceSlot = slots[sourceKi];
  if (!sourceSlot || !canCopyMatrixSlot(sourceSlot)) return false;
  if (matrixSlotPath(sourceKi) !== matrixSlotPath(destKi)) return false;
  const destSlot = slots[destKi];
  if (!destSlot || !isEmptyGridCell(destSlot)) return false;
  return !isColumnPairedSlotBlocked(slots, destKi);
}

function hideMatrixContextMenu(): void {
  const menu = document.getElementById("models-ctx-menu");
  menu?.classList.remove("visible");
  matrixCtxTargetKemplineIndex = null;
}

async function buildChainParamsMapForCopy(
  slot: SlotDebug,
  kemplineSlotIndex: number,
  catalogModelId: string,
  categoryName: string,
  catalogSignal: string | null,
  rawChain: ChainParamValueJson[] | null,
): Promise<Map<string, ChainParamValueJson>> {
  const out = new Map<string, ChainParamValueJson>();
  if (!rawChain || rawChain.length === 0) return out;
  const found = await findModelDefinitionForSlot(slot, catalogModelId, categoryName);
  const params = found?.entry.params ?? [];
  if (params.length === 0) return out;
  const aligned = alignChainValuesToModelParamOrder(
    rawChain,
    params,
    params,
    catalogSignal,
  );
  const merged =
    mergeLiveChainOverridesIntoAligned(
      currentPresetIndex,
      kemplineSlotIndex,
      params,
      aligned,
    ) ?? aligned;
  if (!merged) return out;
  for (let i = 0; i < params.length; i += 1) {
    const sid = (params[i]?.symbolicID ?? "").trim();
    const v = merged[i];
    if (sid && v !== undefined) out.set(sid, v);
  }
  return out;
}

function modelParamSourceOrderIds(
  allModelParams: ModelParamDefJson[],
  catalogSignal: string | null | undefined,
  valueCountHint?: number,
): string[] {
  const signal = normalizeCatalogSignal(catalogSignal);
  const buildSourceOrderIdsFromModels = (includeStereoOnly: boolean): string[] => {
    const out: string[] = [];
    for (const p of allModelParams) {
      if (!includeStereoOnly && p["stereo-only"] === true) continue;
      const sid = (p.symbolicID ?? "").trim();
      if (!sid) continue;
      out.push(sid);
    }
    return out;
  };
  const sourceAll = buildSourceOrderIdsFromModels(true);
  if (signal === "mono" && valueCountHint !== undefined) {
    const sourceMono = buildSourceOrderIdsFromModels(false);
    const diffAll = Math.abs(sourceAll.length - valueCountHint);
    const diffMono = Math.abs(sourceMono.length - valueCountHint);
    if (diffMono < diffAll) return sourceMono;
  }
  return sourceAll;
}

function chainValuesUsbOrderFromSymbolicMap(
  chainParamsBySymbolicId: Map<string, ChainParamValueJson>,
  allModelParams: ModelParamDefJson[],
  catalogSignal: string | null | undefined,
): ChainParamValueJson[] {
  const source = modelParamSourceOrderIds(
    allModelParams,
    catalogSignal,
    chainParamsBySymbolicId.size,
  );
  const out: ChainParamValueJson[] = [];
  for (const sid of source) {
    const v = chainParamsBySymbolicId.get(sid);
    if (v !== undefined) out.push(v);
  }
  return out;
}

async function buildChainValuesFromMatrixClipboard(
  cb: MatrixSlotClipboard,
  slot: SlotDebug,
): Promise<ChainParamValueJson[] | null> {
  const found = await findModelDefinitionForSlot(slot, cb.catalogModelId, cb.categoryName);
  const params = found?.entry.params ?? [];
  if (params.length === 0 || cb.chainParamsBySymbolicId.size === 0) return null;
  const values = chainValuesUsbOrderFromSymbolicMap(
    cb.chainParamsBySymbolicId,
    params,
    cb.catalogSignal,
  );
  return values.length > 0 ? values : null;
}

async function resolveMatrixClipboardAssignContext(
  slot: SlotDebug,
  kemplineSlotIndex: number,
  idTrim: string,
  meta: PresetMetaJson | null,
): Promise<{ assignVariant: string; categoryName: string }> {
  const slotCat = (slot.category ?? "").trim();
  const metaCat = (meta?.categoryName ?? "").trim();

  if (loadedPresetIndex === currentPresetIndex) {
    try {
      const dualParts = await resolveSlotDualParts(kemplineSlotIndex, {
        slot,
        catalogModelId: idTrim,
      });
      if (dualParts?.kind === "amp_cab" && dualParts.parts.length === 2) {
        const cabHex = dualParts.parts[1]?.chainHex?.trim() || null;
        const assignVariant = await usbAssignVariantForAmpCabSlot(
          meta,
          slot.moduleHex,
          "Amp+Cab",
          idTrim,
          cabHex,
        );
        const categoryName = isAmpCabLegacySlotCategory(slotCat)
          ? slotCat
          : assignVariant === "amp+cab-legacy"
            ? "Amp+Cab Legacy"
            : isAmpCabFamilySlotCategory(slotCat)
              ? slotCat
              : "Amp+Cab";
        return { assignVariant, categoryName };
      }
      if (dualParts?.kind === "cab_dual" && dualParts.parts.length === 2) {
        return {
          assignVariant: "dual",
          categoryName: slotCat || "Cab",
        };
      }
    } catch {
      /* repli ci-dessous */
    }
  }

  if (slotWantsCabDualTabs(slot, null, meta)) {
    return { assignVariant: "dual", categoryName: slotCat || "Cab" };
  }

  if (slotWantsAmpCabDualTabs(slot, null) || isAmpCabFamilySlotCategory(slotCat)) {
    const cabHex: string | null = linkedCabHexFromSlot(slot) || null;
    const assignVariant = await usbAssignVariantForAmpCabSlot(
      meta,
      slot.moduleHex,
      slotCat || "Amp+Cab",
      idTrim,
      cabHex,
    );
    const categoryName = isAmpCabFamilySlotCategory(slotCat)
      ? slotCat
      : assignVariant === "amp+cab-legacy"
        ? "Amp+Cab Legacy"
        : assignVariant === "amp+cab"
          ? "Amp+Cab"
          : slotCat || metaCat;
    return { assignVariant, categoryName };
  }

  return {
    assignVariant: usbAssignVariantFromPresetMeta(meta, slot.moduleHex, slotCat),
    categoryName: slotCat || metaCat,
  };
}

async function resolveAmpCabClipboardCab(
  kemplineSlotIndex: number,
  ampCatalogModelId: string,
  assignVariant: string,
): Promise<{ modelId: string; assignVariant: string } | null> {
  const ampNorm = ampCatalogModelId.trim().toLowerCase();
  let fromParts: string | null = null;
  if (loadedPresetIndex === currentPresetIndex) {
    try {
      const dualParts = await resolveSlotDualParts(kemplineSlotIndex, {
        catalogModelId: ampCatalogModelId,
        kind: "amp_cab",
        assignVariant,
      });
      if (dualParts?.kind === "amp_cab" && dualParts.parts.length === 2) {
        fromParts = dualParts.parts[1]?.modelId?.trim() || null;
      }
    } catch {
      /* repli sync UI */
    }
  }
  const pair = await ampCabHexPairFromAssignVariant(ampCatalogModelId, assignVariant);
  const defaultCab = pair
    ? (await getCatalogModelIdForHex(pair.cabHex, "Cab"))?.trim() ?? ""
    : "";
  const pickNonDefault = (id: string | null | undefined): string | null => {
    const t = (id ?? "").trim();
    if (!t) return null;
    if (defaultCab && t.toLowerCase() === defaultCab.toLowerCase()) return null;
    return t;
  };
  let modelId = pickNonDefault(fromParts);
  if (
    !modelId &&
    lastProbePickerAssignContext &&
    lastProbePickerAssignContext.ki === kemplineSlotIndex &&
    lastProbePickerAssignContext.catalogModelId.trim().toLowerCase() === ampNorm
  ) {
    modelId = pickNonDefault(lastProbePickerAssignContext.ampCabCabModelId);
  }
  if (
    !modelId &&
    ampCabDualPickerSync &&
    ampCabDualPickerSync.ampCatalogModelId.trim().toLowerCase() === ampNorm &&
    selectedParamsKemplineSlotIndex === kemplineSlotIndex
  ) {
    modelId = pickNonDefault(ampCabDualPickerSync.cabCatalogModelId);
  }
  if (!modelId) return null;
  const slotCat =
    ampCabDualPickerSync?.slotCategory ??
    (await getPresetMetaForId(ampCatalogModelId))?.categoryName ??
    "Amp+Cab";
  const loc = catalogPickerDataCache
    ? findUsbAssignPickerLocation(catalogPickerDataCache, modelId, "single", slotCat)
    : null;
  return {
    modelId,
    assignVariant: loc ? usbAssignVariantFromPickerSub(loc.subKey, "Cab") : "single",
  };
}

async function pasteAmpCabCabIfNeeded(
  destKi: number,
  ampCatalogModelId: string,
  ampAssignVariant: string,
  cabModelId: string,
  cabAssignVariant: string,
): Promise<void> {
  const cabTrim = cabModelId.trim();
  if (!cabTrim) return;
  const pair = await ampCabHexPairFromAssignVariant(ampCatalogModelId, ampAssignVariant);
  if (pair) {
    const defaultCab = (await getCatalogModelIdForHex(pair.cabHex, "Cab"))?.trim() ?? "";
    if (defaultCab && cabTrim.toLowerCase() === defaultCab.toLowerCase()) return;
  }
  const out = await invoke<string>("probe_slot_model_usb", {
    op: "replace",
    slotIndex: destKi,
    catalogModelId: ampCatalogModelId.trim(),
    assignVariant: ampAssignVariant.trim(),
    cabCatalogModelId: cabTrim,
    cabAssignVariant: cabAssignVariant.trim() || "single",
  });
  console.info("[MatrixPaste] amp+cab cab", cabTrim, out);
}

async function resolveCabDualClipboardCab2(
  kemplineSlotIndex: number,
  dualCatalogModelId: string,
): Promise<{ modelId: string; assignVariant: string } | null> {
  const dualNorm = dualCatalogModelId.trim().toLowerCase();
  let fromParts: string | null = null;
  if (loadedPresetIndex === currentPresetIndex) {
    try {
      const dualParts = await resolveSlotDualParts(kemplineSlotIndex, {
        catalogModelId: dualCatalogModelId,
        kind: "cab_dual",
      });
      if (dualParts?.kind === "cab_dual" && dualParts.parts.length === 2) {
        fromParts = dualParts.parts[1]?.modelId?.trim() || null;
      }
    } catch {
      /* repli sync UI */
    }
  }
  const pair = await cabDualHexPairFromAssignVariant(dualCatalogModelId, "dual");
  const defaultCab2 = pair
    ? (await getCatalogModelIdForCabDualCab2Hex(
        dualCatalogModelId,
        pair.cab2Hex,
        pair.cab1Hex,
      ))?.trim() ?? ""
    : "";
  const pickNonDefault = (id: string | null | undefined): string | null => {
    const t = (id ?? "").trim();
    if (!t) return null;
    if (defaultCab2 && t.toLowerCase() === defaultCab2.toLowerCase()) return null;
    return t;
  };
  let modelId = pickNonDefault(fromParts);
  if (
    !modelId &&
    lastProbePickerAssignContext &&
    lastProbePickerAssignContext.ki === kemplineSlotIndex &&
    lastProbePickerAssignContext.catalogModelId.trim().toLowerCase() === dualNorm
  ) {
    modelId = pickNonDefault(lastProbePickerAssignContext.cabDualCab2ModelId);
  }
  if (
    !modelId &&
    cabDualPickerSync &&
    cabDualPickerSync.dualCatalogModelId.trim().toLowerCase() === dualNorm &&
    selectedParamsKemplineSlotIndex === kemplineSlotIndex
  ) {
    modelId = pickNonDefault(cabDualPickerSync.cab2CatalogModelId);
  }
  if (!modelId) return null;
  const slotCat =
    cabDualPickerSync?.slotCategory ??
    (await getPresetMetaForId(dualCatalogModelId))?.categoryName ??
    "Cab";
  const loc = catalogPickerDataCache
    ? findCabModelPickerLocation(catalogPickerDataCache, modelId, slotCat)
    : null;
  return {
    modelId,
    assignVariant: loc?.assignVariant ?? "single",
  };
}

async function pasteCabDualCab2IfNeeded(
  destKi: number,
  dualCatalogModelId: string,
  cab2ModelId: string,
  cab2AssignVariant: string,
): Promise<void> {
  const cab2Trim = cab2ModelId.trim();
  if (!cab2Trim) return;
  const pair = await cabDualHexPairFromAssignVariant(dualCatalogModelId, "dual");
  if (pair) {
    const defaultCab2 =
      (
        await getCatalogModelIdForCabDualCab2Hex(
          dualCatalogModelId,
          pair.cab2Hex,
          pair.cab1Hex,
        )
      )?.trim() ?? "";
    if (defaultCab2 && cab2Trim.toLowerCase() === defaultCab2.toLowerCase()) return;
  }
  const out = await invoke<string>("probe_slot_model_usb", {
    op: "replace",
    slotIndex: destKi,
    catalogModelId: dualCatalogModelId.trim(),
    assignVariant: "dual",
    cabCatalogModelId: cab2Trim,
    cabAssignVariant: cab2AssignVariant.trim() || "single",
    cabDualCabIndex: 1,
  });
  console.info("[MatrixPaste] cab dual cab2 patch", cab2Trim, out);
}

async function copyMatrixSlotFromCell(kemplineSlotIndex: number, slot: SlotDebug): Promise<void> {
  if (!canCopyMatrixSlot(slot) || currentPresetIndex < 0) return;
  const idTrim =
    (slot.catalogModelId ?? "").trim() ||
    (await getCatalogModelIdForHex(slot.moduleHex, slot.category))?.trim() ||
    "";
  if (!idTrim) {
    console.warn("[MatrixClipboard] copie impossible : pas d’id catalogue pour", slot.moduleHex);
    return;
  }
  const meta = await getPresetMetaForId(idTrim);
  const { assignVariant, categoryName } = await resolveMatrixClipboardAssignContext(
    slot,
    kemplineSlotIndex,
    idTrim,
    meta,
  );
  const catalogSignal = pickSignal(meta, slot.moduleHex);

  let rawChain: ChainParamValueJson[] | null = null;
  if (loadedPresetIndex === currentPresetIndex) {
    rawChain = await resolveChainValuesForKemplineSlot(
      kemplineSlotIndex,
      slot,
      idTrim,
      categoryName,
      catalogSignal,
    );
  }

  const chainParamsBySymbolicId = await buildChainParamsMapForCopy(
    slot,
    kemplineSlotIndex,
    idTrim,
    categoryName,
    catalogSignal,
    rawChain,
  );

  let cabDualCab2ModelId: string | null = null;
  let cabDualCab2AssignVariant: string | null = null;
  let ampCabCabModelId: string | null = null;
  let ampCabCabAssignVariant: string | null = null;
  const assignNorm = assignVariant.trim().toLowerCase();
  if (assignNorm === "dual") {
    const cab2 = await resolveCabDualClipboardCab2(kemplineSlotIndex, idTrim);
    if (cab2) {
      cabDualCab2ModelId = cab2.modelId;
      cabDualCab2AssignVariant = cab2.assignVariant;
    }
  } else if (assignNorm === "amp+cab" || assignNorm === "amp+cab-legacy") {
    const cab = await resolveAmpCabClipboardCab(
      kemplineSlotIndex,
      idTrim,
      assignVariant,
    );
    if (cab) {
      ampCabCabModelId = cab.modelId;
      ampCabCabAssignVariant = cab.assignVariant;
    }
  }

  matrixSlotClipboard = {
    presetIndex: currentPresetIndex,
    sourcePresetLabel: (presetLabelEl.textContent ?? "").trim() || `preset ${currentPresetIndex}`,
    sourceKemplineIndex: kemplineSlotIndex,
    path: matrixSlotPath(kemplineSlotIndex),
    catalogModelId: idTrim,
    displayName: slot.name.trim(),
    categoryName,
    assignVariant,
    moduleHex: slot.moduleHex,
    catalogSignal,
    chainParamsBySymbolicId,
    cabDualCab2ModelId,
    cabDualCab2AssignVariant,
    ampCabCabModelId,
    ampCabCabAssignVariant,
  };
  console.info(
    "[MatrixClipboard] copié",
    matrixSlotClipboard.displayName,
    `from=${matrixSlotClipboard.sourcePresetLabel}`,
    `path=${matrixSlotClipboard.path}`,
    `variant=${assignVariant}`,
    `cat=${categoryName}`,
    cabDualCab2ModelId ? `cab2=${cabDualCab2ModelId}` : "",
    ampCabCabModelId ? `ampCab=${ampCabCabModelId}` : "",
    `params=${chainParamsBySymbolicId.size}`,
  );
  setStatus(
    `Copié : ${matrixSlotClipboard.displayName} (${matrixSlotClipboard.sourcePresetLabel}) — coller sur case vide`,
  );
}

async function replayMatrixClipboardParams(
  destKi: number,
  clipboard: MatrixSlotClipboard,
  slot: SlotDebug,
): Promise<void> {
  if (clipboard.chainParamsBySymbolicId.size === 0) return;
  const found = await findModelDefinitionForSlot(
    slot,
    clipboard.catalogModelId,
    clipboard.categoryName,
  );
  const allParams = found?.entry.params ?? [];
  if (allParams.length === 0) return;

  const catalogSignal = clipboard.catalogSignal ?? pickSignal(await getPresetMetaForId(clipboard.catalogModelId), slot.moduleHex);
  const writeOrder = paramsVisibleForSignal(allParams, catalogSignal);

  await waitUntilHardwareSyncIdle(15_000);
  await enqueueHardwareSlotSwitch(destKi);
  await delayMs(80);

  for (const p of writeOrder) {
    const sid = (p.symbolicID ?? "").trim();
    if (!sid || !clipboard.chainParamsBySymbolicId.has(sid)) continue;
    const cv = clipboard.chainParamsBySymbolicId.get(sid)!;
    let rawValue: number | null = null;
    if (typeof cv === "boolean") rawValue = cv ? 1 : 0;
    else if (typeof cv === "number" && Number.isFinite(cv)) rawValue = cv;
    else continue;

    const rowIndexInAll = allParams.indexOf(p);
    const paramIndex = liveWriteParamIndexForRow(allParams, rowIndexInAll, catalogSignal);
    const rawMin = typeof p.min === "number" && Number.isFinite(p.min) ? p.min : null;
    const rawMax = typeof p.max === "number" && Number.isFinite(p.max) ? p.max : null;
    const vtRaw = p.valueType;
    const valueType =
      vtRaw !== undefined && vtRaw !== null && Number.isFinite(Number(vtRaw))
        ? Number(vtRaw)
        : null;
    const pending: PendingLiveWrite = {
      slotIndex: destKi,
      paramIndex,
      symbolicId: sid,
      displayType: (p.displayType ?? "").trim() || null,
      valueType,
      rawValue,
      rawMin,
      rawMax,
    };
    try {
      await invoke("write_live_param", {
        slotIndex: destKi,
        paramIndex,
        symbolicId: sid,
        displayType: pending.displayType,
        valueType,
        rawValue: liveWriteUsbNormalized01(pending),
        chainMin: rawMin ?? undefined,
        chainMax: rawMax ?? undefined,
      });
      if (currentPresetIndex >= 0) {
        recordLiveChainParamOverrideForKemplineSlot(currentPresetIndex, destKi, sid, cv);
      }
    } catch (e) {
      console.warn("[MatrixPaste] param", sid, e);
    }
    await delayMs(15);
  }
  const sessionVals = await resolveChainValuesForKemplineSlot(
    destKi,
    slot,
    clipboard.catalogModelId,
    clipboard.categoryName,
    catalogSignal,
  );
  if (sessionVals) setSlotChainSessionValues(currentPresetIndex, destKi, sessionVals);
}

async function pasteMatrixSlotToCell(
  destKi: number,
  options?: { skipInteractionLock?: boolean },
): Promise<boolean> {
  const run = async (): Promise<boolean> => {
  const cb = matrixSlotClipboard;
  const slots = lastHwSyncNormalizedSlots;
  if (!cb || !slots || slots.length !== 16 || !canPasteMatrixSlotToEmpty(destKi, slots)) {
    console.warn("[MatrixPaste] destination invalide");
    return false;
  }
  if (loading || loadedPresetIndex !== currentPresetIndex) {
    console.warn("[MatrixPaste] preset pas prêt");
    return false;
  }

  const idTrim = cb.catalogModelId;
  const metaEarly = await getPresetMetaForId(idTrim);
  const moduleHexOpt =
    (await moduleHexForUsbVariant(idTrim, cb.assignVariant, metaEarly))?.trim() ||
    cb.moduleHex ||
    undefined;
  const optimisticSlot: SlotDebug = {
    category: cb.categoryName,
    name: cb.displayName,
    catalogModelId: idTrim,
    moduleHex: moduleHexOpt,
  };

  const prevSnapshot =
    lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16
      ? lastHwSyncNormalizedSlots.map((s) => ({ ...s }))
      : null;

  if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
    const next = lastHwSyncNormalizedSlots.map((s, i) =>
      i === destKi ? { ...optimisticSlot } : { ...s },
    );
    lastHwSyncNormalizedSlots = next;
    lastHwSyncChainSignature = chainLayoutSignature(next);
  }

  patchMatrixSlotVisualFromSlot(destKi, optimisticSlot);
  if (selectedParamsKemplineSlotIndex === destKi) {
    selectParamsPaneByKemplineIndex(destKi);
  }
  if (currentPresetIndex >= 0) {
    clearLiveChainOverridesForKemplineSlot(currentPresetIndex, destKi);
  }
  markLiveWriteUiInteraction();
  slotModelUsbProbeInFlight = destKi;
  const assignVariant = cb.assignVariant.trim().toLowerCase();
  if (assignVariant === "amp+cab" || assignVariant === "amp+cab-legacy") {
    lastProbePickerAssignContext = {
      ki: destKi,
      catalogModelId: idTrim,
      assignVariant: cb.assignVariant,
      category: cb.categoryName,
      ampCabCabModelId: cb.ampCabCabModelId,
      ampCabCabAssignVariant: cb.ampCabCabAssignVariant,
    };
    ampCabDualActiveTab = 0;
  } else if (assignVariant === "dual" && normalizeCategory(cb.categoryName) === "cab") {
    lastProbePickerAssignContext = {
      ki: destKi,
      catalogModelId: idTrim,
      assignVariant: cb.assignVariant,
      category: cb.categoryName,
      cabDualCab2ModelId: cb.cabDualCab2ModelId,
    };
    cabDualActiveTab = 0;
  } else {
    lastProbePickerAssignContext = null;
  }
  try {
    await loadAndShowModelsParamsFromCatalogDefaults(optimisticSlot, idTrim, destKi, {
      assignVariant: cb.assignVariant,
    });

    const out = await invoke<string>("probe_slot_model_usb", {
      op: "add",
      slotIndex: destKi,
      catalogModelId: idTrim,
      assignVariant: cb.assignVariant,
    });
    console.info("[MatrixPaste] add", cb.displayName, idTrim, out);
    mergeProbeSlotModelUntil = {
      ki: destKi,
      deadline: Date.now() + PROBE_SLOT_MERGE_GRACE_MS,
      mergeTraceEmitted: false,
    };
    suppressUsbPresetPollUntilMs = Date.now() + USB_PRESET_POLL_SUPPRESS_AFTER_PROBE_MS;
    lastSoftUsbPresetReadAt = Date.now();

    if (assignVariant !== "dual") {
      await replayMatrixClipboardParams(destKi, cb, optimisticSlot);
    }

    if (
      (assignVariant === "amp+cab" || assignVariant === "amp+cab-legacy") &&
      cb.ampCabCabModelId
    ) {
      await pasteAmpCabCabIfNeeded(
        destKi,
        idTrim,
        cb.assignVariant,
        cb.ampCabCabModelId,
        cb.ampCabCabAssignVariant ?? "single",
      );
      if (lastProbePickerAssignContext?.ki === destKi) {
        lastProbePickerAssignContext = {
          ...lastProbePickerAssignContext,
          ampCabCabModelId: cb.ampCabCabModelId,
          ampCabCabAssignVariant: cb.ampCabCabAssignVariant ?? "single",
        };
      }
      mergeProbeSlotModelUntil = {
        ki: destKi,
        deadline: Date.now() + PROBE_SLOT_MERGE_GRACE_MS,
        mergeTraceEmitted: false,
      };
    }

    if (assignVariant === "dual" && cb.cabDualCab2ModelId) {
      await pasteCabDualCab2IfNeeded(
        destKi,
        idTrim,
        cb.cabDualCab2ModelId,
        cb.cabDualCab2AssignVariant ?? "single",
      );
      if (lastProbePickerAssignContext?.ki === destKi) {
        lastProbePickerAssignContext = {
          ...lastProbePickerAssignContext,
          cabDualCab2ModelId: cb.cabDualCab2ModelId,
        };
      }
      mergeProbeSlotModelUntil = {
        ki: destKi,
        deadline: Date.now() + PROBE_SLOT_MERGE_GRACE_MS,
        mergeTraceEmitted: false,
      };
    }

    if (selectedParamsKemplineSlotIndex === destKi) {
      await loadAndShowModelsParamsForSlot(optimisticSlot, destKi);
    }
    const fromLabel =
      cb.presetIndex !== currentPresetIndex ? ` depuis ${cb.sourcePresetLabel}` : "";
    setStatus(`Collé : ${cb.displayName}${fromLabel} → slot ${destKi + 1}`);
    return true;
  } catch (e) {
    console.warn("[MatrixPaste]", e);
    if (prevSnapshot) {
      const old = prevSnapshot[destKi]!;
      patchMatrixSlotVisualFromSlot(destKi, old);
      lastHwSyncNormalizedSlots = prevSnapshot.map((s) => ({ ...s }));
      lastHwSyncChainSignature = chainLayoutSignature(lastHwSyncNormalizedSlots);
      if (selectedParamsKemplineSlotIndex === destKi) {
        selectParamsPaneByKemplineIndex(destKi);
        await loadAndShowModelsParamsForSlot(old, destKi);
      }
    } else {
      scheduleLoadForPreset(currentPresetIndex, true);
    }
    return false;
  } finally {
    slotModelUsbProbeInFlight = null;
  }
  };
  if (options?.skipInteractionLock) return run();
  return withMatrixUsbInteractionLock(`Collage slot ${destKi + 1}…`, run);
}

/** Déplacer un bloc : copier → vider source → coller destination. */
async function moveMatrixSlotFromTo(sourceKi: number, destKi: number): Promise<void> {
  if (isMatrixUsbInteractionLocked()) return;
  const slots = lastHwSyncNormalizedSlots;
  if (!slots || slots.length !== 16) {
    console.warn("[MatrixMove] snapshot grille absent");
    return;
  }
  if (!canMoveMatrixSlotToEmpty(sourceKi, destKi, slots)) {
    console.warn("[MatrixMove] déplacement invalide", sourceKi, "→", destKi);
    return;
  }
  const sourceSlot = slots[sourceKi];
  if (!sourceSlot) return;

  await withMatrixUsbInteractionLock(
    `Déplacement bloc ${sourceKi + 1} → ${destKi + 1}…`,
    async () => {
    await copyMatrixSlotFromCell(sourceKi, sourceSlot);
    if (!matrixSlotClipboard) {
      console.warn("[MatrixMove] copie échouée", sourceKi);
      return;
    }

    // Purger la source **avant** le coller : après un add le HX est sur la destination ;
    // un remove tardif sur la source est souvent ignoré (doublon HW slot source + dest).
    const removed = await removeMatrixSlotFromCell(sourceKi, {
      reselect: false,
      skipInteractionLock: true,
    });
    if (!removed) {
      console.warn("[MatrixMove] purge source échouée — déplacement annulé", sourceKi);
      setStatus(`Déplacement annulé : impossible de vider le slot ${sourceKi + 1}`);
      return;
    }

    await delayMs(MATRIX_USB_OP_SETTLE_MS);

    const pasted = await pasteMatrixSlotToCell(destKi, { skipInteractionLock: true });
    if (!pasted) {
      console.warn("[MatrixMove] coller échoué après purge source — restauration", sourceKi);
      const restored = await pasteMatrixSlotToCell(sourceKi, { skipInteractionLock: true });
      if (restored) {
        setStatus(`Déplacement annulé — bloc remis sur le slot ${sourceKi + 1}`);
      } else {
        setStatus(
          `Erreur déplacement (slot ${sourceKi + 1} vidé) — utiliser Coller ou recharger le preset`,
        );
      }
      return;
    }

    armProbeSlotMergeGrace(destKi, sourceKi);
    suppressUsbPresetPollUntilMs = Date.now() + USB_PRESET_POLL_SUPPRESS_AFTER_PROBE_MS;
    lastSoftUsbPresetReadAt = Date.now();
    console.info("[MatrixMove] ok", sourceKi, "→", destKi, matrixSlotClipboard.displayName);
    setStatus(`Bloc déplacé (${sourceKi + 1} → ${destKi + 1})`);
    await focusMatrixSlotParamsPane(destKi);
    },
  );
}

function matrixDropTargetFromElement(el: Element | null): HTMLElement | null {
  if (!el) return null;
  const host = el as HTMLElement;
  const direct = host.closest?.(
    "[data-kempline-slot-index].node-empty.node--hx-slot:not(.node-empty-column-blocked)",
  ) as HTMLElement | null;
  if (direct) return direct;
  const cell = host.closest?.(".hx-matrix-cell") as HTMLElement | null;
  if (!cell) return null;
  return cell.querySelector(
    "[data-kempline-slot-index].node-empty.node--hx-slot:not(.node-empty-column-blocked)",
  ) as HTMLElement | null;
}

function matrixKiFromDropTarget(el: HTMLElement): number | null {
  const ki = Number.parseInt(el.dataset.kemplineSlotIndex ?? "", 10);
  return Number.isFinite(ki) && ki >= 0 && ki <= 15 ? ki : null;
}

function clearMatrixDragOverHighlights(): void {
  contentEl
    .querySelectorAll(".node--matrix-drag-over")
    .forEach((n) => n.classList.remove("node--matrix-drag-over"));
}

function bindMatrixSlotDragSource(el: HTMLElement, slot: SlotDebug, ki: number): void {
  if (!canCopyMatrixSlot(slot)) return;
  el.classList.add("node--matrix-draggable");

  el.addEventListener("pointerdown", (ev) => {
    if (isMatrixUsbInteractionLocked()) return;
    if ((ev.target as HTMLElement).closest(".models-slot-remove-btn")) return;
    if (currentPresetIndex < 0 || isModelsContentBusy()) return;
    if (ev.button !== 0) return;

    ev.preventDefault();
    matrixDragSourceKi = ki;
    el.setPointerCapture(ev.pointerId);
    el.classList.add("node--matrix-drag-source");
  });

  el.addEventListener("pointermove", (ev) => {
    if (matrixDragSourceKi !== ki) return;
    clearMatrixDragOverHighlights();

    const target = document.elementFromPoint(ev.clientX, ev.clientY);
    const dropEl = matrixDropTargetFromElement(target);
    if (!dropEl) return;

    const destKi = matrixKiFromDropTarget(dropEl);
    const slots = lastHwSyncNormalizedSlots;
    if (
      destKi === null ||
      !slots ||
      !canMoveMatrixSlotToEmpty(ki, destKi, slots)
    ) {
      return;
    }

    dropEl.classList.add("node--matrix-drag-over");
  });

  el.addEventListener("pointerup", (ev) => {
    if (matrixDragSourceKi !== ki) return;
    if (isMatrixUsbInteractionLocked()) {
      matrixDragSourceKi = null;
      el.classList.remove("node--matrix-drag-source");
      clearMatrixDragOverHighlights();
      return;
    }

    clearMatrixDragOverHighlights();
    el.classList.remove("node--matrix-drag-source");
    if (el.hasPointerCapture(ev.pointerId)) {
      el.releasePointerCapture(ev.pointerId);
    }

    const target = document.elementFromPoint(ev.clientX, ev.clientY);
    const dropEl = matrixDropTargetFromElement(target);

    matrixDragSourceKi = null;

    if (!dropEl) return;
    const destKi = matrixKiFromDropTarget(dropEl);
    if (destKi === null) return;

    console.info("[MatrixMove] pointerup", ki, "→", destKi);
    setStatus(`Déplacement bloc ${ki + 1} → ${destKi + 1}…`);
    void moveMatrixSlotFromTo(ki, destKi);
  });

  el.addEventListener("pointercancel", (ev) => {
    if (matrixDragSourceKi !== ki) return;
    matrixDragSourceKi = null;
    el.classList.remove("node--matrix-drag-source");
    if (el.hasPointerCapture(ev.pointerId)) {
      el.releasePointerCapture(ev.pointerId);
    }
    clearMatrixDragOverHighlights();
  });
}

function bindMatrixSlotDropTarget(_el: HTMLElement, _destKi: number): void {
  // Géré par pointermove/pointerup sur la source (Pointer Events)
}

function initMatrixDragDrop(): void {
  // Géré par Pointer Events dans bindMatrixSlotDragSource
}

function onMatrixSlotContextMenu(ev: MouseEvent, el: HTMLElement, slot: SlotDebug | null): void {
  const kRaw = el.dataset.kemplineSlotIndex;
  const ki = kRaw !== undefined && kRaw !== "" ? Number.parseInt(kRaw, 10) : Number.NaN;
  if (!Number.isFinite(ki) || ki < 0 || ki > 15) return;
  if (currentPresetIndex < 0 || isModelsContentBusy()) return;

  const slots = lastHwSyncNormalizedSlots;
  const slotNow =
    slots && ki >= 0 && ki < slots.length ? slots[ki] : slot;
  const isEmptyCell = slotNow === null || slotNow === undefined || isEmptyGridCell(slotNow);
  const canCopy = slotNow != null && canCopyMatrixSlot(slotNow);
  const canPaste =
    isEmptyCell && slots !== null && slots.length === 16 && canPasteMatrixSlotToEmpty(ki, slots);
  if (!canCopy && !canPaste) return;

  matrixCtxTargetKemplineIndex = ki;
  const menu = document.getElementById("models-ctx-menu");
  const copyItem = document.getElementById("models-ctx-copy");
  const pasteItem = document.getElementById("models-ctx-paste");
  if (!menu || !copyItem || !pasteItem) return;

  copyItem.classList.toggle("disabled", !canCopy);
  pasteItem.classList.toggle("disabled", !canPaste);

  const x = Math.min(ev.clientX, window.innerWidth - 200);
  const y = Math.min(ev.clientY, window.innerHeight - 80);
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;
  menu.classList.add("visible");
}

function initMatrixContextMenu(): void {
  const menu = document.getElementById("models-ctx-menu");
  const copyItem = document.getElementById("models-ctx-copy");
  const pasteItem = document.getElementById("models-ctx-paste");
  if (!menu || !copyItem || !pasteItem) return;

  document.addEventListener("click", hideMatrixContextMenu);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") hideMatrixContextMenu();
  });

  copyItem.addEventListener("click", (e) => {
    e.stopPropagation();
    if (copyItem.classList.contains("disabled")) return;
    const ki = matrixCtxTargetKemplineIndex;
    const slots = lastHwSyncNormalizedSlots;
    if (ki === null || !slots || ki < 0 || ki >= slots.length) return;
    const slot = slots[ki];
    if (!slot || !canCopyMatrixSlot(slot)) return;
    hideMatrixContextMenu();
    void copyMatrixSlotFromCell(ki, slot);
  });

  pasteItem.addEventListener("click", (e) => {
    e.stopPropagation();
    if (pasteItem.classList.contains("disabled")) return;
    const ki = matrixCtxTargetKemplineIndex;
    const slots = lastHwSyncNormalizedSlots;
    if (ki === null || !slots || !canPasteMatrixSlotToEmpty(ki, slots)) return;
    hideMatrixContextMenu();
    void pasteMatrixSlotToCell(ki);
  });
}

/**
 * `slot === null` : slot vide (clic → rien dans le panneau).
 * Sinon : bloc avec modèle (définitions `.models` + liste paramètre / valeur).
 */
function bindSlotParamsInteraction(el: HTMLElement, slot: SlotDebug | null) {
  el.classList.add("node--params-clickable");
  el.tabIndex = 0;
  el.setAttribute("role", "button");
  if (slot !== null) {
    const kRaw = el.dataset.kemplineSlotIndex;
    const kemplineSlotIndex =
      kRaw !== undefined && kRaw !== "" ? Number.parseInt(kRaw, 10) : undefined;
    el.dataset.slotSelectionKey = makeSlotSelectionKey(
      slot,
      Number.isFinite(kemplineSlotIndex) ? kemplineSlotIndex : undefined,
    );
  }
  const activate = (userInitiated: boolean) => {
    if (autoSelectFallbackTimer !== null) {
      window.clearTimeout(autoSelectFallbackTimer);
      autoSelectFallbackTimer = null;
    }
    const kRaw = el.dataset.kemplineSlotIndex;
    const kemplineSlotIndex =
      kRaw !== undefined && kRaw !== "" ? Number.parseInt(kRaw, 10) : undefined;
    const nextSlotIdx = Number.isFinite(kemplineSlotIndex) ? (kemplineSlotIndex as number) : null;
    const nextSlotKey =
      slot === null
        ? (nextSlotIdx !== null ? `empty|${nextSlotIdx}` : "empty")
        : makeSlotSelectionKey(
            slot,
            Number.isFinite(kemplineSlotIndex) ? kemplineSlotIndex : undefined,
          );

    clearSlotSelectionVisual();
    selectedParamsSlotEl = el;
    el.classList.add("node--selected");
    if (slot !== null && nextSlotIdx !== null) {
      attachSelectedSlotRemoveButton(el, nextSlotIdx);
    }
    if (selectedParamsSlotKey !== nextSlotKey) {
      selectedParamsInPlaceUpdater = null;
      selectedParamsInPlaceSlotKey = null;
      selectedParamsHwWireContext = null;
    }
    selectedParamsSlotKey = nextSlotKey;
    selectedParamsKemplineSlotIndex = Number.isFinite(kemplineSlotIndex)
      ? (kemplineSlotIndex as number)
      : null;
    if (nextSlotIdx !== null) {
      clearSpecialSlotPickerContext();
    }
    selectedParamsPresetIndex = currentPresetIndex;
    selectedParamsValuesSig = null;
    const hwBusRaw = el.dataset.hwSlotBus;
    const hwBusParsed =
      hwBusRaw !== undefined && hwBusRaw !== "" ? Number.parseInt(hwBusRaw, 10) : Number.NaN;
    selectedSpecialHwSlotBus =
      nextSlotIdx === null && Number.isFinite(hwBusParsed) ? (hwBusParsed as number) : null;
    if (slot !== null) {
      applyPickerForStructuralSlot(slot, selectedSpecialHwSlotBus);
    }
    const now = Date.now();
    const tooSoon = now - lastUserHwSlotSwitchAt < 120;
    const duplicate = nextSlotIdx !== null && lastUserHwSlotSwitchIndex === nextSlotIdx && tooSoon;
    const shouldSwitchHardware =
      userInitiated &&
      !suppressNextUiSlotHardwareSwitch &&
      !loading &&
      loadedPresetIndex === currentPresetIndex &&
      nextSlotIdx !== null &&
      !duplicate;
    suppressNextUiSlotHardwareSwitch = false;
    if (shouldSwitchHardware) {
      lastUserHwSlotSwitchAt = now;
      lastUserHwSlotSwitchIndex = nextSlotIdx;
    }
    void (async () => {
      const shouldSwitchIoHardware =
        userInitiated &&
        !suppressNextUiSlotHardwareSwitch &&
        !loading &&
        loadedPresetIndex === currentPresetIndex &&
        slot !== null &&
        Number.isFinite(hwBusParsed) &&
        nextSlotIdx === null;
      if (shouldSwitchIoHardware) {
        await waitUntilHardwareSyncIdle(15_000);
        try {
          await invoke("switch_active_hardware_special_slot", { slotBus: hwBusParsed });
        } catch (e) {
          console.warn("[HwIoSlotSync] switch_active_hardware_special_slot", e);
        }
      } else if (shouldSwitchHardware && Number.isFinite(kemplineSlotIndex)) {
        await waitUntilHardwareSyncIdle(15_000);
        await enqueueHardwareSlotSwitch(kemplineSlotIndex as number);
      }
      if (slot === null) {
        suppressNextUiSlotHardwareSwitch = false;
        clearModelsParamsPaneContent();
        return;
      }
      if (!userInitiated && hwUi.blockSyntheticParamsLoad) {
        const deferSlot = slot;
        const deferKi = Number.isFinite(kemplineSlotIndex) ? (kemplineSlotIndex as number) : undefined;
        hwUi.scheduleAfterHwGesture("params", () => {
          if (
            deferSlot === null ||
            selectedParamsKemplineSlotIndex !== deferKi ||
            selectedParamsPresetIndex !== currentPresetIndex
          ) {
            return;
          }
          void loadAndShowModelsParamsForSlot(deferSlot, deferKi);
        });
        return;
      }
      await loadAndShowModelsParamsForSlot(
        slot,
        Number.isFinite(kemplineSlotIndex) ? kemplineSlotIndex : undefined,
      );
      if (slot !== null && normalizeCategory(slot.category) === "input") {
        void refreshInputPickerFromLiveWireDelayed();
      }
      if (slot !== null && normalizeCategory(slot.category) === "split") {
        void refreshSplitPickerFromLiveWireDelayed();
      }
    })();
  };
  el.addEventListener("click", (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    activate(ev.isTrusted);
  });
  el.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter" || ev.key === " ") {
      ev.preventDefault();
      activate(ev.isTrusted);
    }
  });
  el.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    onMatrixSlotContextMenu(ev, el, slot);
  });
}

function padNum(n: number): string {
  return String(n).padStart(3, "0");
}

function isEmpty(name: string): boolean {
  return !name || name === "<empty>";
}

function purgeModelsUi() {
  connectedDeviceName = null;
  matrixUsbInteractionLockDepth = 0;
  presetLoadUiLockDepth = 0;
  document.body.classList.remove("models-matrix-usb-busy");
  document.body.classList.remove("models-preset-load-busy");
  clearMatrixSlotClipboard();
  hideMatrixContextMenu();
  lastProbePickerAssignContext = null;
  clearPath1InputSourceHighlightOverride();
  clearPath1InputMatrixWire();
  clearPath1SplitTypeHighlightOverride();
  currentPresetIndex = -1;
  loadedPresetIndex = -1;
  lastRequestedPresetIndex = -1;
  lastPresetNamesSig = "";
  mainWindowPresetDriftStreak = 0;
  stopLoadingHeartbeat();
  presetLabelEl.textContent = "--";
  renderEmpty("En attente du HX...");
  setStatus("HX déconnecté.");
}

function renderEmpty(text: string) {
  if (modelsSyncTraceEnabled()) {
    const st =
      new Error().stack?.split("\n").slice(2, 7).map((s) => s.trim()).join(" <= ") ?? "";
    emitModelsSyncTrace(
      `renderEmpty text=${JSON.stringify(text)} preset=${currentPresetIndex} loaded=${loadedPresetIndex} loading=${loading} | ${st}`,
    );
  }
  clearSlotSelectionVisual();
  clearSelectedParamsContext();
  resetModelsParamsIdleHint();
  contentEl.innerHTML = `<div class="empty">${text}</div>`;
}

/** Réponse `get_active_preset_stomp_layout` (serde camelCase). */
type ActivePresetStompLayout = {
  routing: {
    splitAfterCol: number;
    mergeAfterCol: number;
    inferredFrom: string;
    kemplineGridOk: boolean;
  };
  chain: Array<{
    index: number;
    kind: string;
    topCategory: string;
    topName: string;
    bottomCategory: string;
    bottomName: string;
    topGridX: number | null;
    topGridY: number | null;
    bottomGridX: number | null;
    bottomGridY: number | null;
  }>;
};

/** PNG dans `icons_category/` ; clés = `normalizeCategory` comme `HX_ModelCatalog.json` `categories[].name`. */
const CATEGORY_ICON_BY_KEY: Record<string, string> = {
  none: "FX_HX_Category_None.png",
  distortion: "FX_HX_Category_Distortion.png",
  dynamics: "FX_HX_Category_Dynamics.png",
  dynamic: "FX_HX_Category_Dynamics.png",
  eq: "FX_HX_Category_EQ.png",
  modulation: "FX_HX_Category_Modulation.png",
  delay: "FX_HX_Category_Delay.png",
  reverb: "FX_HX_Category_Reverb.png",
  "pitch/synth": "FX_HX_Category_PitchSynth.png",
  filter: "FX_HX_Category_Filter.png",
  wah: "FX_HX_Category_Wah.png",
  "volume/pan": "FX_HX_Category_VolumePan.png",
  "vol/pan": "FX_HX_Category_VolumePan.png",
  amp: "FX_HX_Category_Amp.png",
  preamp: "FX_HX_Category_Preamp.png",
  "amp+cab": "FX_HX_Category_AmpCab.png",
  cab: "FX_HX_Category_Cab.png",
  ir: "FX_HX_Category_Impulse Response.png",
  "impulse response": "FX_HX_Category_Impulse Response.png",
  "send/return": "FX_HX_Category_SendReturn_%3.png",
  looper: "FX_HX_Category_Looper.png",
  input: "icon-input-category.png",
  output: "icon-output-category.png",
  split: "FX_HX_Category_Split.png",
  merge: "FX_HX_Category_Merge.png",
  favorites: "FX_HX_Category_Favorites.png",
  routing: "FX_HX_Category_Split.png",
};

/** Diminutif du type d’effet (comme HX Edit : Dyn, Dis…), pas du nom du modèle. */
const EFFECT_TYPE_ABBREV: Record<string, string> = {
  none: "—",
  distortion: "Dist",
  dynamic: "Dyn",
  dynamics: "Dyn",
  eq: "EQ",
  modulation: "Mod",
  delay: "Del",
  reverb: "Rev",
  "pitch/synth": "Pch",
  filter: "Filt",
  wah: "Wah",
  "volume/pan": "Vol",
  "vol/pan": "Vol",
  amp: "Amp",
  preamp: "Pre",
  "amp+cab": "AmC",
  cab: "Cab",
  ir: "IR",
  "impulse response": "IR",
  "send/return": "S/R",
  looper: "Loop",
  input: "In",
  output: "Out",
  split: "Spt",
  merge: "Mrg",
  favorites: "Fav",
  routing: "Rte",
};

function normalizeCategory(category: string): string {
  const c = category.trim().toLowerCase();
  const compact = c.replace(/\s+/g, "");
  if (compact === "amp+cablegacy" || compact === "ampcablegacy" || c === "amp+cab legacy") {
    return "amp+cab";
  }
  if (compact === "ampcab" || compact === "amp+cab") return "amp+cab";
  return c;
}

function slotWantsAmpCabDualTabs(
  slot: SlotDebug,
  assignVariantHint?: string | null,
): boolean {
  if (isAmpCabFamilySlotCategory(slot.category)) return true;
  const v = (assignVariantHint ?? "").trim().toLowerCase();
  return v === "amp+cab" || v === "amp+cab-legacy";
}

function cabDualSubCategoryHint(meta: PresetMetaJson | null | undefined): string {
  const sc = meta?.subCategory;
  if (typeof sc === "string") return sc.trim().toLowerCase();
  if (Array.isArray(sc)) {
    return sc
      .map((x) => (typeof x === "string" ? x.trim().toLowerCase() : ""))
      .filter(Boolean)
      .join(" ");
  }
  return "";
}

function isCabLegacyFromMeta(meta: PresetMetaJson | null | undefined): boolean {
  return cabDualSubCategoryHint(meta).includes("legacy");
}

/** Variante USB live write Cab dual : IR moderne vs hybrid legacy (`c3:19`). */
function cabDualAssignVariantFromMeta(meta: PresetMetaJson | null | undefined): string {
  return isCabLegacyFromMeta(meta) ? "dual-legacy" : "dual";
}

function cabAssignVariantFromMeta(meta: PresetMetaJson | null | undefined): string | null {
  return isCabLegacyFromMeta(meta) ? "legacy" : null;
}

/** Sous-catégorie picker pour le cab 2 d'un dual IR (hardware : single IR uniquement). */
function cabDualCab2PickerSub(legacy: boolean): string {
  return legacy ? "Single Legacy" : "Single";
}

/** Sous-catégorie picker pour le bloc dual (cab 1) d'un slot Cab dual. */
function cabDualCab1PickerSub(legacy: boolean): string {
  return legacy ? "Dual Legacy" : "Dual";
}

function slotWantsCabDualTabs(
  slot: SlotDebug,
  assignVariantHint?: string | null,
  meta?: PresetMetaJson | null,
): boolean {
  const v = (assignVariantHint ?? "").trim().toLowerCase();
  if (v === "dual") return true;
  if (isCabDualWireHex(slot.moduleHex)) return true;
  if (normalizeCategory(slot.category) !== "cab") return false;
  const sub = cabDualSubCategoryHint(meta);
  return sub === "dual" || sub.includes("dual");
}

/** ID catalogue pour un slot matrice (fil dual cab, catégorie slot, etc.). */
async function resolveSlotCatalogModelId(slot: SlotDebug): Promise<string> {
  const fromSlot = (slot.catalogModelId ?? "").trim();
  if (fromSlot) return fromSlot;
  const wire = (slot.moduleHex ?? "").trim();
  if (!wire) return "";
  const dualWire = cabDualWireParts(wire);
  if (dualWire) {
    const fromCab1 =
      (await getCatalogModelIdForHex(dualWire.cab1Hex, "Cab"))?.trim() ?? "";
    if (fromCab1) {
      const meta = await getPresetMetaForId(fromCab1);
      const sub = cabDualSubCategoryHint(meta);
      if (sub === "dual" || sub.includes("dual")) return fromCab1;
    }
  }
  return (await getCatalogModelIdForHex(wire, slot.category))?.trim() ?? "";
}

/** Le DOM du panneau params correspond-il au mode simple vs onglets Amp/Cab ou Cab 1/2 ? */
function paramsPaneDualStructureMatches(
  dualSlotKind: "amp_cab" | "cab_dual" | null,
): boolean {
  const inner = getModelsParamsInner();
  const hasDualTabs = inner?.querySelector(".models-params-dual-tabs") !== null;
  return (dualSlotKind !== null) === hasDualTabs;
}

function probePickerAssignVariantHint(kemplineSlotIndex: number | undefined): string | null {
  if (kemplineSlotIndex === undefined || !Number.isInteger(kemplineSlotIndex)) return null;
  const ctx = lastProbePickerAssignContext;
  if (!ctx || ctx.ki !== kemplineSlotIndex) return null;
  return ctx.assignVariant;
}

/** Cab 2 connu hors preset dump (collage matrice, probe cab 2, sync UI). */
function probePickerCabDualCab2Hint(
  kemplineSlotIndex: number | undefined,
  dualCatalogModelId: string,
): string | null {
  const dualNorm = dualCatalogModelId.trim().toLowerCase();
  if (kemplineSlotIndex !== undefined && Number.isInteger(kemplineSlotIndex)) {
    const ctx = lastProbePickerAssignContext;
    if (
      ctx &&
      ctx.ki === kemplineSlotIndex &&
      ctx.assignVariant.trim().toLowerCase() === "dual"
    ) {
      const fromCtx = (ctx.cabDualCab2ModelId ?? "").trim();
      if (fromCtx) return fromCtx;
    }
    const cb = matrixSlotClipboard;
    if (
      cb &&
      cb.presetIndex === currentPresetIndex &&
      cb.catalogModelId.trim().toLowerCase() === dualNorm &&
      cb.assignVariant.trim().toLowerCase() === "dual"
    ) {
      const fromCb = (cb.cabDualCab2ModelId ?? "").trim();
      if (fromCb) return fromCb;
    }
    if (
      cabDualPickerSync &&
      cabDualPickerSync.dualCatalogModelId.trim().toLowerCase() === dualNorm
    ) {
      const fromSync = cabDualPickerSync.cab2CatalogModelId.trim();
      if (fromSync) return fromSync;
    }
  }
  return null;
}

async function applyCabDualPane2ModelOverride(
  panes: DualTabPaneConfig[],
  cab2ModelId: string,
  opts?: { forceDefaults?: boolean },
): Promise<DualTabPaneConfig[]> {
  if (panes.length < 2) return panes;
  const pane1 = panes[1];
  if (!pane1) return panes;
  const cab2Trim = cab2ModelId.trim();
  const idMatches =
    (pane1.catalogModelId ?? "").trim().toLowerCase() === cab2Trim.toLowerCase();
  if (!cab2Trim || (idMatches && !opts?.forceDefaults)) {
    return panes;
  }
  const def = await findModelDefinitionBySymbolicId(cab2Trim, "Cab");
  const partMeta = await getPresetMetaForId(cab2Trim);
  const modelImage = await getCatalogModelImageForId(cab2Trim);
  const signal = pickSignal(partMeta, undefined);
  const defaults = await cabDefaultChainValuesForCatalogModelId(cab2Trim);
  const title =
    def?.entry.name.trim() ||
    (await getCatalogModelNameForId(cab2Trim))?.trim() ||
    "—";
  const next = [...panes];
  next[1] = {
    ...pane1,
    catalogModelId: cab2Trim,
    modelTitle: title,
    basedOn: pickBasedOn(partMeta),
    modelImage,
    params: def?.entry.params ?? [],
    chainValues: defaults,
    catalogRoutingSignal: signal ?? pane1.catalogRoutingSignal,
  };
  return next;
}

async function mountCabDualPickerSyncForSlot(
  dualTabPanes: DualTabPaneConfig[],
  dualCatalogModelId: string,
  meta: PresetMetaJson | null,
  slot: SlotDebug,
  kemplineSlotIndex: number | undefined,
  tabIndex: 0 | 1 = cabDualActiveTab,
): Promise<void> {
  const cab2Hint = probePickerCabDualCab2Hint(kemplineSlotIndex, dualCatalogModelId);
  await armCabDualPickerSync(
    dualTabPanes,
    dualCatalogModelId,
    meta,
    slot,
    kemplineSlotIndex,
    cab2Hint,
  );
  await syncPickerForCabDualTab(tabIndex);
}

/** Cab Amp+Cab connu hors preset dump (collage matrice, probe cab, sync UI). */
function probePickerAmpCabCabHint(
  kemplineSlotIndex: number | undefined,
  ampCatalogModelId: string,
): string | null {
  const ampNorm = ampCatalogModelId.trim().toLowerCase();
  if (kemplineSlotIndex !== undefined && Number.isInteger(kemplineSlotIndex)) {
    const ctx = lastProbePickerAssignContext;
    if (
      ctx &&
      ctx.ki === kemplineSlotIndex &&
      (ctx.assignVariant.trim().toLowerCase() === "amp+cab" ||
        ctx.assignVariant.trim().toLowerCase() === "amp+cab-legacy")
    ) {
      const fromCtx = (ctx.ampCabCabModelId ?? "").trim();
      if (fromCtx) return fromCtx;
    }
    const cb = matrixSlotClipboard;
    if (
      cb &&
      cb.presetIndex === currentPresetIndex &&
      cb.catalogModelId.trim().toLowerCase() === ampNorm &&
      (cb.assignVariant.trim().toLowerCase() === "amp+cab" ||
        cb.assignVariant.trim().toLowerCase() === "amp+cab-legacy")
    ) {
      const fromCb = (cb.ampCabCabModelId ?? "").trim();
      if (fromCb) return fromCb;
    }
    if (
      ampCabDualPickerSync &&
      ampCabDualPickerSync.ampCatalogModelId.trim().toLowerCase() === ampNorm
    ) {
      const fromSync = ampCabDualPickerSync.cabCatalogModelId.trim();
      if (fromSync) return fromSync;
    }
  }
  return null;
}

type AmpCabDualResolve = {
  dualTabPanes: DualTabPaneConfig[] | null;
  linkedCabHex: string | null;
  cabCatalogModelId: string | null;
  assignVariant: string;
};

async function mountAmpCabPickerSyncForSlot(
  ampCabDual: AmpCabDualResolve,
  catalogModelIdTrimmed: string,
  meta: PresetMetaJson | null,
  slot: SlotDebug,
  kemplineSlotIndex: number | undefined,
  tabIndex: 0 | 1 = ampCabDualActiveTab,
): Promise<void> {
  const cabHint = probePickerAmpCabCabHint(kemplineSlotIndex, catalogModelIdTrimmed);
  const cabId = (cabHint ?? ampCabDual.cabCatalogModelId ?? "").trim();
  if (!cabId) return;
  let linkedCabHex = ampCabDual.linkedCabHex;
  if (cabHint) {
    const cabMeta = await getPresetMetaForId(cabHint);
    linkedCabHex =
      (await moduleHexForUsbVariant(cabHint, "single", cabMeta))?.trim() ||
      linkedCabHex;
  }
  const probeSlotCategory =
    kemplineSlotIndex !== undefined &&
    lastProbePickerAssignContext?.ki === kemplineSlotIndex
      ? (lastProbePickerAssignContext.category ?? "").trim()
      : "";
  ampCabDualPickerSync = {
    ampCatalogModelId: catalogModelIdTrimmed,
    meta,
    moduleHex: slot.moduleHex,
    slotCategory: probeSlotCategory || slot.category,
    linkedCabHex,
    cabCatalogModelId: cabId,
  };
  await syncPickerForAmpCabDualTab(tabIndex);
}

function optimisticSlotDuringProbeMerge(ki: number): SlotDebug | null {
  const m = mergeProbeSlotModelUntil;
  if (!m || Date.now() > m.deadline) return null;
  if (m.ki !== ki && !m.extraKis?.includes(ki)) return null;
  const optimistic = lastHwSyncNormalizedSlots?.[ki];
  if (!optimistic || isEmptyGridCell(optimistic)) return null;
  return { ...optimistic };
}

/** Pendant probe matrice / picker : ne pas écraser la ligne optimiste (ex. cab dual → simple hex scroll). */
function shouldSkipHwSlotModelVisualOverwrite(ki: number): boolean {
  if (slotModelUsbProbeInFlight === ki) return true;
  const m = mergeProbeSlotModelUntil;
  if (!m || Date.now() > m.deadline) return false;
  return m.ki === ki || (m.extraKis?.includes(ki) ?? false);
}

/** Catégorie picker pour jonctions Split / Merge (parseur preset peut renvoyer « Routing »). */
function resolveRoutingPickerCategory(
  slotCategory: string,
  slotName: string,
  catalogModelId: string,
  meta: PresetMetaJson | null,
): "Split" | "Merge" | null {
  const nk = normalizeCategory(slotCategory);
  const nn = slotName.trim().toLowerCase();
  if (nk === "split" || nn.includes("split")) return "Split";
  if (nk === "merge" || nn.includes("merge")) return "Merge";
  if (nk === "routing") {
    if (nn.includes("merge")) return "Merge";
    if (nn.includes("split")) return "Split";
  }
  const cn = normalizeCategory(meta?.categoryName ?? "");
  if (cn === "split") return "Split";
  if (cn === "merge") return "Merge";
  const id = catalogModelId.trim();
  if (/^HD2_AppDSPFlowSplit/i.test(id)) return "Split";
  if (id === "HD2_AppDSPFlowJoin") return "Merge";
  return null;
}

// --- Fichiers `src-tauri/resources/models/*.models` (panneau Paramètres Models) ---

type ModelParamDefJson = {
  symbolicID: string;
  name: string;
  /** Ordre des valeurs `read_params` côté firmware (tri croissant dans la chaîne). */
  assign?: number;
  displayType?: string;
  displayType_stereo?: string;
  /** 0 = entier, 1 = float, 2 = bool (Line 6). */
  valueType?: number;
  /** JSON Line 6 : souvent nombres ; bool pour `off_on` (ex. Bright / Contour). */
  min?: number | boolean;
  min_stereo?: number | boolean;
  max?: number | boolean;
  max_stereo?: number | boolean;
  default?: number | string | boolean;
  default_stereo?: number | string | boolean;
  "stereo-only"?: boolean;
};

type ModelDefinitionJson = {
  symbolicID?: string;
  name: string;
  params?: ModelParamDefJson[];
};

const modelsDefinitionsCache = new Map<string, ModelDefinitionJson[]>();
let modelsParamsLoadSeq = 0;

/** Fichiers `.models` cab / IR Line 6, dans l’ordre de recherche (ids souvent dans `cabmicirs` ou `cabmicirswithpan`). */
const CAB_MODEL_DEFINITION_BASES = ["cab", "cabmicirs", "cabmicirswithpan"] as const;

/**
 * Repli si `HX_ModelUsbAssign.json` → `modelsFileByCategory` ne couvre pas la catégorie.
 * Source de vérité : tables en tête du fichier assign (maintenues par sync_usb_assign_from_catalog.py).
 */
function modelsDefinitionFileBasesForCategoryFallback(category: string): string[] {
  const k = normalizeCategory(category);
  const m: Record<string, string[]> = {
    amp: ["amp"],
    preamp: ["preamp"],
    "amp+cab": ["amp", "cab", "preamp"],
    cab: [...CAB_MODEL_DEFINITION_BASES],
    ir: ["fixed", "cabmicirs", "cabmicirswithpan"],
    "impulse response": ["fixed", "cabmicirs", "cabmicirswithpan"],
    delay: ["delay"],
    reverb: ["reverb"],
    dynamics: ["compressor", "gate"],
    dynamic: ["compressor", "gate"],
    eq: ["eq"],
    modulation: ["modulation"],
    distortion: ["distortion"],
    filter: ["filter"],
    wah: ["wah"],
    "pitch/synth": ["pitch-synth"],
    "pitch synth": ["pitch-synth"],
    "volume/pan": ["volumepan"],
    "vol/pan": ["volumepan"],
    "send/return": ["sendreturn"],
    looper: ["fixed"],
    input: ["io"],
    output: ["io"],
    split: ["io"],
    merge: ["io"],
    routing: ["io"],
  };
  return m[k] ?? [];
}

/** Export Line 6 : préfixe avant le tableau JSON (ex. `amp.models` commence par `SVT4[`). */
function stripModelsDefinitionFilePreamble(raw: string): string {
  const s = raw.replace(/^\uFEFF/, "").trimStart();
  const i = s.indexOf("[");
  if (i <= 0) return s;
  return s.slice(i);
}

async function loadModelsDefinitionArray(fileBase: string): Promise<ModelDefinitionJson[]> {
  const hit = modelsDefinitionsCache.get(fileBase);
  if (hit) return hit;
  const url = `/src-tauri/resources/models/${fileBase}.models`;
  const res = await fetch(url);
  let raw: string;
  if (res.ok) {
    raw = await res.text();
  } else {
    raw = await invoke<string>("read_models_definition_file", { fileBase });
  }
  const parsed = JSON.parse(stripModelsDefinitionFilePreamble(raw)) as unknown;
  if (!Array.isArray(parsed)) {
    throw new Error("Format .models invalide (tableau attendu).");
  }
  modelsDefinitionsCache.set(fileBase, parsed as ModelDefinitionJson[]);
  return parsed as ModelDefinitionJson[];
}

/**
 * Charge la définition `.models` pour un slot : **jointure stricte par `symbolicID` = id catalogue**
 * (`chainHex` → id, ou `catalogModelId`). Le **fichier** (`amp` vs `preamp`, etc.) vient du
 * **`presetMeta.categoryName`** de cette entrée catalogue quand il est fourni — pas du nom USB et
 * pas seulement de la catégorie affichée par la grille (peut diverger).
 */
async function findModelDefinitionForSlot(
  slot: SlotDebug,
  catalogModelId?: string | null,
  /** `HX_ModelCatalog.json` → `presetMeta.categoryName` pour l’id joint (ex. « Amp », « Preamp »). */
  catalogPresetCategoryName?: string | null,
): Promise<{ entry: ModelDefinitionJson; fileBase: string } | null> {
  const nameTarget = slot.name.trim();
  if (!nameTarget || nameTarget === "<empty>") return null;
  const idTarget = (catalogModelId ?? "").trim();
  if (!idTarget) return null;
  const categoryForFiles =
    (catalogPresetCategoryName ?? "").trim() || slot.category;
  const bases =
    (await modelsDefinitionFileBasesFromUsbAssign(idTarget, categoryForFiles)) ??
    modelsDefinitionFileBasesForCategoryFallback(categoryForFiles);
  if (bases.length === 0) return null;
  for (const fileBase of bases) {
    let list: ModelDefinitionJson[];
    try {
      list = await loadModelsDefinitionArray(fileBase);
    } catch {
      continue;
    }
    const byId = list.find((e) => (e.symbolicID || "").trim() === idTarget);
    if (byId) return { entry: byId, fileBase };
  }
  if (DEBUG_MODEL_ID_JOIN_FALLBACK) {
    console.warn(
      `[models] Aucun match par ID: id=${idTarget} filesCategory="${categoryForFiles}" slotCategory="${slot.category}" slot="${nameTarget}" tried=${bases.join(",")}`,
    );
  }
  return null;
}

async function findModelDefinitionBySymbolicId(
  symbolicId: string,
  categoryHint: string,
): Promise<{ entry: ModelDefinitionJson; fileBase: string } | null> {
  const idTarget = symbolicId.trim();
  if (!idTarget) return null;
  const bases =
    (await modelsDefinitionFileBasesFromUsbAssign(idTarget, categoryHint)) ??
    modelsDefinitionFileBasesForCategoryFallback(categoryHint);
  if (bases.length === 0) return null;
  for (const fileBase of bases) {
    let list: ModelDefinitionJson[];
    try {
      list = await loadModelsDefinitionArray(fileBase);
    } catch {
      continue;
    }
    const byId = list.find((e) => (e.symbolicID || "").trim() === idTarget);
    if (byId) return { entry: byId, fileBase };
  }
  return null;
}

function formatParamBound(n: number | undefined): string {
  if (n === undefined || !Number.isFinite(n)) return "—";
  const s = String(n);
  if (s.includes("e") || s.includes("E")) return n.toPrecision(4);
  return s;
}

/** Min / max affichés comme la colonne « chaîne » (Helix + `displayType`) quand c’est possible. */
function formatParamBoundForDisplay(
  bound: number | boolean | undefined,
  param: ModelParamDefJson,
  helixControlsMap?: Map<string, HelixControlDefJson>,
): string {
  if (bound === undefined) return "—";
  if (typeof bound === "boolean") {
    const dt = (param.displayType ?? "").trim();
    if (dt && helixControlsMap?.has(dt)) {
      const def = helixControlsMap.get(dt)!;
      return formatHelixFromControl(bound ? 1 : 0, def, dt);
    }
    return bound ? "on" : "off";
  }
  if (!Number.isFinite(bound)) return "—";
  const dt = (param.displayType ?? "").trim();
  if (dt && helixControlsMap?.has(dt)) {
    return formatChainParamValueJson(bound, param, helixControlsMap);
  }
  return formatParamBound(bound);
}

/** Valeurs `ChainParamValue` sérialisées (serde untagged). */
type ChainParamValueJson = boolean | number | string;

function chainParamValuesJsonEqual(
  a: ChainParamValueJson | undefined,
  b: ChainParamValueJson | undefined,
): boolean {
  if (a === b) return true;
  if (a === undefined || b === undefined) return a === b;
  if (typeof a === "number" && typeof b === "number") {
    if (!Number.isFinite(a) || !Number.isFinite(b)) return a === b;
    const d = Math.abs(a - b);
    return d <= 1e-9 * (1 + Math.max(Math.abs(a), Math.abs(b)));
  }
  return a === b;
}

type DualTabPaneConfig = {
  /** Libellé court de l’onglet (ex. « Amp », « Cab »). */
  tabLabel: string;
  /** Nom affiché dans l’en-tête quand l’onglet est actif. */
  modelTitle: string;
  catalogModelId: string | null;
  basedOn: string | null;
  modelImage: string | null;
  params: ModelParamDefJson[];
  chainValues: ChainParamValueJson[] | null;
  catalogRoutingSignal: string | null;
};
type HelixControlFormatBandJson = {
  lowerBound?: number;
  upperBound?: number;
  format?: string;
  formatUnits?: string;
  unitsMultiplier?: number;
  /** Présent sur les segments `step[]` Helix (pas sur `format[]`). */
  fine?: number;
  coarse?: number;
};
type HelixControlDefJson = {
  dspToDisplayScale?: number;
  /**
   * Décalage entier DSP → affichage (ex. `integer_slider_1based` : indices 0…N-1 côté chaîne,
   * libellés 1…N côté UI → offset `1`).
   */
  dspToDisplayIntegerOffset?: number;
  /** `format` dans le JSON : `%.1f`, tableau de plages, ou liste de libellés (`off_on`, `sync_note`, …). */
  format?: string | string[] | HelixControlFormatBandJson[];
  formatUnits?: string;
  unitsMultiplier?: number;
  isDiscrete?: boolean;
  /** `step` Helix : `{ fine, coarse }` ou tableau de plages (même logique que `format`). */
  step?: unknown;
};

/**
 * Exceptions rares : retourner une chaîne non vide pour court-circuiter le pipeline générique.
 * Clé = `displayType` (clé racine dans `HelixControls.json`).
 */
const HELIX_DISPLAY_EXCEPTIONS: Record<
  string,
  (raw: number, def: HelixControlDefJson) => string | null
> = {};

let helixControlsMapPromise: Promise<Map<string, HelixControlDefJson>> | null = null;

function deepCloneHelixControl(def: HelixControlDefJson): HelixControlDefJson {
  try {
    return JSON.parse(JSON.stringify(def)) as HelixControlDefJson;
  } catch {
    return { ...def };
  }
}

/** Parse un objet valeur Helix (sans résolution d’`alias`). */
function parseHelixControlObject(o: Record<string, unknown>): HelixControlDefJson {
  let parsedFormat: string | string[] | HelixControlFormatBandJson[] | undefined;
  if (typeof o.format === "string") {
    parsedFormat = o.format;
  } else if (Array.isArray(o.format) && o.format.length > 0) {
    const allStr = o.format.every((x) => typeof x === "string");
    const allObj = o.format.every((x) => x && typeof x === "object");
    if (allStr) {
      parsedFormat = o.format as string[];
    } else if (allObj) {
      const bands: HelixControlFormatBandJson[] = [];
      for (const it of o.format) {
        if (!it || typeof it !== "object") continue;
        const b = it as {
          lowerBound?: unknown;
          upperBound?: unknown;
          format?: unknown;
          formatUnits?: unknown;
          unitsMultiplier?: unknown;
        };
        bands.push({
          lowerBound:
            typeof b.lowerBound === "number" && Number.isFinite(b.lowerBound)
              ? b.lowerBound
              : undefined,
          upperBound:
            typeof b.upperBound === "number" && Number.isFinite(b.upperBound)
              ? b.upperBound
              : undefined,
          format: typeof b.format === "string" ? b.format : undefined,
          formatUnits: typeof b.formatUnits === "string" ? b.formatUnits : undefined,
          unitsMultiplier:
            typeof b.unitsMultiplier === "number" && Number.isFinite(b.unitsMultiplier)
              ? b.unitsMultiplier
              : undefined,
        });
      }
      parsedFormat = bands;
    }
  }
  return {
    dspToDisplayScale:
      typeof o.dspToDisplayScale === "number" && Number.isFinite(o.dspToDisplayScale)
        ? o.dspToDisplayScale
        : undefined,
    dspToDisplayIntegerOffset:
      typeof o.dspToDisplayIntegerOffset === "number" && Number.isFinite(o.dspToDisplayIntegerOffset)
        ? o.dspToDisplayIntegerOffset
        : undefined,
    format: parsedFormat,
    formatUnits: typeof o.formatUnits === "string" ? o.formatUnits : undefined,
    unitsMultiplier:
      typeof o.unitsMultiplier === "number" && Number.isFinite(o.unitsMultiplier)
        ? o.unitsMultiplier
        : undefined,
    isDiscrete: o.isDiscrete === true,
    step: o.step !== undefined ? o.step : undefined,
  };
}

async function loadHelixControlsMap(): Promise<Map<string, HelixControlDefJson>> {
  const url = "/src-tauri/resources/HelixControls.json";
  const res = await fetch(url);
  if (!res.ok) {
    console.warn("HelixControls.json : chargement impossible.", res.status);
    return new Map();
  }
  const raw = await res.text();
  const data = JSON.parse(raw) as Record<string, unknown>;
  const rawMap = new Map<string, Record<string, unknown>>();
  for (const [k, v] of Object.entries(data)) {
    if (v && typeof v === "object" && !Array.isArray(v)) {
      rawMap.set(k, v as Record<string, unknown>);
    }
  }
  const map = new Map<string, HelixControlDefJson>();
  const stack = new Set<string>();

  function resolveKey(key: string): HelixControlDefJson {
    if (map.has(key)) return map.get(key)!;
    if (stack.has(key)) {
      console.warn(`HelixControls.json : alias cyclique ou manquant autour de "${key}".`);
      return {};
    }
    stack.add(key);
    const o = rawMap.get(key);
    if (!o) {
      stack.delete(key);
      return {};
    }
    const aliasRaw = o.alias;
    let def: HelixControlDefJson;
    if (typeof aliasRaw === "string" && aliasRaw.trim()) {
      def = deepCloneHelixControl(resolveKey(aliasRaw.trim()));
    } else {
      def = parseHelixControlObject(o);
    }
    map.set(key, def);
    stack.delete(key);
    return def;
  }

  for (const key of rawMap.keys()) {
    resolveKey(key);
  }
  return map;
}

async function getHelixControlsMap(): Promise<Map<string, HelixControlDefJson>> {
  if (!helixControlsMapPromise) {
    helixControlsMapPromise = loadHelixControlsMap().catch((e) => {
      helixControlsMapPromise = null;
      throw e;
    });
  }
  return helixControlsMapPromise;
}

function parsePrintfFloatPrecision(format: string): number | null {
  const m = format.match(/^%[+\- 0#]*(?:\.(\d+))?f$/);
  if (!m) return null;
  const n = Number.parseInt(m[1] ?? "6", 10);
  if (!Number.isFinite(n) || n < 0 || n > 12) return null;
  return n;
}

function formatWithPrintfFloat(value: number, format: string): string | null {
  const precision = parsePrintfFloatPrecision(format);
  if (precision === null) return null;
  const s = value.toFixed(precision);
  if (format.includes("+") && value >= 0) return `+${s}`;
  return s;
}

function formatWithPrintfTemplate(
  value: number,
  template: string,
): { numeric: string; rendered: string } | null {
  const token = template.match(HELIX_PRINTF_TOKEN_RE)?.[0];
  if (!token) return null;
  const numeric = formatWithPrintfFloat(value, token);
  if (numeric === null) return null;
  return { numeric, rendered: template.replace(HELIX_PRINTF_TOKEN_RE, numeric) };
}

function pickFormatBandForValue(
  value: number,
  bands: HelixControlFormatBandJson[],
): HelixControlFormatBandJson | null {
  let fallback: HelixControlFormatBandJson | null = null;
  for (let i = 0; i < bands.length; i += 1) {
    const b = bands[i];
    if (i === bands.length - 1) fallback = b;
    const lb = typeof b.lowerBound === "number" ? b.lowerBound : Number.NEGATIVE_INFINITY;
    const ub = typeof b.upperBound === "number" ? b.upperBound : Number.POSITIVE_INFINITY;
    // Bornes semi-ouvertes [lowerBound, upperBound) pour éviter les ambiguïtés à 20/1000.
    if (value >= lb && value < ub) return b;
  }
  return fallback;
}

/** Incrément en unité **brute chaîne** (DSP) pour le snap du slider. */
function helixRawIncrementFromStep(rawValue: number, def: HelixControlDefJson): number | null {
  const st = def.step;
  if (st === undefined || st === null) return null;
  if (typeof st === "object" && !Array.isArray(st) && "fine" in st) {
    const fine = (st as { fine?: unknown }).fine;
    if (typeof fine !== "number" || !Number.isFinite(fine) || fine <= 0) return null;
    const dsp = def.dspToDisplayScale;
    if (typeof dsp === "number" && dsp > 0 && Number.isFinite(dsp)) return fine / dsp;
    return fine;
  }
  if (Array.isArray(st) && st.length > 0 && typeof st[0] === "object") {
    const segs = st as HelixControlFormatBandJson[];
    const band = pickFormatBandForValue(rawValue, segs);
    const fine = band?.fine;
    if (typeof fine === "number" && Number.isFinite(fine) && fine > 0) return fine;
  }
  return null;
}

function fallbackRawIncrement(p: ModelParamDefJson, min: number, max: number): number {
  if (p.valueType === 0) return 1;
  const span = max - min;
  if (!Number.isFinite(span) || span <= 0) return 0.001;
  const coarse = span / 200;
  return Math.max(1e-6, Math.min(span, coarse));
}

function snapRawToIncrement(
  v: number,
  min: number,
  max: number,
  inc: number,
  valueType?: number,
): number {
  if (!Number.isFinite(inc) || inc <= 0) return Math.min(max, Math.max(min, v));
  const n = Math.round((v - min) / inc);
  let s = min + n * inc;
  s = Math.min(max, Math.max(min, s));
  if (valueType === 0) s = Math.round(s);
  return s;
}

const HELIX_PRINTF_TOKEN_RE = /%[+\- 0#]*(?:\.\d+)?f/;

/** `%%` dans les chaînes Helix = un `%` littéral (convention sprintf). */
function helixUnescapePercentMarks(s: string): string {
  return s.replace(/%%/g, "%");
}

/**
 * Pipeline générique `HelixControls.json` pour une valeur numérique de chaîne :
 * liste discrète (`format: ["Off","On"]`), plages (`format: [{ lowerBound, upperBound, … }]`),
 * ou format simple (`dspToDisplayScale` + `format` + `formatUnits` optionnel).
 * Les exceptions métier se greffent sur `HELIX_DISPLAY_EXCEPTIONS`.
 */
function formatHelixFromControl(rawValue: number, control: HelixControlDefJson, displayType: string): string {
  const ex = HELIX_DISPLAY_EXCEPTIONS[displayType];
  if (ex) {
    const s = ex(rawValue, control);
    if (s !== null && s.length > 0) return s;
  }

  const fmt = control.format;

  if (Array.isArray(fmt) && fmt.length > 0 && typeof fmt[0] === "string") {
    const labels = fmt as string[];
    const idx = Math.max(0, Math.min(labels.length - 1, Math.round(rawValue)));
    return labels[idx] ?? "—";
  }

  let format: string | undefined;
  let formatUnits = control.formatUnits;
  let unitsMultiplier = control.unitsMultiplier;

  if (typeof fmt === "string") {
    format = fmt;
  } else if (Array.isArray(fmt) && fmt.length > 0 && typeof fmt[0] === "object") {
    const bands = fmt as HelixControlFormatBandJson[];
    const dspPick = control.dspToDisplayScale;
    const valueForBandPick =
      typeof dspPick === "number" && Number.isFinite(dspPick) ? rawValue * dspPick : rawValue;
    const band = pickFormatBandForValue(valueForBandPick, bands);
    if (band) {
      format = band.format ?? format;
      formatUnits = band.formatUnits ?? formatUnits;
      unitsMultiplier = band.unitsMultiplier ?? unitsMultiplier;
    }
  }

  if (format && !HELIX_PRINTF_TOKEN_RE.test(format)) {
    const lit = (formatUnits ?? format).trim();
    return helixUnescapePercentMarks(lit.length > 0 ? lit : "—");
  }

  let value = rawValue;
  const dsp = control.dspToDisplayScale;
  if (typeof dsp === "number" && Number.isFinite(dsp)) {
    value *= dsp;
  }
  if (typeof unitsMultiplier === "number" && Number.isFinite(unitsMultiplier)) {
    value *= unitsMultiplier;
  }
  const intOff = control.dspToDisplayIntegerOffset;
  if (typeof intOff === "number" && Number.isFinite(intOff)) {
    value += intOff;
  }

  const formatTemplate = format ? formatWithPrintfTemplate(value, format) : null;
  const formattedNumeric = formatTemplate?.numeric ?? null;
  const formattedFromFormat = formatTemplate?.rendered ?? null;
  if (formatUnits) {
    if (HELIX_PRINTF_TOKEN_RE.test(formatUnits)) {
      // Le template `formatUnits` peut porter sa propre précision (ex. `%.1f kHz`)
      // différente de `format` (ex. `%.0f`). On le formate donc directement avec `value`
      // pour éviter une double étape qui arrondit trop tôt (cas 1.6 kHz -> 2 kHz).
      const unitsTemplate = formatWithPrintfTemplate(value, formatUnits);
      if (unitsTemplate?.rendered) {
        return helixUnescapePercentMarks(unitsTemplate.rendered);
      }
      if (formattedNumeric !== null) {
        return helixUnescapePercentMarks(
          formatUnits.replace(HELIX_PRINTF_TOKEN_RE, formattedNumeric),
        );
      }
    }
    return helixUnescapePercentMarks(formatUnits);
  }
  if (formattedFromFormat !== null) return helixUnescapePercentMarks(formattedFromFormat);
  if (control.isDiscrete) {
    return String(Math.round(value));
  }
  const s = String(value);
  if (s.includes("e") || s.includes("E")) return value.toPrecision(4);
  return s;
}

function normalizeCatalogSignal(signal: string | null | undefined): "mono" | "stereo" | null {
  const s = (signal ?? "").trim().toLowerCase();
  if (!s) return null;
  if (s.includes("stereo")) return "stereo";
  if (s.includes("mono")) return "mono";
  return null;
}

function paramForSignalVariant(
  p: ModelParamDefJson,
  catalogSignal: string | null | undefined,
): ModelParamDefJson {
  if (normalizeCatalogSignal(catalogSignal) !== "stereo") return p;
  if (
    p.displayType_stereo === undefined &&
    p.min_stereo === undefined &&
    p.max_stereo === undefined &&
    p.default_stereo === undefined
  ) {
    return p;
  }
  return {
    ...p,
    displayType: p.displayType_stereo ?? p.displayType,
    min: p.min_stereo ?? p.min,
    max: p.max_stereo ?? p.max,
    default: p.default_stereo ?? p.default,
  };
}

function paramsVisibleForSignal(
  params: ModelParamDefJson[],
  catalogSignal: string | null | undefined,
): ModelParamDefJson[] {
  const signal = normalizeCatalogSignal(catalogSignal);
  if (signal !== "mono") return params;
  return params.filter((p) => p["stereo-only"] !== true);
}

/** Masqué en mono : même règle que `paramsVisibleForSignal` (l’index `chainValues` reste celui du `.models` complet). */
function paramHiddenForMonoStereoOnly(
  p: ModelParamDefJson,
  catalogSignal: string | null | undefined,
): boolean {
  return paramsVisibleForSignal([p], catalogSignal).length === 0;
}

/**
 * **Split A/B** (`split_ab_route_to`), **Split Y** (`split_balance`) et **`pan`** avec **min 0 / max 1**
 * (ex. **Mixer** `A Pan` / `B Pan`) : valeur souvent **normalisée 0…1** sur le fil alors que
 * `HelixControls.json` (`pan`) formate en **-100…+100**. Conversion `×200−100` seulement si les bornes
 * `.models` sont bien 0 et 1 et que la valeur est dans ~[0, 1].
 */
function helixNumericInputForSplitNormalized0To1(
  raw: number,
  param: ModelParamDefJson | undefined,
): number {
  const dt = (param?.displayType ?? "").trim();
  const panLike =
    dt === "split_ab_route_to" || dt === "split_balance" || dt === "pan";
  if (!panLike) return raw;
  const minN = param?.min;
  const maxN = param?.max;
  if (typeof minN !== "number" || typeof maxN !== "number" || minN !== 0 || maxN !== 1) {
    return raw;
  }
  if (!Number.isFinite(raw)) return raw;
  if (raw < -0.0001 || raw > 1.0001) return raw;
  return raw * 200 - 100;
}

function formatChainParamValueJson(
  v: ChainParamValueJson,
  param?: ModelParamDefJson,
  helixControlsMap?: Map<string, HelixControlDefJson>,
): string {
  if (typeof v === "boolean") {
    const controlKey = (param?.displayType ?? "").trim();
    if (controlKey && helixControlsMap?.has(controlKey)) {
      const def = helixControlsMap.get(controlKey);
      if (def) {
        return formatHelixFromControl(v ? 1 : 0, def, controlKey);
      }
    }
    return v ? "on" : "off";
  }
  if (typeof v === "number" && Number.isFinite(v)) {
    const controlKey = (param?.displayType ?? "").trim();
    if (controlKey && helixControlsMap?.has(controlKey)) {
      const def = helixControlsMap.get(controlKey);
      if (def) {
        return formatHelixFromControl(
          helixNumericInputForSplitNormalized0To1(v, param),
          def,
          controlKey,
        );
      }
    }
    const s = String(v);
    if (s.includes("e") || s.includes("E")) return v.toPrecision(4);
    return s;
  }
  if (typeof v === "string") {
    const t = v.trim();
    if (t.length > 48) return `${t.slice(0, 44)}…`;
    return t || "—";
  }
  return "—";
}

function formatRawChainParamValueJson(v: ChainParamValueJson): string {
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number" && Number.isFinite(v)) return String(v);
  if (typeof v === "string") return v;
  return "—";
}

/** Infobulle ligne / slider : toujours valeur brute (avant toute conversion d'affichage). */
function paramSliderHoverTitle(
  cv: ChainParamValueJson | undefined,
  _p: ModelParamDefJson,
  _helixControlsMap?: Map<string, HelixControlDefJson>,
): string {
  if (cv === undefined) return "—";
  return formatRawChainParamValueJson(cv);
}

/** `true` / `false` pour bool ; `0` / `1` pour entiers discrets ; sinon pas d’UI bool. */
function chainValueAsBool(cv: ChainParamValueJson | undefined): boolean | null {
  if (cv === undefined) return null;
  if (typeof cv === "boolean") return cv;
  if (typeof cv === "number" && Number.isFinite(cv)) {
    if (cv === 0 || cv === 1) return cv !== 0;
    return null;
  }
  return null;
}

/*
 * Masquage des bandes EQ quand le switch **EQ** est off (`appendModelsParamRows`, ex. **HD2_AmpSVT4Pro** /
 * **HD2_PreampSVT4Pro**) :
 * - Maître : `symbolicID === "EQ"`, `valueType` booléen, `displayType` `off_on` (`modelsEqMasterIndex`).
 * - Suiveurs : entrées après EQ dans `paramsForDisplay` jusqu’au premier `symbolicID` commençant par `@`
 *   (souvent absent du catalogue HX → jusqu’à **EQLevel** inclus).
 * - `li.hidden` + `data-models-eq-band` ; le handler du toggle EQ met à jour `hidden` sur ces `li`.
 *
 * Particularité : `.models-params-row` impose `display: grid` ou `table-row`, ce qui surcharge l’effet
 * navigateur de `[hidden]`. Conserver dans **styles.css** :
 *   `.models-params-list > .models-params-row[hidden] { display: none !important; }`
 */

/** EQ allumé côté affichage : valeur chaîne (bool / 0–1) ou défaut du `.models`. */
function eqSwitchDisplayedOn(cv: ChainParamValueJson | undefined, eqParam: ModelParamDefJson): boolean {
  const b = chainValueAsBool(cv);
  if (b !== null) return b;
  const d = eqParam.default;
  if (typeof d === "boolean") return d;
  return true;
}

function modelsEqMasterIndex(params: ModelParamDefJson[]): number {
  const i = params.findIndex((p) => (p.symbolicID ?? "").trim() === "EQ");
  if (i < 0) return -1;
  const p = params[i];
  const vt = Number(p.valueType);
  if (!Number.isFinite(vt) || vt !== 2) return -1;
  if ((p.displayType ?? "").trim().toLowerCase() !== "off_on") return -1;
  return i;
}

/** Premier index strictement après `EQ` où le `symbolicID` commence par `@`, sinon fin de liste. */
function modelsEqGraphicSectionEndExclusive(params: ModelParamDefJson[], eqIdx: number): number {
  for (let k = eqIdx + 1; k < params.length; k += 1) {
    const sid = (params[k].symbolicID ?? "").trim();
    if (sid.startsWith("@")) return k;
  }
  return params.length;
}

function modelsParamRowIsEqGraphicFollower(params: ModelParamDefJson[], eqIdx: number, rowIndex: number): boolean {
  if (eqIdx < 0 || rowIndex <= eqIdx) return false;
  return rowIndex < modelsEqGraphicSectionEndExclusive(params, eqIdx);
}

function isOffOnDisplayType(displayType: string | undefined): boolean {
  const t = (displayType ?? "").trim().toLowerCase();
  return t === "off_on";
}

function isPolarityDisplayType(displayType: string | undefined): boolean {
  return (displayType ?? "").trim().toLowerCase() === "polarity";
}

function canModelsParamsBoolToggle(p: ModelParamDefJson, cv: ChainParamValueJson | undefined): boolean {
  if (cv === undefined) return false;
  if (chainValueAsBool(cv) === null) return false;
  if (p.valueType === 2) return true;
  if (isOffOnDisplayType(p.displayType)) return true;
  /** `polarity` : Normal / Inverted (`HelixControls.json`), bool ou 0/1 sur fil — pas un slider. */
  if (isPolarityDisplayType(p.displayType)) return true;
  return false;
}

function isMicParam(p: ModelParamDefJson): boolean {
  const sid = (p.symbolicID ?? "").trim();
  return sid === "Mic" || sid === "@mic";
}

function helixStringFormatLabels(def: HelixControlDefJson | undefined): string[] | null {
  const fmt = def?.format;
  if (!Array.isArray(fmt) || fmt.length === 0) return null;
  if (typeof fmt[0] !== "string") return null;
  return fmt as string[];
}

function chainValueAsMicIndex(cv: ChainParamValueJson | undefined): number | null {
  if (cv === undefined) return null;
  if (typeof cv === "number" && Number.isFinite(cv)) return Math.round(cv);
  return null;
}

function canModelsParamsMicCombo(
  p: ModelParamDefJson,
  cv: ChainParamValueJson | undefined,
  helixControlsMap: Map<string, HelixControlDefJson> | undefined,
  minN: number | boolean | undefined,
  maxN: number | boolean | undefined,
): boolean {
  if (!isMicParam(p)) return false;
  if (chainValueAsMicIndex(cv) === null) return false;
  if (typeof minN !== "number" || typeof maxN !== "number") return false;
  if (!Number.isFinite(minN) || !Number.isFinite(maxN)) return false;
  if (maxN < minN) return false;
  const dt = (p.displayType ?? "").trim();
  if (!dt || !helixControlsMap?.has(dt)) return false;
  const labels = helixStringFormatLabels(helixControlsMap.get(dt));
  return labels !== null && labels.length > 0;
}

function chainValueFromParamDefault(p: ModelParamDefJson): ChainParamValueJson | undefined {
  const d = p.default;
  if (typeof d === "boolean" || typeof d === "number" || typeof d === "string") return d;
  return undefined;
}

/** Valeurs chaîne « comme device » dérivées des défauts `.models` (ordre source mono/stéréo aligné sur `alignChainValuesToModelParamOrder`). */
function buildDefaultChainValuesForSourceOrder(
  allModelParams: ModelParamDefJson[],
  catalogSignal: string | null | undefined,
): ChainParamValueJson[] {
  const signal = normalizeCatalogSignal(catalogSignal);
  const buildSourceOrderIdsFromModels = (includeStereoOnly: boolean): string[] => {
    const out: string[] = [];
    for (let i = 0; i < allModelParams.length; i += 1) {
      const p = allModelParams[i];
      if (!includeStereoOnly && p["stereo-only"] === true) continue;
      const sid = (p.symbolicID ?? "").trim();
      if (!sid) continue;
      out.push(sid);
    }
    return out;
  };
  const sourceAll = buildSourceOrderIdsFromModels(true);
  const sourceMono = buildSourceOrderIdsFromModels(false);
  let source = sourceAll;
  if (signal === "mono") {
    const diffAll = Math.abs(sourceAll.length);
    const diffMono = Math.abs(sourceMono.length);
    if (diffMono < diffAll) source = sourceMono;
  }
  const byId = new Map(
    allModelParams
      .map((p) => [(p.symbolicID ?? "").trim(), p] as const)
      .filter(([k]) => Boolean(k)),
  );
  const out: ChainParamValueJson[] = [];
  for (const sid of source) {
    const p = byId.get(sid);
    let v: ChainParamValueJson | undefined = p ? chainValueFromParamDefault(p) : undefined;
    if (v === undefined && p) {
      if (typeof p.min === "number" && Number.isFinite(p.min)) v = p.min;
      else if (typeof p.default === "boolean") v = p.default;
      else v = 0;
    } else if (v === undefined) {
      v = 0;
    }
    out.push(v);
  }
  return out;
}

/**
 * Le binaire preset aligne les valeurs sur l'ordre DSP (`assign` croissant), puis les champs sans
 * `assign` dans l'ordre où ils apparaissent dans le JSON. Le panneau UI zippe par indice dans
 * `params[]` : sans cette étape, ampli / préampli (ex. Ch Vol vs Master) et d'autres blocs peuvent
 * afficher la mauvaise valeur sur chaque ligne.
 */
function alignChainValuesToModelParamOrder(
  chainValues: ChainParamValueJson[] | null | undefined,
  paramsForDisplay: ModelParamDefJson[],
  allModelParams: ModelParamDefJson[],
  catalogSignal?: string | null,
): Array<ChainParamValueJson | undefined> | null | undefined {
  if (chainValues == null) return chainValues;
  const signal = normalizeCatalogSignal(catalogSignal);

  const stereoOnlyById = new Map<string, boolean>();
  for (const p of allModelParams) {
    const sid = (p.symbolicID ?? "").trim();
    if (!sid || stereoOnlyById.has(sid)) continue;
    stereoOnlyById.set(sid, p["stereo-only"] === true);
  }

  const buildSourceOrderIdsFromModels = (includeStereoOnly: boolean): string[] => {
    const out: string[] = [];
    for (let i = 0; i < allModelParams.length; i += 1) {
      const p = allModelParams[i];
      if (!includeStereoOnly && p["stereo-only"] === true) continue;
      const sid = (p.symbolicID ?? "").trim();
      if (!sid) continue;
      out.push(sid);
    }
    return out;
  };

  const fullAll = buildSourceOrderIdsFromModels(true);
  const fullMono = buildSourceOrderIdsFromModels(false);
  const sourceAll = fullAll;

  let source = sourceAll;
  if (signal === "mono") {
    const sourceMono = fullMono;
    const diffAll = Math.abs(sourceAll.length - chainValues.length);
    const diffMono = Math.abs(sourceMono.length - chainValues.length);
    if (diffMono < diffAll) source = sourceMono;
  }

  const valueBySymbolicId = new Map<string, ChainParamValueJson>();
  const n = Math.min(chainValues.length, source.length);
  for (let i = 0; i < n; i += 1) {
    const sid = source[i];
    if (!sid || valueBySymbolicId.has(sid)) continue;
    valueBySymbolicId.set(sid, chainValues[i]);
  }
  const out: Array<ChainParamValueJson | undefined> = new Array(paramsForDisplay.length);
  for (let i = 0; i < paramsForDisplay.length; i += 1) {
    const sid = (paramsForDisplay[i].symbolicID ?? "").trim();
    if (!sid) continue;
    if (valueBySymbolicId.has(sid)) {
      out[i] = valueBySymbolicId.get(sid);
    }
  }
  return out;
}

type ModelsComboItem = { value: string; label: string };

function labelForComboValue(items: ModelsComboItem[], value: string): string {
  return items.find((i) => i.value === value)?.label ?? value;
}

type ModelsComboHandle = {
  trigger: HTMLButtonElement;
  /** Met à jour le libellé du bouton et `aria-selected` sur les options (ex. après clamp). */
  syncSelection(value: string): void;
};

/**
 * Liste déroulante scrollable (~5 lignes visibles) : remplace un `<select>` natif
 * dont la hauteur du popup ne peut pas être contrôlée par le CSS.
 */
function mountModelsCombo(
  parent: HTMLElement,
  items: ModelsComboItem[],
  selectedValue: string,
  onSelect: (value: string) => void,
  ariaLabel: string,
): ModelsComboHandle {
  const sortedItems = [...items].sort((a, b) =>
    a.label.localeCompare(b.label, undefined, { sensitivity: "base", numeric: true }),
  );
  const wrap = document.createElement("div");
  wrap.className = "models-params-combo-wrap";
  const trigger = document.createElement("button");
  trigger.type = "button";
  trigger.className = "models-params-combo-trigger";
  trigger.setAttribute("aria-label", ariaLabel);
  trigger.setAttribute("aria-haspopup", "listbox");
  trigger.setAttribute("aria-expanded", "false");

  const panel = document.createElement("div");
  panel.className = "models-params-combo-panel";
  panel.hidden = true;
  panel.setAttribute("role", "listbox");

  for (const it of sortedItems) {
    const opt = document.createElement("button");
    opt.type = "button";
    opt.className = "models-params-combo-option";
    opt.setAttribute("role", "option");
    opt.dataset.value = it.value;
    opt.textContent = it.label;
    panel.appendChild(opt);
  }

  function syncSelection(value: string): void {
    trigger.textContent =
      labelForComboValue(sortedItems, value) || sortedItems[0]?.label || "—";
    for (const el of panel.querySelectorAll<HTMLElement>(".models-params-combo-option")) {
      el.setAttribute("aria-selected", el.dataset.value === value ? "true" : "false");
    }
  }

  syncSelection(selectedValue);

  let outsideAc: AbortController | null = null;

  function closePanel(): void {
    if (panel.hidden) return;
    panel.hidden = true;
    trigger.setAttribute("aria-expanded", "false");
    outsideAc?.abort();
    outsideAc = null;
  }

  function openPanel(): void {
    panel.hidden = false;
    trigger.setAttribute("aria-expanded", "true");
    outsideAc = new AbortController();
    const { signal } = outsideAc;
    document.addEventListener(
      "pointerdown",
      (e) => {
        if (!wrap.contains(e.target as Node)) closePanel();
      },
      { capture: true, signal },
    );
    document.addEventListener(
      "keydown",
      (ev: KeyboardEvent) => {
        if (ev.key === "Escape") closePanel();
      },
      { capture: true, signal },
    );
    requestAnimationFrame(() => {
      panel
        .querySelector<HTMLElement>('.models-params-combo-option[aria-selected="true"]')
        ?.scrollIntoView({ block: "nearest" });
    });
  }

  trigger.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (panel.hidden) openPanel();
    else closePanel();
  });

  panel.addEventListener("click", (e) => {
    const t = (e.target as HTMLElement).closest(".models-params-combo-option");
    if (!(t instanceof HTMLElement)) return;
    const v = t.dataset.value;
    if (v === undefined) return;
    e.stopPropagation();
    closePanel();
    onSelect(v);
  });

  wrap.append(trigger, panel);
  parent.appendChild(wrap);
  return { trigger, syncSelection };
}

function appendModelsParamRows(
  list: HTMLUListElement,
  params: ModelParamDefJson[],
  chainValues: Array<ChainParamValueJson | undefined> | null | undefined,
  helixControlsMap?: Map<string, HelixControlDefJson>,
  catalogSignal?: string | null,
  ariaScopeLabel = "",
  liveWriteSlotIndex?: number,
  liveWriteParamIndexBase = 0,
  liveWriteDualPart: "amp" | "cab" | "cab1" | "cab2" | null = null,
  liveWriteAmpCabAssignVariant: string | null = null,
  liveWriteCabDualAssignVariant: string | null = null,
  liveWriteAmpCabAmpParamCount: number | null = null,
  liveWriteWireLocalSelector = false,
): (nextChainValues: Array<ChainParamValueJson | undefined> | null | undefined) => void {
  const eqIdx = modelsEqMasterIndex(params);
  const eqOn =
    eqIdx >= 0 ? eqSwitchDisplayedOn(chainValues?.[eqIdx], params[eqIdx]) : true;
  const rowValueUpdaters: Array<(v: ChainParamValueJson | undefined) => void> = [];
  for (let j = 0; j < params.length; j += 1) {
    const pRaw = params[j];
    if (paramHiddenForMonoStereoOnly(pRaw, catalogSignal)) continue;
    const p = paramForSignalVariant(pRaw, catalogSignal);
    const li = document.createElement("li");
    li.className = "models-params-row";
    const isEqGraphicFollower = modelsParamRowIsEqGraphicFollower(params, eqIdx, j);
    if (isEqGraphicFollower) {
      li.dataset.modelsEqBand = "1";
      // Voir docblock au-dessus de `eqSwitchDisplayedOn` : `hidden` exige le correctif CSS sur `.models-params-row[hidden]`.
      li.hidden = !eqOn;
    }
    const label = document.createElement("span");
    label.className = "models-params-row-name";
    label.textContent = (p.name || p.symbolicID || "").trim() || "—";
    const minEl = document.createElement("span");
    minEl.className = "models-params-row-min";
    minEl.textContent = formatParamBoundForDisplay(p.min, p, helixControlsMap);
    const chainEl = document.createElement("span");
    chainEl.className = "models-params-row-chain";
    const cv = chainValues?.[j];
    chainEl.textContent =
      cv !== undefined ? formatChainParamValueJson(cv, p, helixControlsMap) : "—";
    const maxEl = document.createElement("span");
    maxEl.className = "models-params-row-max";
    maxEl.textContent = formatParamBoundForDisplay(p.max, p, helixControlsMap);
    const sliderCell = document.createElement("div");
    sliderCell.className = "models-params-slider-cell";
    const hoverTitleStr = paramSliderHoverTitle(cv, p, helixControlsMap);
    li.title = hoverTitleStr;
    sliderCell.title = hoverTitleStr;
    li.append(label, minEl, sliderCell, maxEl);

    const minN = p.min;
    const maxN = p.max;
    const micCombo = canModelsParamsMicCombo(p, cv, helixControlsMap, minN, maxN);
    const canSlider =
      !micCombo &&
      typeof cv === "number" &&
      Number.isFinite(cv) &&
      typeof minN === "number" &&
      typeof maxN === "number" &&
      Number.isFinite(minN) &&
      Number.isFinite(maxN) &&
      maxN > minN;
    if (micCombo) {
      minEl.textContent = "—";
      maxEl.textContent = "—";
      sliderCell.classList.add("models-params-slider-cell--combo-only");
      const dt = (p.displayType ?? "").trim();
      const helixDef = helixControlsMap!.get(dt)!;
      const labels = helixStringFormatLabels(helixDef)!;
      const minI = Math.round(minN as number);
      const maxI = Math.round(maxN as number);
      let current = chainValueAsMicIndex(cv)!;
      current = Math.max(minI, Math.min(maxI, current));
      const micItems: ModelsComboItem[] = [];
      for (let i = minI; i <= maxI; i += 1) {
        micItems.push({ value: String(i), label: labels[i] ?? `Mic ${i}` });
      }
      const micAria = `${(p.name || p.symbolicID || "").trim()} (menu déroulant) — aperçu local${ariaScopeLabel}, non envoyé au Helix`;
      const { trigger: micTrigger, syncSelection: syncMicCombo } = mountModelsCombo(
        sliderCell,
        micItems,
        String(current),
        (raw) => {
          const v = Number.parseInt(raw, 10);
          if (!Number.isFinite(v)) return;
          const clamped = Math.max(minI, Math.min(maxI, v));
          const s = formatRawChainParamValueJson(clamped);
          li.title = s;
          sliderCell.title = s;
          micTrigger.title = s;
          syncMicCombo(String(clamped));
          if (liveWriteQueueEnabled()) {
            markLiveWriteUiInteraction();
          }
          const writeParamIndex = liveWriteParamIndexForRow(
            params,
            j,
            catalogSignal,
            liveWriteParamIndexBase,
            liveWriteWireLocalSelector,
          );
          scheduleLiveParamWriteProbe(
            liveWriteSlotIndex,
            writeParamIndex,
            p,
            clamped,
            liveWriteDualPart,
            liveWriteAmpCabAssignVariant,
            liveWriteCabDualAssignVariant,
            liveWriteAmpCabAmpParamCount,
          );
        },
        micAria,
      );
      micTrigger.title = formatRawChainParamValueJson(current);
      rowValueUpdaters[j] = (nextCv) => {
        if (typeof nextCv !== "number" || !Number.isFinite(nextCv)) return;
        const nextI = Math.max(minI, Math.min(maxI, Math.round(nextCv)));
        const raw = formatRawChainParamValueJson(nextI);
        chainEl.textContent = formatChainParamValueJson(nextI, p, helixControlsMap);
        li.title = raw;
        sliderCell.title = raw;
        micTrigger.title = raw;
        syncMicCombo(String(nextI));
      };
    } else {
      if (canSlider) {
      sliderCell.append(chainEl);
      const dt = (p.displayType ?? "").trim();
      const helixDef =
        dt && helixControlsMap?.has(dt) ? helixControlsMap.get(dt)! : undefined;
      let inc = helixDef ? helixRawIncrementFromStep(cv, helixDef) : null;
      if (inc === null || !Number.isFinite(inc) || inc <= 0) {
        inc = fallbackRawIncrement(p, minN, maxN);
      }
      if (p.valueType === 0) {
        inc = Math.max(1, Math.round(inc));
      }
      if (
        (dt === "split_ab_route_to" || dt === "split_balance" || dt === "pan") &&
        minN === 0 &&
        maxN === 1
      ) {
        inc = 0.01;
      }
      const init = snapRawToIncrement(cv, minN, maxN, inc, p.valueType);
      const input = document.createElement("input");
      input.type = "range";
      input.className = "models-params-slider";
      if (p.valueType !== 0) {
        input.classList.add("models-params-slider--filled");
      }
      input.min = String(minN);
      input.max = String(maxN);
      if (inc >= 1e-9) {
        const span = maxN - minN;
        if (inc < span / 2) input.step = String(inc);
      }
      input.value = String(init);
      {
        let v = Number(input.value);
        if (!Number.isFinite(v)) v = init;
        v = snapRawToIncrement(v, minN, maxN, inc, p.valueType);
        if (Number(input.value) !== v) input.value = String(v);
        if (p.valueType !== 0) {
          setSliderFillVisual(input, v, minN, maxN);
        }
      }
      input.title = hoverTitleStr;
      input.setAttribute(
        "aria-label",
        `${(p.name || p.symbolicID || "").trim()} — aperçu local${ariaScopeLabel}, non envoyé au Helix`,
      );
      input.addEventListener("input", () => {
        let v = Number(input.value);
        if (!Number.isFinite(v)) return;
        v = snapRawToIncrement(v, minN, maxN, inc, p.valueType);
        if (Number(input.value) !== v) input.value = String(v);
        if (p.valueType !== 0) {
          setSliderFillVisual(input, v, minN, maxN);
        }
        chainEl.textContent = formatChainParamValueJson(v, p, helixControlsMap);
        const s = paramSliderHoverTitle(v, p, helixControlsMap);
        li.title = s;
        sliderCell.title = s;
        input.title = s;
        if (liveWriteQueueEnabled()) {
          markLiveWriteUiInteraction();
        }
        const writeParamIndex = liveWriteParamIndexForRow(
          params,
          j,
          catalogSignal,
          liveWriteParamIndexBase,
          liveWriteWireLocalSelector,
        );
        scheduleLiveParamWriteProbe(
          liveWriteSlotIndex,
          writeParamIndex,
          p,
          v,
          liveWriteDualPart,
          liveWriteAmpCabAssignVariant,
          liveWriteCabDualAssignVariant,
          liveWriteAmpCabAmpParamCount,
        );
      });
      sliderCell.append(input);
      const tickCount = discreteSliderTickCount(p.valueType, minN, maxN);
      if (tickCount !== null) {
        const ticks = document.createElement("div");
        ticks.className = "models-params-slider-ticks";
        for (let i = 0; i < tickCount; i += 1) {
          const tick = document.createElement("span");
          tick.className = "models-params-slider-tick";
          tick.style.left = `${(i * 100) / (tickCount - 1)}%`;
          ticks.appendChild(tick);
        }
        sliderCell.append(ticks);
      }
      rowValueUpdaters[j] = (nextCv) => {
        if (typeof nextCv !== "number" || !Number.isFinite(nextCv)) return;
        let v = snapRawToIncrement(nextCv, minN, maxN, inc, p.valueType);
        if (!Number.isFinite(v)) return;
        if (Number(input.value) !== v) input.value = String(v);
        if (p.valueType !== 0) {
          setSliderFillVisual(input, v, minN, maxN);
        }
        chainEl.textContent = formatChainParamValueJson(v, p, helixControlsMap);
        const s = paramSliderHoverTitle(v, p, helixControlsMap);
        li.title = s;
        sliderCell.title = s;
        input.title = s;
      };
      } else if (cv !== undefined && canModelsParamsBoolToggle(p, cv)) {
      sliderCell.append(chainEl);
      let currentB = chainValueAsBool(cv)!;
      const input = document.createElement("input");
      input.type = "range";
      input.className = "models-params-slider";
      input.min = "0";
      input.max = "1";
      input.step = "1";
      input.value = currentB ? "1" : "0";
      input.title = hoverTitleStr;
      input.setAttribute(
        "aria-label",
        `${(p.name || p.symbolicID || "").trim()} — aperçu local${ariaScopeLabel}, non envoyé au Helix`,
      );
      const isEqMaster = (p.symbolicID ?? "").trim() === "EQ";
      /** Sync hardware / in-place uniquement : ne pas ré-enfiler de write USB. */
      const applyBool = (nextB: boolean, syncInput = true): void => {
        currentB = nextB;
        const v: ChainParamValueJson = typeof cv === "boolean" ? nextB : nextB ? 1 : 0;
        if (syncInput) input.value = nextB ? "1" : "0";
        chainEl.textContent = formatChainParamValueJson(v, p, helixControlsMap);
        const s = paramSliderHoverTitle(v, p, helixControlsMap);
        li.title = s;
        sliderCell.title = s;
        input.title = s;
        // Même contrainte CSS que pour `li.hidden` à l’init (docblock `eqSwitchDisplayedOn`).
        if (isEqMaster) {
          for (const node of list.querySelectorAll("li[data-models-eq-band]")) {
            if (node instanceof HTMLLIElement) node.hidden = !nextB;
          }
        }
      };
      /** Write live seulement depuis le geste utilisateur (comme les sliders float). */
      const applyBoolFromUserInput = (nextB: boolean): void => {
        applyBool(nextB, false);
        if (liveWriteQueueEnabled()) {
          markLiveWriteUiInteraction();
        }
        const writeParamIndex = liveWriteParamIndexForRow(
          params,
          j,
          catalogSignal,
          liveWriteParamIndexBase,
          liveWriteWireLocalSelector,
        );
        scheduleLiveParamWriteProbe(
          liveWriteSlotIndex,
          writeParamIndex,
          p,
          nextB ? 1 : 0,
          liveWriteDualPart,
          liveWriteAmpCabAssignVariant,
          liveWriteCabDualAssignVariant,
          liveWriteAmpCabAmpParamCount,
        );
      };
      input.addEventListener("input", () => {
        const nextB = Number(input.value) >= 0.5;
        if (nextB === currentB) return;
        applyBoolFromUserInput(nextB);
      });
      sliderCell.append(input);
      rowValueUpdaters[j] = (nextCv) => {
        if (!canModelsParamsBoolToggle(p, nextCv)) return;
        applyBool(chainValueAsBool(nextCv)!, true);
        if (isEqMaster) {
          for (const node of list.querySelectorAll("li[data-models-eq-band]")) {
            if (node instanceof HTMLLIElement) node.hidden = !currentB;
          }
        }
      };
      } else {
      sliderCell.append(chainEl);
      rowValueUpdaters[j] = (nextCv) => {
        chainEl.textContent =
          nextCv !== undefined ? formatChainParamValueJson(nextCv, p, helixControlsMap) : "—";
        const s = paramSliderHoverTitle(nextCv, p, helixControlsMap);
        li.title = s;
        sliderCell.title = s;
      };
      }
    }
    list.appendChild(li);
  }
  let previousAligned: Array<ChainParamValueJson | undefined> | undefined;
  return (nextChainValues) => {
    if (nextChainValues == null) {
      previousAligned = undefined;
      for (let i = 0; i < rowValueUpdaters.length; i += 1) {
        rowValueUpdaters[i]?.(undefined);
      }
      return;
    }
    const isFirst = previousAligned === undefined;
    for (let i = 0; i < rowValueUpdaters.length; i += 1) {
      const updater = rowValueUpdaters[i];
      if (!updater) continue;
      const next = nextChainValues[i];
      if (!isFirst && chainParamValuesJsonEqual(previousAligned![i], next)) continue;
      updater(next);
    }
    previousAligned = nextChainValues.slice() as Array<ChainParamValueJson | undefined>;
  };
}

function applyModelsParamsDualHeader(slot: SlotDebug, pane: DualTabPaneConfig): void {
  const subhead = getModelsParamsSubheadEl();
  if (subhead) {
    subhead.replaceChildren();
    const head = document.createElement("div");
    head.className = "models-params-model-head";
    const title = document.createElement("h2");
    title.className = "models-params-model-title";
    title.textContent = pane.modelTitle.trim() || "—";
    head.appendChild(title);
    subhead.appendChild(head);
  }
  const bo = (pane.basedOn ?? "").trim();
  const basedOnEl = getModelsParamsBasedOnEl();
  if (basedOnEl) {
    basedOnEl.replaceChildren();
    basedOnEl.title = bo;
    if (bo) {
      const label = document.createElement("span");
      label.className = "models-params-pane-basedon-label";
      label.textContent = "Based on : ";
      const value = document.createElement("span");
      value.className = "models-params-pane-basedon-value";
      value.textContent = bo;
      basedOnEl.append(label, value);
    }
  }
  setModelsParamsHeaderIcon(slot, pane.modelImage);
}

function renderModelsParamsDualTabs(
  inner: HTMLElement,
  tabPanes: DualTabPaneConfig[],
  helixControlsMap: Map<string, HelixControlDefJson> | undefined,
  kemplineSlotIndex: number | undefined,
  slot: SlotDebug,
  dualSlotKind: "amp_cab" | "cab_dual",
  ampCabAssignVariant: string | null = null,
  cabDualAssignVariant: string | null = null,
): (rawPrimaryChainValues: ChainParamValueJson[] | null) => void {
  const wrap = document.createElement("div");
  wrap.className = "models-params-dual-tabs";
  const bar = document.createElement("div");
  bar.className = "models-params-dual-tab-bar";
  bar.setAttribute("role", "tablist");
  const panelsWrap = document.createElement("div");
  panelsWrap.className = "models-params-dual-tab-panels";

  const tabs: HTMLButtonElement[] = [];
  const panels: HTMLElement[] = [];
  const updaters: Array<((v: ChainParamValueJson[] | null) => void) | null> = [];

  const initialActiveTab =
    dualSlotKind === "amp_cab"
      ? ampCabDualActiveTab
      : dualSlotKind === "cab_dual"
        ? cabDualActiveTab
        : (0 as 0 | 1);

  applyModelsParamsDualHeader(slot, tabPanes[initialActiveTab]);

  const ampCabAmpParamCount =
    dualSlotKind === "amp_cab"
      ? paramsVisibleForSignal(tabPanes[0]!.params, tabPanes[0]!.catalogRoutingSignal).length
      : 0;

  tabPanes.forEach((pane, idx) => {
    const tab = document.createElement("button");
    tab.type = "button";
    tab.className = "models-params-dual-tab";
    if (idx === initialActiveTab) tab.classList.add("is-active");
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", idx === initialActiveTab ? "true" : "false");
    tab.id = `models-params-dual-tab-${idx}`;
    tab.textContent = pane.tabLabel.trim() || "—";
    tab.title = pane.modelTitle.trim() || pane.tabLabel;

    const panel = document.createElement("div");
    panel.className = "models-params-dual-tab-panel";
    panel.setAttribute("role", "tabpanel");
    panel.setAttribute("aria-labelledby", tab.id);
    panel.hidden = idx !== initialActiveTab;

    const list = document.createElement("ul");
    list.className = "models-params-list";
    const chainAligned = mergeLiveChainOverridesIntoAligned(
      currentPresetIndex,
      kemplineSlotIndex,
      pane.params,
      alignChainValuesToModelParamOrder(
        pane.chainValues,
        pane.params,
        pane.params,
        pane.catalogRoutingSignal,
      ),
    );
    const dualPart: "amp" | "cab" | "cab1" | "cab2" | null =
      dualSlotKind === "amp_cab"
        ? idx === 1
          ? "cab"
          : "amp"
        : dualSlotKind === "cab_dual"
          ? idx === 0
            ? "cab1"
            : "cab2"
          : null;
    const paramIndexBase = 0;
    // Cab dual LEGACY : sélecteur wire-local (discret/float 0-based séparés), comme le single legacy.
    // Le dual modern garde l'index global. Aligné sur le routage Rust (HX_DUAL_LEGACY_STD_PARAM).
    const cabDualLegacyWireLocal =
      dualSlotKind === "cab_dual" &&
      (cabDualAssignVariant ?? "").trim().toLowerCase() === "dual-legacy";
    const updater = appendModelsParamRows(
      list,
      pane.params,
      chainAligned ?? null,
      helixControlsMap,
      pane.catalogRoutingSignal,
      "",
      kemplineSlotIndex,
      paramIndexBase,
      dualPart,
      dualSlotKind === "amp_cab" ? ampCabAssignVariant : null,
      dualSlotKind === "cab_dual" ? cabDualAssignVariant : null,
      dualSlotKind === "amp_cab" && dualPart === "cab" ? ampCabAmpParamCount : null,
      cabDualLegacyWireLocal,
    );
    updaters.push(updater);
    panel.appendChild(list);
    tabs.push(tab);
    panels.push(panel);
    bar.appendChild(tab);
    panelsWrap.appendChild(panel);

    tab.addEventListener("click", () => {
      tabs.forEach((t, i) => {
        const on = i === idx;
        t.classList.toggle("is-active", on);
        t.setAttribute("aria-selected", on ? "true" : "false");
        panels[i].hidden = !on;
      });
      applyModelsParamsDualHeader(slot, tabPanes[idx]);
      if (
        selectedParamsPresetIndex === currentPresetIndex &&
        selectedParamsKemplineSlotIndex !== null &&
        selectedParamsKemplineSlotIndex === kemplineSlotIndex
      ) {
        const hwCtx = dualPaneHwWireContext(dualSlotKind, idx, tabPanes);
        selectedParamsHwWireContext = hwCtx;
      }
      if (dualSlotKind === "amp_cab") {
        ampCabDualActiveTab = idx as 0 | 1;
        void syncPickerForAmpCabDualTab(idx as 0 | 1);
        if (idx === 1 && kemplineSlotIndex !== undefined) {
          void invoke("focus_amp_cab_usb_part", {
            slotIndex: kemplineSlotIndex,
            part: "cab",
            ampCabAssignVariant,
          });
        }
      } else if (dualSlotKind === "cab_dual") {
        cabDualActiveTab = idx as 0 | 1;
        void (async () => {
          await ensureCabDualPickerSynced(idx as 0 | 1);
          if (kemplineSlotIndex !== undefined) {
            void invoke("focus_cab_dual_usb_part", {
              slotIndex: kemplineSlotIndex,
              part: idx === 1 ? "cab2" : "cab1",
            });
          }
        })();
      }
    });
  });

  wrap.append(bar, panelsWrap);
  inner.appendChild(wrap);

  if (
    dualSlotKind === "amp_cab" &&
    initialActiveTab === 1 &&
    kemplineSlotIndex !== undefined
  ) {
    void invoke("focus_amp_cab_usb_part", {
      slotIndex: kemplineSlotIndex,
      part: "cab",
      ampCabAssignVariant,
    });
  }

  if (
    dualSlotKind === "cab_dual" &&
    kemplineSlotIndex !== undefined
  ) {
    void invoke("focus_cab_dual_usb_part", {
      slotIndex: kemplineSlotIndex,
      part: initialActiveTab === 1 ? "cab2" : "cab1",
    });
  }

  return (rawPrimaryChainValues) => {
    const primaryUpdater = updaters[0];
    const primaryPane = tabPanes[0];
    if (!primaryUpdater || !primaryPane) return;
    const aligned = mergeLiveChainOverridesIntoAligned(
      currentPresetIndex,
      kemplineSlotIndex,
      primaryPane.params,
      alignChainValuesToModelParamOrder(
        rawPrimaryChainValues,
        primaryPane.params,
        primaryPane.params,
        primaryPane.catalogRoutingSignal,
      ),
    );
    primaryUpdater((aligned ?? null) as ChainParamValueJson[] | null);
  };
}

async function buildDualTabPanesFromParts(
  dualParts: DualSlotPartsJson,
  dualCatalogModelId?: string | null,
): Promise<DualTabPaneConfig[] | null> {
  if (dualParts.parts.length !== 2) return null;
  const paneDefs: DualTabPaneConfig[] = [];
  for (let i = 0; i < dualParts.parts.length; i += 1) {
    const part = dualParts.parts[i];
    const catHint =
      part.category.trim() ||
      (dualParts.kind === "amp_cab" ? "Amp" : "Cab");
    const hx = part.chainHex.trim();
    let modelId = part.modelId.trim();
    if (dualParts.kind === "cab_dual" && i === 1 && hx) {
      const dualId = (dualCatalogModelId ?? "").trim();
      if (dualId) {
        const fromDualCtx = await getCatalogModelIdForCabDualCab2Hex(
          dualId,
          hx,
          dualParts.parts[0]?.chainHex,
        );
        if (fromDualCtx) modelId = fromDualCtx;
      }
    }
    if (!modelId && hx) {
      modelId =
        dualParts.kind === "cab_dual"
          ? (await getCatalogModelIdForCabSingleHex(hx))?.trim() ?? ""
          : (await getCatalogModelIdForHex(hx, catHint))?.trim() ?? "";
    }
    const def = modelId
      ? await findModelDefinitionBySymbolicId(modelId, catHint)
      : null;
    const partMeta = modelId ? await getPresetMetaForId(modelId) : null;
    const modelImage = modelId ? await getCatalogModelImageForId(modelId) : null;
    const tabLabel =
      dualParts.kind === "amp_cab"
        ? i === 0
          ? "Amp"
          : "Cab"
        : i === 0
          ? "Cab 1"
          : "Cab 2";
    paneDefs.push({
      tabLabel,
      modelTitle: part.name.trim() || def?.entry.name.trim() || "—",
      catalogModelId: modelId || null,
      basedOn: pickBasedOn(partMeta),
      modelImage,
      params: def?.entry.params ?? [],
      chainValues: part.values ?? null,
      catalogRoutingSignal: pickSignal(partMeta, part.chainHex),
    });
  }
  return paneDefs.length === 2 ? paneDefs : null;
}

/** Suffixe Cab 2 usine sur fil dual (`cd02d6` = Jazz Rivet sur Soup Pro Ellipse). */
const CAB_DUAL_FACTORY_CAB2_SUFFIX = "cd02d6";

/**
 * Cab 2 affiché : fil `c319` scroll d’abord ; `cabHexHint` (`c219`) seulement si le fil a encore le suffixe usine.
 */
function cabDualEffectiveCab2Hex(
  wireCab2: string,
  hint?: string | null,
): string {
  const wire = wireCab2.trim().toLowerCase();
  const h = (hint ?? "").trim().toLowerCase();
  if (wire) {
    if (
      h &&
      h !== wire &&
      wire === CAB_DUAL_FACTORY_CAB2_SUFFIX &&
      h !== CAB_DUAL_FACTORY_CAB2_SUFFIX
    ) {
      return h;
    }
    return wire;
  }
  return h;
}

/** Cab 2 depuis trame scroll : fil `c319` puis hint `c219` si suffixe usine encore sur le fil. */
function cabDualCab2HexFromSlotTrame(slot: SlotDebug): string {
  const wireCab2 = cabDualWireParts(slot.moduleHex)?.cab2Hex ?? "";
  return cabDualEffectiveCab2Hex(wireCab2, slot.cabHexHint);
}

async function resolveCabDualCab2HexFromTrame(
  slot: SlotDebug,
  _kemplineSlotIndex?: number,
): Promise<string> {
  const wireCab2 = cabDualWireParts(slot.moduleHex)?.cab2Hex ?? "";
  return cabDualEffectiveCab2Hex(wireCab2, slot.cabHexHint);
}

async function buildDualTabPanesFromCabDualWire(
  dualCatalogModelId: string,
  cab1Hex: string,
  cab2Hex: string,
  found: { entry: ModelDefinitionJson },
  dualMeta: PresetMetaJson | null,
  dualImage: string | null,
  cab1ChainValues: ChainParamValueJson[] | null,
  cab1RoutingSignal: string | null,
): Promise<DualTabPaneConfig[] | null> {
  const dualId = dualCatalogModelId.trim();
  const cab2ModelId =
    (await getCatalogModelIdForCabDualCab2Hex(dualId, cab2Hex, cab1Hex))?.trim() ?? "";
  if (!cab2ModelId) return null;
  const cab2Def = await findModelDefinitionBySymbolicId(cab2ModelId, "Cab");
  const cab2Meta = await getPresetMetaForId(cab2ModelId);
  const cab2Image = await getCatalogModelImageForId(cab2ModelId);
  const cab2Signal = pickSignal(cab2Meta, cab2Hex);
  const cab2Defaults = cab2Def
    ? buildDefaultChainValuesForSourceOrder(cab2Def.entry.params ?? [], cab2Signal)
    : [];
  const cab2Title =
    cab2Def?.entry.name.trim() ||
    (await getCatalogModelNameForId(cab2ModelId))?.trim() ||
    "—";
  return [
    {
      tabLabel: "Cab 1",
      modelTitle: found.entry.name.trim() || "—",
      catalogModelId: dualId,
      basedOn: pickBasedOn(dualMeta),
      modelImage: dualImage,
      params: found.entry.params ?? [],
      chainValues: cab1ChainValues,
      catalogRoutingSignal: cab1RoutingSignal,
    },
    {
      tabLabel: "Cab 2",
      modelTitle: cab2Title,
      catalogModelId: cab2ModelId,
      basedOn: pickBasedOn(cab2Meta),
      modelImage: cab2Image,
      params: cab2Def?.entry.params ?? [],
      chainValues: cab2Defaults,
      catalogRoutingSignal: cab2Signal ?? cab1RoutingSignal,
    },
  ];
}

async function buildDualTabPanesFromAmpCabCatalog(
  ampCatalogModelId: string,
  assignVariant: string,
  found: { entry: ModelDefinitionJson },
  ampMeta: PresetMetaJson | null,
  ampImage: string | null,
  ampChainValues: ChainParamValueJson[] | null,
  ampRoutingSignal: string | null,
  cabCatalogModelIdOverride?: string | null,
): Promise<DualTabPaneConfig[] | null> {
  const pair = await ampCabHexPairFromAssignVariant(ampCatalogModelId, assignVariant);
  if (!pair) return null;
  const cabOverride = (cabCatalogModelIdOverride ?? "").trim();
  let cabModelId = cabOverride || (await getCatalogModelIdForHex(pair.cabHex, "Cab"));
  if (!cabModelId) return null;
  const cabMeta = await getPresetMetaForId(cabModelId);
  const cabHexForSignal =
    (cabOverride
      ? (await moduleHexForUsbVariant(cabOverride, "single", cabMeta))?.trim()
      : null) || pair.cabHex;
  const cabDef = await findModelDefinitionBySymbolicId(cabModelId, "Cab");
  const cabImage = await getCatalogModelImageForId(cabModelId);
  const cabSignal = pickSignal(cabMeta, cabHexForSignal);
  const cabDefaults = cabDef
    ? buildDefaultChainValuesForSourceOrder(cabDef.entry.params ?? [], cabSignal)
    : [];
  const cabTitle =
    cabDef?.entry.name.trim() ||
    (await getCatalogModelNameForId(cabModelId))?.trim() ||
    "—";
  return [
    {
      tabLabel: "Amp",
      modelTitle: found.entry.name.trim() || "—",
      catalogModelId: ampCatalogModelId.trim(),
      basedOn: pickBasedOn(ampMeta),
      modelImage: ampImage,
      params: found.entry.params ?? [],
      chainValues: ampChainValues,
      catalogRoutingSignal: ampRoutingSignal,
    },
    {
      tabLabel: "Cab",
      modelTitle: cabTitle,
      catalogModelId: cabModelId.trim(),
      basedOn: pickBasedOn(cabMeta),
      modelImage: cabImage,
      params: cabDef?.entry.params ?? [],
      chainValues: cabDefaults,
      catalogRoutingSignal: cabSignal,
    },
  ];
}

async function buildDualTabPanesFromCabDualCatalog(
  dualCatalogModelId: string,
  assignVariant: string,
  found: { entry: ModelDefinitionJson },
  dualMeta: PresetMetaJson | null,
  dualImage: string | null,
  cab1ChainValues: ChainParamValueJson[] | null,
  cab1RoutingSignal: string | null,
  cab1ModelIdOverride?: string | null,
  cab2ModelIdOverride?: string | null,
): Promise<DualTabPaneConfig[] | null> {
  const pair = await cabDualHexPairFromAssignVariant(dualCatalogModelId, assignVariant);
  if (!pair) return null;
  const cab1ModelId =
    (cab1ModelIdOverride ?? "").trim() || dualCatalogModelId.trim();
  const cab2ModelId =
    (cab2ModelIdOverride ?? "").trim() ||
    (await getCatalogModelIdForCabDualCab2Hex(
      dualCatalogModelId,
      pair.cab2Hex,
      pair.cab1Hex,
    ))?.trim() ||
    "";
  if (!cab2ModelId) return null;
  const hexes = [pair.cab1Hex, pair.cab2Hex];
  const modelIds = [cab1ModelId, cab2ModelId];
  const paneDefs: DualTabPaneConfig[] = [];
  for (let i = 0; i < 2; i += 1) {
    const modelId = modelIds[i]!;
    const def =
      i === 0
        ? found
        : await findModelDefinitionBySymbolicId(modelId, "Cab");
    const partMeta = modelId ? await getPresetMetaForId(modelId) : null;
    const modelImage =
      i === 0 ? dualImage : await getCatalogModelImageForId(modelId);
    const signal = pickSignal(partMeta, hexes[i]);
    const defaults = def
      ? buildDefaultChainValuesForSourceOrder(def.entry.params ?? [], signal)
      : [];
    const title =
      def?.entry.name.trim() ||
      (await getCatalogModelNameForId(modelId))?.trim() ||
      "—";
    paneDefs.push({
      tabLabel: i === 0 ? "Cab 1" : "Cab 2",
      modelTitle: title,
      catalogModelId: modelId,
      basedOn: pickBasedOn(i === 0 ? dualMeta : partMeta),
      modelImage,
      params: def?.entry.params ?? [],
      chainValues: i === 0 ? cab1ChainValues : defaults,
      catalogRoutingSignal: signal ?? cab1RoutingSignal,
    });
  }
  return paneDefs.length === 2 ? paneDefs : null;
}

type CabDualResolve = {
  dualTabPanes: DualTabPaneConfig[] | null;
};

/** Onglets Cab 1 / Cab 2 : trame HW (fil + `c219`) d’abord, preset ensuite, défaut JSON en dernier recours. */
async function resolveCabDualTabPanes(
  kemplineSlotIndex: number | undefined,
  slot: SlotDebug,
  catalogModelIdTrimmed: string,
  found: { entry: ModelDefinitionJson },
  meta: PresetMetaJson | null,
  catalogImage: string | null,
  cabChainValues: ChainParamValueJson[] | null,
  catalogRoutingSignal: string | null,
  assignVariantHint?: string | null,
): Promise<CabDualResolve> {
  if (!slotWantsCabDualTabs(slot, assignVariantHint, meta)) {
    return { dualTabPanes: null };
  }
  const wire = cabDualWireParts(slot.moduleHex);
  const cab2HexTrame = await resolveCabDualCab2HexFromTrame(slot, kemplineSlotIndex);
  const pair = await cabDualHexPairFromAssignVariant(catalogModelIdTrimmed, "dual");
  const cab1Hex = wire?.cab1Hex ?? pair?.cab1Hex ?? "";
  const probeCab2Hint = probePickerCabDualCab2Hint(
    kemplineSlotIndex,
    catalogModelIdTrimmed,
  );

  const applyCab2Overrides = async (
    panes: DualTabPaneConfig[],
  ): Promise<DualTabPaneConfig[]> => {
    let next = panes;
    if (cab2HexTrame && cab1Hex) {
      const fromTrame = (
        await getCatalogModelIdForCabDualCab2Hex(
          catalogModelIdTrimmed,
          cab2HexTrame,
          cab1Hex,
        )
      )?.trim();
      if (fromTrame) {
        next = await applyCabDualPane2ModelOverride(next, fromTrame);
      }
    }
    if (probeCab2Hint) {
      next = await applyCabDualPane2ModelOverride(next, probeCab2Hint, {
        forceDefaults: true,
      });
    }
    return next;
  };

  // 1) Trame matérielle (scroll / fil combiné) — vérité Cab 2, pas le cd02d6 usine du bulkHex JSON.
  if (cab1Hex && cab2HexTrame) {
    const fromWire = await buildDualTabPanesFromCabDualWire(
      catalogModelIdTrimmed,
      cab1Hex,
      cab2HexTrame,
      found,
      meta,
      catalogImage,
      cabChainValues,
      catalogRoutingSignal,
    );
    if (fromWire) {
      return { dualTabPanes: await applyCab2Overrides(fromWire) };
    }
  }

  if (
    kemplineSlotIndex !== undefined &&
    Number.isInteger(kemplineSlotIndex)
  ) {
    const dualParts = await resolveSlotDualParts(kemplineSlotIndex, {
      slot,
      catalogModelId: catalogModelIdTrimmed,
      kind: "cab_dual",
      assignVariant: assignVariantHint,
    });
    if (
      dualParts &&
      dualParts.parts.length === 2 &&
      dualParts.kind === "cab_dual"
    ) {
      let panes = await buildDualTabPanesFromParts(
        dualParts,
        catalogModelIdTrimmed,
      );
      if (panes) {
        return { dualTabPanes: await applyCab2Overrides(panes) };
      }
    }
  }
  const probeAssign =
    (assignVariantHint ?? "").trim().toLowerCase() ||
    (probePickerAssignVariantHint(kemplineSlotIndex) ?? "").trim().toLowerCase();
  const metaAssign = usbAssignVariantFromPresetMeta(meta, slot.moduleHex, slot.category);
  const assignVariant =
    probeAssign === "dual"
      ? "dual"
      : metaAssign === "dual" || slotWantsCabDualTabs(slot, null, meta)
        ? "dual"
        : metaAssign;
  if (assignVariant !== "dual") {
    return { dualTabPanes: null };
  }
  const cabSync =
    cabDualPickerSync &&
    kemplineSlotIndex !== undefined &&
    cabDualPickerSync.dualCatalogModelId.trim().toLowerCase() ===
      catalogModelIdTrimmed.trim().toLowerCase()
      ? cabDualPickerSync
      : null;
  // Pas de repli sur le Cab 2 par défaut du JSON si la trame a déjà un hex Cab 2.
  let cab2ModelOverride = probeCab2Hint ?? cabSync?.cab2CatalogModelId ?? null;
  if (!cab2ModelOverride && cab2HexTrame && cab1Hex) {
    cab2ModelOverride =
      (await getCatalogModelIdForCabDualCab2Hex(
        catalogModelIdTrimmed,
        cab2HexTrame,
        cab1Hex,
      ))?.trim() ?? null;
  }
  if (!cab2ModelOverride && cab2HexTrame) {
    return { dualTabPanes: null };
  }
  const panes = await buildDualTabPanesFromCabDualCatalog(
    catalogModelIdTrimmed,
    assignVariant,
    found,
    meta,
    catalogImage,
    cabChainValues,
    catalogRoutingSignal,
    cabSync?.cab1CatalogModelId,
    cab2ModelOverride,
  );
  return { dualTabPanes: panes };
}

/** Onglets Amp/Cab : preset dump si dispo, sinon paire par défaut depuis `HX_ModelUsbAssign.json`. */
async function resolveAmpCabDualTabPanes(
  kemplineSlotIndex: number | undefined,
  slot: SlotDebug,
  catalogModelIdTrimmed: string,
  found: { entry: ModelDefinitionJson },
  meta: PresetMetaJson | null,
  catalogImage: string | null,
  ampChainValues: ChainParamValueJson[] | null,
  catalogRoutingSignal: string | null,
  assignVariantHint?: string | null,
): Promise<AmpCabDualResolve> {
  const probeAssign =
    (assignVariantHint ?? "").trim().toLowerCase() ||
    (probePickerAssignVariantHint(kemplineSlotIndex) ?? "").trim().toLowerCase();
  const preferCatalogAssign =
    probeAssign === "amp+cab" || probeAssign === "amp+cab-legacy";

  if (
    !preferCatalogAssign &&
    kemplineSlotIndex !== undefined &&
    Number.isInteger(kemplineSlotIndex)
  ) {
    const dualParts = await resolveSlotDualParts(kemplineSlotIndex, {
      slot,
      catalogModelId: catalogModelIdTrimmed,
      kind: "amp_cab",
      assignVariant: assignVariantHint,
    });
    if (
      dualParts &&
      dualParts.parts.length === 2 &&
      dualParts.kind === "amp_cab"
    ) {
      const presetAmpId = dualParts.parts[0]?.modelId?.trim() ?? "";
      if (
        !presetAmpId ||
        presetAmpId.toLowerCase() === catalogModelIdTrimmed.trim().toLowerCase()
      ) {
        let panes = await buildDualTabPanesFromParts(dualParts);
        if (panes) {
          const cabHint = probePickerAmpCabCabHint(
            kemplineSlotIndex,
            catalogModelIdTrimmed,
          );
          if (cabHint) {
            panes = await applyCabDualPane2ModelOverride(panes, cabHint);
          }
          const cabCatalogModelId =
            cabHint ?? dualParts.parts[1]?.modelId?.trim() ?? null;
          let linkedCabHex = dualParts.parts[1]?.chainHex?.trim() ?? null;
          if (cabHint) {
            const cabMeta = await getPresetMetaForId(cabHint);
            linkedCabHex =
              (await moduleHexForUsbVariant(cabHint, "single", cabMeta))?.trim() ||
              linkedCabHex;
          }
          const assignVariantForDual =
            probeAssign ||
            (await usbAssignVariantForAmpCabSlot(
              meta,
              slot.moduleHex,
              slot.category,
              catalogModelIdTrimmed,
              linkedCabHex,
            ));
          return {
            dualTabPanes: panes,
            linkedCabHex,
            cabCatalogModelId,
            assignVariant: assignVariantForDual,
          };
        }
      }
    }
  }
  const assignVariant =
    probeAssign ||
    (await usbAssignVariantForAmpCabSlot(
      meta,
      slot.moduleHex,
      slot.category,
      catalogModelIdTrimmed,
    ));
  if (assignVariant !== "amp+cab" && assignVariant !== "amp+cab-legacy") {
    return { dualTabPanes: null, linkedCabHex: null, cabCatalogModelId: null, assignVariant: "amp+cab" };
  }
  const pair = await ampCabHexPairFromAssignVariant(catalogModelIdTrimmed, assignVariant);
  if (!pair) {
    return { dualTabPanes: null, linkedCabHex: null, cabCatalogModelId: null, assignVariant };
  }
  const cabOverride =
    probePickerAmpCabCabHint(kemplineSlotIndex, catalogModelIdTrimmed) ??
    (ampCabDualPickerSync &&
    kemplineSlotIndex !== undefined &&
    ampCabDualPickerSync.ampCatalogModelId.trim().toLowerCase() ===
      catalogModelIdTrimmed.trim().toLowerCase()
      ? ampCabDualPickerSync.cabCatalogModelId
      : null);
  const panes = await buildDualTabPanesFromAmpCabCatalog(
    catalogModelIdTrimmed,
    assignVariant,
    found,
    meta,
    catalogImage,
    ampChainValues,
    catalogRoutingSignal,
    cabOverride,
  );
  const cabCatalogModelId =
    (cabOverride ?? "").trim() ||
    (await getCatalogModelIdForHex(pair.cabHex, "Cab"))?.trim() ||
    null;
  const linkedCabHex =
    (cabOverride
      ? (
          await moduleHexForUsbVariant(
            cabOverride,
            "single",
            await getPresetMetaForId(cabOverride),
          )
        )?.trim()
      : null) ||
    pair.cabHex;
  return {
    dualTabPanes: panes,
    linkedCabHex,
    cabCatalogModelId,
    assignVariant,
  };
}

function showModelsParamsLoading() {
  clearModelsParamsSubheadAndIcon();
  selectedParamsInPlaceUpdater = null;
  selectedParamsInPlaceSlotKey = null;
  selectedParamsHwWireContext = null;
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  const p = document.createElement("p");
  p.className = "models-params-placeholder";
  p.textContent = "Chargement des paramètres…";
  inner.appendChild(p);
}

function renderModelsParamsPane(
  slot: SlotDebug,
  params: ModelParamDefJson[],
  resolvedCatalogModelName?: string,
  chainValues?: ChainParamValueJson[] | null,
  catalogBasedOn?: string | null,
  _catalogSubcategoryLabel?: string | null,
  catalogRoutingSignal?: string | null,
  helixControlsMap?: Map<string, HelixControlDefJson>,
  catalogModelImage?: string | null,
  kemplineSlotIndex?: number,
  dualTabPanes?: DualTabPaneConfig[] | null,
  dualSlotKind?: "amp_cab" | "cab_dual" | null,
  ampCabAssignVariant?: string | null,
  cabDualAssignVariant?: string | null,
  cabAssignVariant?: string | null,
) {
  const inner = getModelsParamsInner();
  if (!inner) return;

  const useDualTabs = dualTabPanes && dualTabPanes.length === 2;

  if (!useDualTabs) {
  const head = document.createElement("div");
  head.className = "models-params-model-head";
  const baseName = (resolvedCatalogModelName ?? slot.name).trim() || "—";
  const bo = (catalogBasedOn ?? "").trim();
  const title = document.createElement("h2");
  title.className = "models-params-model-title";
  title.textContent = baseName;
  const lines: HTMLElement[] = [title];
  const basedOnEl = getModelsParamsBasedOnEl();
  if (basedOnEl) {
    basedOnEl.replaceChildren();
    basedOnEl.title = bo;
    if (bo) {
      const label = document.createElement("span");
      label.className = "models-params-pane-basedon-label";
      label.textContent = "Based on : ";
      const value = document.createElement("span");
      value.className = "models-params-pane-basedon-value";
      value.textContent = bo;
      basedOnEl.append(label, value);
    }
  }
  if (resolvedCatalogModelName && resolvedCatalogModelName.trim() !== slot.name.trim()) {
    const usb = document.createElement("div");
    usb.className = "models-params-model-usb-name";
    usb.textContent = slot.name.trim();
    lines.push(usb);
    head.append(...lines);
  } else {
    head.append(...lines);
  }
  const subhead = getModelsParamsSubheadEl();
  if (subhead) {
    subhead.replaceChildren(head);
  }
  setModelsParamsHeaderIcon(slot, catalogModelImage);
  }

  inner.replaceChildren();

  let applyRawChainValuesInPlace: ((raw: ChainParamValueJson[] | null) => void) | null = null;

  if (useDualTabs) {
    applyRawChainValuesInPlace = renderModelsParamsDualTabs(
      inner,
      dualTabPanes,
      helixControlsMap,
      kemplineSlotIndex,
      slot,
      dualSlotKind ?? "cab_dual",
      ampCabAssignVariant ?? null,
      cabDualAssignVariant ?? null,
    );
  } else {
  const list = document.createElement("ul");
  list.className = "models-params-list";
  const paramsForDisplay = params;
  const catN = normalizeCategory(slot.category);
  let chainForAlign = chainValues;
  if (
    (catN === "split" || catN === "merge") &&
    (!chainForAlign || chainForAlign.length === 0) &&
    params.length > 0
  ) {
    const order: string[] = params.map((p) => (p.symbolicID ?? "").trim()).filter(Boolean);
    const bySid = new Map(
      params
        .map((p) => [(p.symbolicID ?? "").trim(), p] as const)
        .filter(([k]) => Boolean(k)),
    );
    const synth: ChainParamValueJson[] = [];
    for (const sid of order) {
      const p = bySid.get(sid);
      const d = p ? chainValueFromParamDefault(p) : undefined;
      if (d !== undefined) synth.push(d);
    }
    if (synth.length > 0) chainForAlign = synth;
  }
  const chainAligned = mergeLiveChainOverridesIntoAligned(
    currentPresetIndex,
    kemplineSlotIndex,
    paramsForDisplay,
    alignChainValuesToModelParamOrder(
      chainForAlign,
      paramsForDisplay,
      params,
      catalogRoutingSignal,
    ),
  );
  const updateAlignedValues = appendModelsParamRows(
    list,
    paramsForDisplay,
    chainAligned ?? null,
    helixControlsMap,
    catalogRoutingSignal,
    "",
    kemplineSlotIndex,
    0,
    null,
    ampCabAssignVariant ?? null,
    cabDualAssignVariant ?? null,
    null,
    cabAssignVariant === "legacy",
  );
  applyRawChainValuesInPlace = (rawChainValues: ChainParamValueJson[] | null): void => {
    const aligned = mergeLiveChainOverridesIntoAligned(
      currentPresetIndex,
      kemplineSlotIndex,
      paramsForDisplay,
      alignChainValuesToModelParamOrder(
        rawChainValues,
        paramsForDisplay,
        params,
        catalogRoutingSignal,
      ),
    );
    updateAlignedValues(aligned ?? null);
  };
  inner.appendChild(list);
  }

  const paramsForDisplay = useDualTabs
    ? (dualTabPanes![
        dualSlotKind === "amp_cab"
          ? ampCabDualActiveTab
          : dualSlotKind === "cab_dual"
            ? cabDualActiveTab
            : 0
      ]?.params ?? params)
    : params;
  const slotKeyForPane = makeSlotSelectionKey(slot, kemplineSlotIndex);
  const kempPaneMatches =
    kemplineSlotIndex !== undefined &&
    selectedParamsKemplineSlotIndex !== null &&
    selectedParamsKemplineSlotIndex === kemplineSlotIndex;
  if (
    selectedParamsPresetIndex === currentPresetIndex &&
    selectedParamsSlotKey &&
    (selectedParamsSlotKey === slotKeyForPane || kempPaneMatches)
  ) {
    selectedParamsInPlaceUpdater = applyRawChainValuesInPlace;
    selectedParamsInPlaceSlotKey = slotKeyForPane;
    selectedParamsSlotKey = slotKeyForPane;
    if (useDualTabs && dualTabPanes && dualSlotKind) {
      const activeTab =
        dualSlotKind === "amp_cab"
          ? ampCabDualActiveTab
          : dualSlotKind === "cab_dual"
            ? cabDualActiveTab
            : 0;
      selectedParamsHwWireContext = dualPaneHwWireContext(
        dualSlotKind,
        activeTab,
        dualTabPanes,
      );
    } else {
      selectedParamsHwWireContext = {
        paramsForDisplay,
        catalogSignal: catalogRoutingSignal,
        wireParamIndexBase: 0,
      };
    }
  }
}

function showModelsParamsNotFound(slot: SlotDebug, resolvedCatalogId?: string | null) {
  clearModelsParamsSubheadAndIcon();
  resetSlotPickerToIdle();
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  const p = document.createElement("p");
  p.className = "models-params-placeholder";
  const hex = (slot.moduleHex ?? "").trim();
  const id = (resolvedCatalogId ?? "").trim();
  p.textContent = id
    ? `Aucune entrée .models pour l’id catalogue « ${id} » (USB « ${slot.name.trim()} », catégorie « ${slot.category.trim()} », chainHex ${hex ? hex.toUpperCase() : "—"}). Jointure par id symbolique, pas par nom.`
    : `Aucune définition « ${slot.name.trim()} » pour la catégorie « ${slot.category.trim()} ».`;
  inner.appendChild(p);
}

function showModelsParamsError(message: string) {
  clearModelsParamsSubheadAndIcon();
  resetSlotPickerToIdle();
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  const p = document.createElement("p");
  p.className = "models-params-placeholder models-params-error";
  p.textContent = message;
  inner.appendChild(p);
}

async function loadAndShowModelsParamsForSlot(
  slot: SlotDebug,
  kemplineSlotIndex?: number,
) {
  if (
    kemplineSlotIndex !== undefined &&
    Number.isInteger(kemplineSlotIndex) &&
    slotModelUsbProbeInFlight === kemplineSlotIndex
  ) {
    return;
  }
  const seq = ++modelsParamsLoadSeq;
  const slotKeyEarly = makeSlotSelectionKey(slot, kemplineSlotIndex);
  const innerBefore = getModelsParamsInner();
  const kempMatchesEarly =
    kemplineSlotIndex === undefined ||
    (selectedParamsKemplineSlotIndex !== null && selectedParamsKemplineSlotIndex === kemplineSlotIndex);
  const preserveParamsChrome =
    kempMatchesEarly &&
    selectedParamsPresetIndex === currentPresetIndex &&
    selectedParamsSlotKey === slotKeyEarly &&
    selectedParamsInPlaceUpdater !== null &&
    selectedParamsInPlaceSlotKey === slotKeyEarly &&
    innerBefore !== null &&
    (innerBefore.querySelector("ul.models-params-list") !== null ||
      innerBefore.querySelector(".models-params-dual-tabs") !== null);
  if (!preserveParamsChrome) {
    showModelsParamsLoading();
  }
  const nk = normalizeCategory(slot.category);
  if (nk === "routing" || nk === "none" || nk === "favorites") {
    if (seq === modelsParamsLoadSeq) showModelsParamsNotFound(slot, null);
    return;
  }
  applyPickerForStructuralSlot(slot);
  try {
    let chainValues: ChainParamValueJson[] | null = null;
    let dualTabPanes: DualTabPaneConfig[] | null = null;
    let dualSlotKind: "amp_cab" | "cab_dual" | null = null;
    let linkedCabHexForPicker: string | null = null;
    let ampCabDualResolve: AmpCabDualResolve | null = null;
    let cabDualAssignVariant: string | null = null;
    let catalogModelIdTrimmed = await resolveSlotCatalogModelId(slot);
    const mergeLike =
      nk === "merge" ||
      (nk === "routing" && slot.name.toLowerCase().includes("merge"));
    if (!catalogModelIdTrimmed && mergeLike) {
      catalogModelIdTrimmed = FLOW_JOIN_CATALOG_ID;
    }
    if (!catalogModelIdTrimmed) {
      if (seq !== modelsParamsLoadSeq) return;
      const hex = (slot.moduleHex ?? "").trim();
      showModelsParamsError(
        hex
          ? `Jointure ID impossible : aucun modèle catalogue pour chainHex « ${hex.toUpperCase()} ».`
          : "Jointure ID impossible : chainHex manquant pour ce slot.",
      );
      return;
    }
    if (seq !== modelsParamsLoadSeq) return;
    const presetMetaForFiles = await getPresetMetaForId(catalogModelIdTrimmed);
    const catalogPresetCategoryName = presetMetaForFiles?.categoryName ?? null;
    const found = await findModelDefinitionForSlot(
      slot,
      catalogModelIdTrimmed,
      catalogPresetCategoryName,
    );
    if (seq !== modelsParamsLoadSeq) return;
    if (!found) {
      showModelsParamsNotFound(slot, catalogModelIdTrimmed);
      return;
    }
    const short = found.entry.name.trim();
    kemplineTooltipCache.set(tooltipCacheKey(slot), short);
    applyShortNameToSlotNodes(slot, short);
    const meta = presetMetaForFiles;
    const catalogImage = await getCatalogModelImageForId(catalogModelIdTrimmed);
    if (seq !== modelsParamsLoadSeq) return;
    const catalogBasedOn = pickBasedOn(meta);
    const catalogSubcategoryLabel = formatSubCategoryForHeader(meta, slot.moduleHex);
    const catalogRoutingSignal = pickSignal(meta, slot.moduleHex);
    if (
      !slotWantsAmpCabDualTabs(slot, probePickerAssignVariantHint(kemplineSlotIndex)) &&
      !slotWantsCabDualTabs(
        slot,
        probePickerAssignVariantHint(kemplineSlotIndex),
        meta,
      )
    ) {
      clearAmpCabDualPickerContext();
      clearCabDualPickerContext();
    }
    if (
      kemplineSlotIndex !== undefined &&
      Number.isInteger(kemplineSlotIndex) &&
      nk !== "input" &&
      nk !== "output" &&
      nk !== "split" &&
      nk !== "merge"
    ) {
      chainValues = await resolveChainValuesForKemplineSlot(
        kemplineSlotIndex,
        slot,
        catalogModelIdTrimmed,
        catalogPresetCategoryName,
        catalogRoutingSignal,
      );
    }
    if (
      kemplineSlotIndex !== undefined &&
      Number.isInteger(kemplineSlotIndex) &&
      slotWantsAmpCabDualTabs(slot, probePickerAssignVariantHint(kemplineSlotIndex))
    ) {
      const assignHint = probePickerAssignVariantHint(kemplineSlotIndex);
      const ampCabDual = await resolveAmpCabDualTabPanes(
        kemplineSlotIndex,
        slot,
        catalogModelIdTrimmed,
        found,
        meta,
        catalogImage,
        chainValues,
        catalogRoutingSignal,
        assignHint,
      );
      if (ampCabDual.dualTabPanes) {
        dualSlotKind = "amp_cab";
        dualTabPanes = ampCabDual.dualTabPanes;
        linkedCabHexForPicker = ampCabDual.linkedCabHex;
        ampCabDualResolve = ampCabDual;
        ampCabDualPickerSync = {
          ampCatalogModelId: catalogModelIdTrimmed,
          meta,
          moduleHex: slot.moduleHex,
          slotCategory: slot.category,
          linkedCabHex: linkedCabHexForPicker,
          cabCatalogModelId:
            ampCabDual.cabCatalogModelId ??
            probePickerAmpCabCabHint(kemplineSlotIndex, catalogModelIdTrimmed) ??
            "",
        };
        syncSlotDualPartsSessionFromTabPanes(
          kemplineSlotIndex!,
          "amp_cab",
          ampCabDual.dualTabPanes,
          slot,
          ampCabDual.linkedCabHex,
        );
      }
    } else if (
      slotWantsCabDualTabs(
        slot,
        probePickerAssignVariantHint(kemplineSlotIndex),
        meta,
      )
    ) {
      const cabDual = await resolveCabDualTabPanes(
        kemplineSlotIndex,
        slot,
        catalogModelIdTrimmed,
        found,
        meta,
        catalogImage,
        chainValues,
        catalogRoutingSignal,
        probePickerAssignVariantHint(kemplineSlotIndex),
      );
      if (cabDual.dualTabPanes) {
        dualSlotKind = "cab_dual";
        dualTabPanes = cabDual.dualTabPanes;
        cabDualAssignVariant = cabDualAssignVariantFromMeta(meta);
        lastCabDualTabPanesContext = {
          dualTabPanes: cabDual.dualTabPanes,
          dualCatalogModelId: catalogModelIdTrimmed,
          meta,
          slot,
          kemplineSlotIndex,
        };
        if (kemplineSlotIndex !== undefined && Number.isInteger(kemplineSlotIndex)) {
          syncSlotDualPartsSessionFromTabPanes(
            kemplineSlotIndex,
            "cab_dual",
            cabDual.dualTabPanes,
            slot,
          );
        }
      } else {
        lastCabDualTabPanesContext = null;
      }
    }
    const helixControlsMap = await getHelixControlsMap();
    if (seq !== modelsParamsLoadSeq) return;
    const slotKeyNow = makeSlotSelectionKey(slot, kemplineSlotIndex);
    const prevCatalogId = paramsPaneCatalogBySlotKey.get(slotKeyNow);
    const modelChangedAtSlot =
      prevCatalogId !== undefined && prevCatalogId !== catalogModelIdTrimmed;
    if (
      modelChangedAtSlot &&
      kemplineSlotIndex !== undefined &&
      Number.isInteger(kemplineSlotIndex) &&
      currentPresetIndex >= 0
    ) {
      clearLiveChainOverridesForKemplineSlot(currentPresetIndex, kemplineSlotIndex);
      clearSlotDualPartsSessionForKemplineSlot(currentPresetIndex, kemplineSlotIndex);
    }
    const kempMatchesNow =
      kemplineSlotIndex === undefined ||
      (selectedParamsKemplineSlotIndex !== null && selectedParamsKemplineSlotIndex === kemplineSlotIndex);
    const wantsDualTabs =
      dualSlotKind !== null ||
      slotWantsAmpCabDualTabs(slot, probePickerAssignVariantHint(kemplineSlotIndex)) ||
      slotWantsCabDualTabs(
        slot,
        probePickerAssignVariantHint(kemplineSlotIndex),
        meta,
      );
    const canPatchValuesOnly =
      !modelChangedAtSlot &&
      kempMatchesNow &&
      selectedParamsPresetIndex === currentPresetIndex &&
      selectedParamsSlotKey === slotKeyNow &&
      selectedParamsInPlaceUpdater !== null &&
      selectedParamsInPlaceSlotKey === slotKeyNow &&
      paramsPaneDualStructureMatches(dualSlotKind) &&
      (!wantsDualTabs || dualSlotKind !== null);

    if (canPatchValuesOnly && chainValues !== null && chainValues.length > 0) {
      const patchFn = selectedParamsInPlaceUpdater;
      if (patchFn) patchFn(chainValues);
      paramsPaneCatalogBySlotKey.set(slotKeyNow, catalogModelIdTrimmed);
      if (nk === "input") {
        const modelParams = found.entry.params ?? [];
        const aligned = alignChainValuesToModelParamOrder(
          chainValues,
          modelParams,
          modelParams,
          catalogRoutingSignal,
        );
        const inputParamChainIndex = modelParams.findIndex(
          (p) => (p.symbolicID ?? "").trim() === "@input",
        );
        void mountModelsSlotPicker().then(async () => {
          syncInputPickerHighlight(
            catalogModelIdTrimmed,
            aligned ?? chainValues,
            inputParamChainIndex >= 0 ? inputParamChainIndex : 0,
          );
          void refreshInputPickerFromLiveWireDelayed();
        });
      } else if (
        hwSlotBusFromSelectedParamsEl() === HW_SLOT_BUS_SPLIT ||
        hwSlotBusFromSelectedParamsEl() === HW_SLOT_BUS_MERGE ||
        resolveRoutingPickerCategory(slot.category, slot.name, catalogModelIdTrimmed, meta) ||
        nk === "output"
      ) {
        void mountModelsSlotPicker().then(async () => {
          await syncModelsSlotPickerFromLoadedModel(
            catalogModelIdTrimmed,
            meta,
            slot.moduleHex,
            slot.category,
            linkedCabHexForPicker,
            chainValues,
            0,
            slot.name,
            hwSlotBusFromSelectedParamsEl(),
          );
        });
      } else if (dualSlotKind === "amp_cab" && ampCabDualResolve) {
        void mountModelsSlotPicker().then(async () => {
          await mountAmpCabPickerSyncForSlot(
            ampCabDualResolve!,
            catalogModelIdTrimmed,
            meta,
            slot,
            kemplineSlotIndex,
          );
        });
      } else if (dualSlotKind === "cab_dual" && dualTabPanes) {
        void mountModelsSlotPicker().then(async () => {
          await mountCabDualPickerSyncForSlot(
            dualTabPanes,
            catalogModelIdTrimmed,
            meta,
            slot,
            kemplineSlotIndex,
          );
        });
      }
      if (
        selectedParamsPresetIndex === currentPresetIndex &&
        selectedParamsKemplineSlotIndex !== null &&
        selectedParamsKemplineSlotIndex === kemplineSlotIndex
      ) {
        selectedParamsValuesSig = `${currentPresetIndex}|${kemplineSlotIndex}|${chainValuesSignature(chainValues)}`;
      }
      return;
    }

    renderModelsParamsPane(
      slot,
      found.entry.params ?? [],
      short,
      chainValues,
      catalogBasedOn,
      catalogSubcategoryLabel,
      catalogRoutingSignal,
      helixControlsMap,
      catalogImage,
      kemplineSlotIndex,
      dualTabPanes,
      dualSlotKind,
      ampCabDualResolve?.assignVariant ?? null,
      cabDualAssignVariant,
      dualSlotKind ? null : cabAssignVariantFromMeta(meta),
    );
    void mountModelsSlotPicker().then(async () => {
      if (dualSlotKind === "amp_cab" && ampCabDualResolve) {
        await mountAmpCabPickerSyncForSlot(
          ampCabDualResolve,
          catalogModelIdTrimmed,
          meta,
          slot,
          kemplineSlotIndex,
        );
        return;
      }
      if (dualSlotKind === "cab_dual" && dualTabPanes) {
        await mountCabDualPickerSyncForSlot(
          dualTabPanes,
          catalogModelIdTrimmed,
          meta,
          slot,
          kemplineSlotIndex,
        );
        return;
      }
      const modelParams = found.entry.params ?? [];
      const aligned = alignChainValuesToModelParamOrder(
        chainValues,
        modelParams,
        modelParams,
        catalogRoutingSignal,
      );
      const inputParamChainIndex = modelParams.findIndex(
        (p) => (p.symbolicID ?? "").trim() === "@input",
      );
      await syncModelsSlotPickerFromLoadedModel(
        catalogModelIdTrimmed,
        meta,
        slot.moduleHex,
        slot.category,
        linkedCabHexForPicker,
        aligned ?? chainValues,
        inputParamChainIndex >= 0 ? inputParamChainIndex : 0,
        slot.name,
        hwSlotBusFromSelectedParamsEl(),
      );
      if (nk === "input") {
        void refreshInputPickerFromLiveWireDelayed();
      }
    });
    paramsPaneCatalogBySlotKey.set(slotKeyNow, catalogModelIdTrimmed);
    if (
      selectedParamsPresetIndex === currentPresetIndex &&
      selectedParamsKemplineSlotIndex !== null &&
      selectedParamsKemplineSlotIndex === kemplineSlotIndex
    ) {
      selectedParamsValuesSig = `${currentPresetIndex}|${kemplineSlotIndex}|${chainValuesSignature(chainValues)}`;
    }
  } catch (e) {
    if (seq !== modelsParamsLoadSeq) return;
    showModelsParamsError(e instanceof Error ? e.message : String(e));
  }
}

/** Panneau paramètres depuis le catalogue / `.models` uniquement (défauts), sans lecture chaîne USB. */
async function loadAndShowModelsParamsFromCatalogDefaults(
  slot: SlotDebug,
  catalogModelIdTrimmed: string,
  kemplineSlotIndex: number,
  options?: {
    assignVariant?: string;
    ampChainValues?: ChainParamValueJson[] | null;
  },
): Promise<void> {
  const seq = ++modelsParamsLoadSeq;
  const nk = normalizeCategory(slot.category);
  if (nk === "routing" || nk === "none" || nk === "favorites") {
    if (seq === modelsParamsLoadSeq) showModelsParamsNotFound(slot, null);
    return;
  }
  if (slotWantsAmpCabDualTabs(slot, options?.assignVariant)) {
    applyPickerForStructuralSlot(slot);
  }
  try {
    const presetMetaForFiles = await getPresetMetaForId(catalogModelIdTrimmed);
    const catalogPresetCategoryName = presetMetaForFiles?.categoryName ?? null;
    const found = await findModelDefinitionForSlot(
      slot,
      catalogModelIdTrimmed,
      catalogPresetCategoryName,
    );
    if (seq !== modelsParamsLoadSeq) return;
    if (!found) {
      showModelsParamsNotFound(slot, catalogModelIdTrimmed);
      return;
    }
    const short = found.entry.name.trim();
    kemplineTooltipCache.set(tooltipCacheKey(slot), short);
    applyShortNameToSlotNodes(slot, short);
    const meta = presetMetaForFiles;
    const catalogImage = await getCatalogModelImageForId(catalogModelIdTrimmed);
    if (seq !== modelsParamsLoadSeq) return;
    const catalogBasedOn = pickBasedOn(meta);
    const catalogSubcategoryLabel = formatSubCategoryForHeader(meta, slot.moduleHex);
    const catalogRoutingSignal = pickSignal(meta, slot.moduleHex);
    if (
      !slotWantsAmpCabDualTabs(slot, options?.assignVariant) &&
      !slotWantsCabDualTabs(slot, options?.assignVariant, meta)
    ) {
      clearAmpCabDualPickerContext();
      clearCabDualPickerContext();
    }
    const helixControlsMap = await getHelixControlsMap();
    if (seq !== modelsParamsLoadSeq) return;
    const defaultChain = buildDefaultChainValuesForSourceOrder(
      found.entry.params ?? [],
      catalogRoutingSignal,
    );
    const ampChainValues = options?.ampChainValues ?? defaultChain;
    let dualTabPanes: DualTabPaneConfig[] | null = null;
    let dualSlotKind: "amp_cab" | "cab_dual" | null = null;
    let linkedCabHexForPicker: string | null = null;
    let ampCabDualResolve: AmpCabDualResolve | null = null;
    let cabDualAssignVariant: string | null = null;
    if (slotWantsAmpCabDualTabs(slot, options?.assignVariant)) {
      const ampCabDual = await resolveAmpCabDualTabPanes(
        kemplineSlotIndex,
        slot,
        catalogModelIdTrimmed,
        found,
        meta,
        catalogImage,
        ampChainValues,
        catalogRoutingSignal,
        options?.assignVariant,
      );
      if (ampCabDual.dualTabPanes) {
        dualSlotKind = "amp_cab";
        dualTabPanes = ampCabDual.dualTabPanes;
        linkedCabHexForPicker = ampCabDual.linkedCabHex;
        ampCabDualResolve = ampCabDual;
        ampCabDualPickerSync = {
          ampCatalogModelId: catalogModelIdTrimmed,
          meta,
          moduleHex: slot.moduleHex,
          slotCategory: slot.category,
          linkedCabHex: linkedCabHexForPicker,
          cabCatalogModelId:
            ampCabDual.cabCatalogModelId ??
            probePickerAmpCabCabHint(kemplineSlotIndex, catalogModelIdTrimmed) ??
            "",
        };
        syncSlotDualPartsSessionFromTabPanes(
          kemplineSlotIndex!,
          "amp_cab",
          ampCabDual.dualTabPanes,
          slot,
          ampCabDual.linkedCabHex,
        );
      }
    } else if (slotWantsCabDualTabs(slot, options?.assignVariant, meta)) {
      const cabDual = await resolveCabDualTabPanes(
        kemplineSlotIndex,
        slot,
        catalogModelIdTrimmed,
        found,
        meta,
        catalogImage,
        ampChainValues,
        catalogRoutingSignal,
        options?.assignVariant,
      );
      if (cabDual.dualTabPanes) {
        dualSlotKind = "cab_dual";
        dualTabPanes = cabDual.dualTabPanes;
        cabDualAssignVariant = cabDualAssignVariantFromMeta(meta);
        lastCabDualTabPanesContext = {
          dualTabPanes: cabDual.dualTabPanes,
          dualCatalogModelId: catalogModelIdTrimmed,
          meta,
          slot,
          kemplineSlotIndex,
        };
        if (kemplineSlotIndex !== undefined && Number.isInteger(kemplineSlotIndex)) {
          syncSlotDualPartsSessionFromTabPanes(
            kemplineSlotIndex,
            "cab_dual",
            cabDual.dualTabPanes,
            slot,
          );
        }
      } else {
        lastCabDualTabPanesContext = null;
      }
    }
    renderModelsParamsPane(
      slot,
      found.entry.params ?? [],
      short,
      ampChainValues,
      catalogBasedOn,
      catalogSubcategoryLabel,
      catalogRoutingSignal,
      helixControlsMap,
      catalogImage,
      kemplineSlotIndex,
      dualTabPanes,
      dualSlotKind,
      ampCabDualResolve?.assignVariant ?? null,
      cabDualAssignVariant,
      dualSlotKind ? null : cabAssignVariantFromMeta(meta),
    );
    void mountModelsSlotPicker().then(async () => {
      if (dualSlotKind === "amp_cab" && ampCabDualResolve) {
        await mountAmpCabPickerSyncForSlot(
          ampCabDualResolve,
          catalogModelIdTrimmed,
          meta,
          slot,
          kemplineSlotIndex,
        );
        return;
      }
      if (dualSlotKind === "cab_dual" && dualTabPanes) {
        await mountCabDualPickerSyncForSlot(
          dualTabPanes,
          catalogModelIdTrimmed,
          meta,
          slot,
          kemplineSlotIndex,
        );
        return;
      }
      await syncModelsSlotPickerFromLoadedModel(
        catalogModelIdTrimmed,
        meta,
        slot.moduleHex,
        slot.category,
        linkedCabHexForPicker,
        undefined,
        0,
        slot.name,
        hwSlotBusFromSelectedParamsEl(),
      );
    });
    const slotKeyNow = makeSlotSelectionKey(slot, kemplineSlotIndex);
    paramsPaneCatalogBySlotKey.set(slotKeyNow, catalogModelIdTrimmed);
    if (
      selectedParamsPresetIndex === currentPresetIndex &&
      selectedParamsKemplineSlotIndex !== null &&
      selectedParamsKemplineSlotIndex === kemplineSlotIndex
    ) {
      selectedParamsValuesSig = `${currentPresetIndex}|${kemplineSlotIndex}|${chainValuesSignature(ampChainValues)}`;
    }
  } catch (e) {
    if (seq !== modelsParamsLoadSeq) return;
    showModelsParamsError(e instanceof Error ? e.message : String(e));
  }
}

/** Même clé que pour le cache des infobulles (catégorie normalisée + nom Kempline brut). */
function tooltipCacheKey(slot: SlotDebug): string {
  return JSON.stringify([normalizeCategory(slot.category), slot.name.trim()]);
}

const kemplineTooltipCache = new Map<string, string>();

async function resolveShortModelDisplayName(slot: SlotDebug): Promise<string> {
  const key = tooltipCacheKey(slot);
  const hit = kemplineTooltipCache.get(key);
  if (hit !== undefined) return hit;
  const nk = normalizeCategory(slot.category);
  let catalogModelId =
    (slot.catalogModelId ?? "").trim() || (await getCatalogModelIdForHex(slot.moduleHex));
  let idTrim = (catalogModelId ?? "").trim();
  const mergeLike =
    nk === "merge" ||
    (nk === "routing" && slot.name.toLowerCase().includes("merge"));
  if (!idTrim && mergeLike) idTrim = FLOW_JOIN_CATALOG_ID;
  const metaForFiles = await getPresetMetaForId(idTrim);
  const found = await findModelDefinitionForSlot(
    slot,
    idTrim,
    metaForFiles?.categoryName ?? null,
  );
  const display = (found?.entry.name ?? slot.name).trim() || "—";
  kemplineTooltipCache.set(key, display);
  return display;
}

function applyShortNameToSlotNodes(slot: SlotDebug, tip: string) {
  const cat = slot.category.trim();
  const kn = slot.name.trim();
  const aria = tip.replace(/\n/g, " — ");
  for (const el of contentEl.querySelectorAll<HTMLElement>(
    ".node.node--hx-slot.node--params-clickable",
  )) {
    if (el.classList.contains("node-empty")) continue;
    if (el.dataset.slotCategory === cat && el.dataset.slotKemplineName === kn) {
      el.title = tip;
      el.setAttribute("aria-label", aria);
    }
  }
}

async function refreshAllSlotTooltipsInContent(): Promise<void> {
  const nodes = [...contentEl.querySelectorAll<HTMLElement>(
    ".node.node--hx-slot.node--params-clickable",
  )].filter((el) => !el.classList.contains("node-empty"));

  const groups = new Map<string, HTMLElement[]>();
  for (const el of nodes) {
    const cat = el.dataset.slotCategory ?? "";
    const kn = el.dataset.slotKemplineName ?? "";
    if (!kn || kn === "<empty>") continue;
    const key = JSON.stringify([normalizeCategory(cat), kn]);
    const arr = groups.get(key) ?? [];
    arr.push(el);
    groups.set(key, arr);
  }

  await Promise.all(
    [...groups.entries()].map(async ([key, els]) => {
      const [catNorm, kn] = JSON.parse(key) as [string, string];
      const catRaw = els[0]?.dataset.slotCategory ?? catNorm;
      const tip = await resolveShortModelDisplayName({ category: catRaw, name: kn });
      const aria = tip.replace(/\n/g, " — ");
      for (const el of els) {
        el.title = tip;
        el.setAttribute("aria-label", aria);
      }
    }),
  );
}

/** URL Vite vers un asset sous `src-tauri/resources/` (encode le nom de fichier : `%` casse decodeURI). */
function tauriResourceUrl(subdir: string, filename: string): string {
  return `/src-tauri/resources/${subdir}/${encodeURIComponent(filename)}`;
}

function iconForCategory(category: string, name: string): string | null {
  const key = normalizeCategory(category);
  if (key === "routing" && name.toLowerCase().includes("merge")) {
    return tauriResourceUrl("icons_category", "FX_HX_Category_Merge.png");
  }
  const filename = CATEGORY_ICON_BY_KEY[key];
  if (!filename) return null;
  return tauriResourceUrl("icons_category", filename);
}

/** Fichier `image` assign : nom PNG sûr pour `icons_models/` (éviter `%` — préférer `_fN` pour N bandes). */
function sanitizeIconsModelsFilename(name: string): string | null {
  const t = name.trim();
  if (!t || t.includes("/") || t.includes("\\") || t.includes("..")) return null;
  if (!/^[a-zA-Z0-9_.()-]+\.png$/i.test(t)) return null;
  return t;
}

let modelsParamsIconPreviewPopover: HTMLDivElement | null = null;
let modelsParamsIconPreviewPopoverImg: HTMLImageElement | null = null;
let modelsParamsIconPreviewHideTimer: ReturnType<typeof setTimeout> | null = null;
let modelsParamsIconPreviewScrollController: AbortController | null = null;
let modelsParamsIconLivePreviewController: AbortController | null = null;

function getModelsParamsIconPreviewPopover(): { root: HTMLDivElement; img: HTMLImageElement } {
  if (!modelsParamsIconPreviewPopover) {
    const root = document.createElement("div");
    root.className = "models-params-pane-model-icon-preview-popover";
    root.hidden = true;
    root.setAttribute("role", "tooltip");
    const img = document.createElement("img");
    img.className = "models-params-pane-model-icon-preview-img";
    img.alt = "";
    img.decoding = "async";
    root.append(img);
    document.body.append(root);
    modelsParamsIconPreviewPopover = root;
    modelsParamsIconPreviewPopoverImg = img;
    root.addEventListener("mouseenter", () => {
      if (modelsParamsIconPreviewHideTimer !== null) {
        clearTimeout(modelsParamsIconPreviewHideTimer);
        modelsParamsIconPreviewHideTimer = null;
      }
    });
    root.addEventListener("mouseleave", () => {
      scheduleHideModelsParamsIconPreviewPopover(160);
    });
  }
  return { root: modelsParamsIconPreviewPopover, img: modelsParamsIconPreviewPopoverImg! };
}

function disposeIconPreviewScrollListeners(): void {
  modelsParamsIconPreviewScrollController?.abort();
  modelsParamsIconPreviewScrollController = null;
}

function disposeModelsParamsIconLivePreview(): void {
  modelsParamsIconLivePreviewController?.abort();
  modelsParamsIconLivePreviewController = null;
}

function hideModelsParamsIconPreviewPopover(): void {
  if (modelsParamsIconPreviewHideTimer !== null) {
    clearTimeout(modelsParamsIconPreviewHideTimer);
    modelsParamsIconPreviewHideTimer = null;
  }
  disposeIconPreviewScrollListeners();
  if (modelsParamsIconPreviewPopoverImg) {
    modelsParamsIconPreviewPopoverImg.onload = null;
    modelsParamsIconPreviewPopoverImg.onerror = null;
    modelsParamsIconPreviewPopoverImg.removeAttribute("src");
  }
  if (modelsParamsIconPreviewPopover) modelsParamsIconPreviewPopover.hidden = true;
}

function scheduleHideModelsParamsIconPreviewPopover(ms: number): void {
  if (modelsParamsIconPreviewHideTimer !== null) {
    clearTimeout(modelsParamsIconPreviewHideTimer);
    modelsParamsIconPreviewHideTimer = null;
  }
  modelsParamsIconPreviewHideTimer = window.setTimeout(() => {
    modelsParamsIconPreviewHideTimer = null;
    hideModelsParamsIconPreviewPopover();
  }, ms);
}

function positionModelsParamsIconPreviewPopover(anchor: HTMLElement): void {
  const pop = modelsParamsIconPreviewPopover;
  if (!pop || pop.hidden) return;
  const rect = anchor.getBoundingClientRect();
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const gap = 8;
  const pad = 6;
  const rw = pop.offsetWidth;
  const rh = pop.offsetHeight;
  let left = rect.right - rw;
  let top = rect.top - gap - rh;
  if (top < pad) top = rect.bottom + gap;
  if (left < pad) left = pad;
  if (left + rw > vw - pad) left = Math.max(pad, vw - pad - rw);
  if (top + rh > vh - pad) top = Math.max(pad, vh - pad - rh);
  pop.style.left = `${Math.round(left)}px`;
  pop.style.top = `${Math.round(top)}px`;
}

function attachIconPreviewScrollListeners(anchor: HTMLElement): void {
  disposeIconPreviewScrollListeners();
  modelsParamsIconPreviewScrollController = new AbortController();
  const { signal } = modelsParamsIconPreviewScrollController;
  const repos = () => positionModelsParamsIconPreviewPopover(anchor);
  window.addEventListener("scroll", repos, { capture: true, signal });
  window.addEventListener("resize", repos, { signal });
}

function showModelsParamsIconPreviewPopover(anchor: HTMLElement, imageSrc: string): void {
  if (modelsParamsIconPreviewHideTimer !== null) {
    clearTimeout(modelsParamsIconPreviewHideTimer);
    modelsParamsIconPreviewHideTimer = null;
  }
  const { root, img } = getModelsParamsIconPreviewPopover();
  img.src = imageSrc;
  const reveal = (): void => {
    root.hidden = false;
    requestAnimationFrame(() => {
      positionModelsParamsIconPreviewPopover(anchor);
      attachIconPreviewScrollListeners(anchor);
    });
  };
  img.onload = null;
  img.onerror = () => {
    reveal();
  };
  if (img.complete && img.naturalWidth > 0) reveal();
  else img.onload = () => reveal();
}

function bindModelsParamsIconLivePreview(
  wrap: HTMLElement,
  imageSrc: string,
  signal: AbortSignal,
): void {
  wrap.addEventListener(
    "mouseenter",
    () => {
      showModelsParamsIconPreviewPopover(wrap, imageSrc);
    },
    { signal },
  );
  wrap.addEventListener(
    "mouseleave",
    () => {
      scheduleHideModelsParamsIconPreviewPopover(120);
    },
    { signal },
  );
}

/**
 * Icône modèle (`HX_ModelCatalog.json` → `icons_models/`) ; repli icône catégorie matrice si absent ou erreur chargement.
 */
function setModelsParamsHeaderIcon(slot: SlotDebug, catalogModelImage?: string | null): void {
  const wrap = getModelsParamsModelIconWrapEl();
  if (!wrap) return;
  disposeModelsParamsIconLivePreview();
  hideModelsParamsIconPreviewPopover();
  wrap.replaceChildren();
  const safe = catalogModelImage ? sanitizeIconsModelsFilename(catalogModelImage) : null;
  let src: string | null = null;
  if (safe) src = tauriResourceUrl("icons_models", safe);
  if (!src) src = iconForCategory(slot.category, slot.name);
  if (src) {
    const img = document.createElement("img");
    img.className = "models-params-pane-model-icon-img";
    img.alt = "";
    img.width = 22;
    img.height = 22;
    img.decoding = "async";
    if (safe) {
      const catFallback = iconForCategory(slot.category, slot.name);
      if (catFallback) {
        img.addEventListener(
          "error",
          () => {
            if (img.src !== catFallback) img.src = catFallback;
          },
          { once: true },
        );
      }
    }
    img.src = src;
    wrap.appendChild(img);
    modelsParamsIconLivePreviewController = new AbortController();
    bindModelsParamsIconLivePreview(wrap, src, modelsParamsIconLivePreviewController.signal);
  } else {
    const ph = document.createElement("div");
    ph.className = "models-params-pane-model-icon-fallback";
    ph.textContent = "?";
    wrap.appendChild(ph);
  }
}

function lettersOnlyAlpha(s: string): string {
  return s.replace(/[^a-zA-Z]/g, "");
}

/**
 * Diminutif du **type** d’effet (catégorie), ex. Rochester Comp en Dynamics → « Dyn ».
 */
function abbrevEffectType(category: string, name: string): string {
  const key = normalizeCategory(category);
  if (key === "routing") {
    const n = name.toLowerCase();
    if (n.includes("merge")) return "Mrg";
    if (n.includes("split")) return "Spt";
    return "Rte";
  }
  const mapped = EFFECT_TYPE_ABBREV[key];
  if (mapped) return mapped;
  const raw = lettersOnlyAlpha(category);
  if (raw.length >= 3) return raw.slice(0, 3);
  if (raw.length > 0) return raw;
  return "???";
}

/** Infobulle : nom court catalogue si déjà résolu, sinon libellé Kempline/USB jusqu’à `refreshAllSlotTooltipsInContent`. */
function slotTooltipText(slot: SlotDebug): string {
  const hit = kemplineTooltipCache.get(tooltipCacheKey(slot));
  if (hit !== undefined) return hit;
  return slot.name.trim() || "—";
}

function isAmpCategory(category: string): boolean {
  const c = normalizeCategory(category);
  return c === "amp" || c === "preamp" || c === "amp+cab";
}

function isSingleDspDevice(name: string | null): boolean {
  if (!name) return false;
  return name.toLowerCase().includes("stomp");
}

function isEmptyGridCell(slot: SlotDebug): boolean {
  return !slot.category && slot.name === "<empty>";
}

/** Paire Path 1 (0..7) ↔ Path 2 (8..15) : même colonne visuelle. */
function pairedKemplineSlotIndex(kemplineSlotIndex: number): number | null {
  if (kemplineSlotIndex >= 0 && kemplineSlotIndex <= 7) return kemplineSlotIndex + 8;
  if (kemplineSlotIndex >= 8 && kemplineSlotIndex <= 15) return kemplineSlotIndex - 8;
  return null;
}

/** Slot vide non assignable : l'autre path de la même colonne porte déjà un bloc (max 1 model / colonne). */
function isColumnPairedSlotBlocked(slots: SlotDebug[], kemplineSlotIndex: number): boolean {
  const slot = slots[kemplineSlotIndex];
  if (!slot || !isEmptyGridCell(slot)) return false;
  const paired = pairedKemplineSlotIndex(kemplineSlotIndex);
  if (paired === null) return false;
  const other = slots[paired];
  return other !== undefined && !isEmptyGridCell(other);
}

function countRealBlocks(slots: SlotDebug[]): number {
  return slots.filter((s) => !isEmptyGridCell(s)).length;
}

/** Nombre de cases vides consécutives depuis le début d'une rangée (8 cases). */
function countLeadingEmptiesInRow(slots: SlotDebug[], rowStart: number, len: number): number {
  let n = 0;
  for (let i = 0; i < len; i += 1) {
    if (isEmptyGridCell(slots[rowStart + i])) n += 1;
    else break;
  }
  return n;
}

/** Dernier index occupé dans une rangée (0..len-1), ou -1 si tout vide. */
function lastFilledSlotRowIndex(slots: SlotDebug[], rowStart: number, len: number): number {
  for (let i = len - 1; i >= 0; i -= 1) {
    if (!isEmptyGridCell(slots[rowStart + i])) return i;
  }
  return -1;
}

/**
 * Colonnes 0..8 alignées sur les 8 slots visuels (après INPUT) : bordure verticale
 * du split après `splitCol` slots vides communs en tête ; merge après la dernière
 * colonne occupée sur A ou B (heuristique grille Kempline).
 */
function computeRoutingJunctionColumns(slots: SlotDebug[]): { splitCol: number; mergeCol: number } {
  const leadA = countLeadingEmptiesInRow(slots, 0, 8);
  const leadB = countLeadingEmptiesInRow(slots, 8, 8);
  const lastA = lastFilledSlotRowIndex(slots, 0, 8);
  const lastB = lastFilledSlotRowIndex(slots, 8, 8);
  const hasAnyB = lastB >= 0;

  let splitCol = hasAnyB ? Math.max(leadA, leadB) : leadA;
  splitCol = Math.min(8, Math.max(0, splitCol));

  const lastUsed = Math.max(lastA, lastB);
  let mergeCol = lastUsed < 0 ? 8 : Math.min(8, lastUsed + 1);
  if (mergeCol <= splitCol) mergeCol = Math.min(8, splitCol + 1);
  return { splitCol, mergeCol };
}

function makeNode(slot: SlotDebug, opts?: { showTypeAbbrev?: boolean }): HTMLElement {
  const showTypeAbbrev = opts?.showTypeAbbrev !== false;
  const item = document.createElement("div");
  item.className =
    "node node--hx-slot" + (normalizeCategory(slot.category) === "routing" ? " routing" : "");
  if (!showTypeAbbrev) item.classList.add("node--icon-only");
  item.dataset.slotCategory = slot.category;
  item.dataset.slotKemplineName = slot.name.trim();

  const tip = slotTooltipText(slot);
  item.setAttribute("aria-label", tip.replace(/\n/g, " — "));
  item.title = tip;

  const iconWrap = document.createElement("div");
  iconWrap.className = "node-icon-wrap";
  const iconPath = iconForCategory(slot.category, slot.name);
  if (iconPath) {
    const img = document.createElement("img");
    img.className = "node-icon-img";
    img.src = iconPath;
    img.alt = "";
    iconWrap.appendChild(img);
  } else {
    const ph = document.createElement("div");
    ph.className = "node-icon-fallback";
    ph.textContent = "?";
    iconWrap.appendChild(ph);
  }

  item.appendChild(iconWrap);

  if (showTypeAbbrev) {
    const abbr = document.createElement("div");
    abbr.className = "node-abbr";
    abbr.textContent = abbrevEffectType(slot.category, slot.name);
    item.appendChild(abbr);
  }

  bindSlotParamsInteraction(item, slot);
  return item;
}

function appendPipe(row: HTMLElement) {
  const pipe = document.createElement("div");
  pipe.className = "pipe";
  row.appendChild(pipe);
}

function appendNode(row: HTMLElement, slot: SlotDebug) {
  if (row.childElementCount > 0) appendPipe(row);
  row.appendChild(makeNode(slot));
}

function appendPlaceholder(row: HTMLElement) {
  if (row.childElementCount > 0) appendPipe(row);
  const placeholder = document.createElement("div");
  placeholder.className = "placeholder";
  row.appendChild(placeholder);
}

/** 16 cases : grille Kempline (8 + 8), slots vides = catégorie vide + nom `<empty>`. */
function isKemplineGrid16(slots: SlotDebug[]): boolean {
  if (slots.length !== 16) return false;
  return slots.every((s) => {
    const emptyCell = !s.category && s.name === "<empty>";
    const filled = s.category.length > 0;
    return emptyCell || filled;
  });
}

/** Slot vide matrice : cadre au survol / sélection sauf si colonne bloquée par l'autre path. */
function makeEmptySlotNode(opts?: { columnBlocked?: boolean }): HTMLElement {
  const item = document.createElement("div");
  item.className = "node node-empty node--hx-slot node-empty-flat";
  if (opts?.columnBlocked) {
    item.classList.add("node-empty-column-blocked");
    item.title = "Colonne déjà utilisée sur l'autre path";
    item.setAttribute("aria-label", "Slot indisponible — colonne déjà utilisée");
    item.setAttribute("aria-disabled", "true");
  } else {
    item.title = "Slot vide";
    item.setAttribute("aria-label", "Slot vide");
    bindSlotParamsInteraction(item, null);
  }
  return item;
}

const IO_INPUT_ICON = "/src-tauri/resources/icons_category/icon-input-category.png";
const IO_OUTPUT_ICON = "/src-tauri/resources/icons_category/icon-output-category.png";

function path1InputParentModelId(): string {
  return flowIoCatalogIdsForConnectedDevice(connectedDeviceName).input;
}

function resolvePath1InputIoSourceRow(): CatalogPickerModelRow | null {
  if (!catalogPickerDataCache) return null;
  const parentId = path1InputParentModelId();
  if (path1InputSourceHighlightOverride) {
    const fromOverride = findIoSourceRowById(catalogPickerDataCache, path1InputSourceHighlightOverride);
    if (fromOverride) return fromOverride;
  }
  if (path1InputMatrixWire != null && Number.isFinite(path1InputMatrixWire)) {
    const fromWire = findIoSourceRowByWireValue(
      catalogPickerDataCache,
      parentId,
      path1InputMatrixWire,
      connectedDeviceName,
    );
    if (fromWire) return fromWire;
  }
  return findIoSourceRowByWireValue(catalogPickerDataCache, parentId, 1, connectedDeviceName);
}

function queryPath1InputMatrixNode(): HTMLElement | null {
  return contentEl.querySelector<HTMLElement>('.hx-matrix-cell .hx-io[data-hw-slot-bus="0"]');
}

function makeIoSourceIconImg(row: CatalogPickerModelRow): HTMLImageElement | null {
  const safe = row.image ? sanitizeIconsModelsFilename(row.image) : null;
  if (!safe) return null;
  const img = document.createElement("img");
  img.className = "hx-io-icon";
  img.decoding = "async";
  img.alt = "";
  img.src = tauriResourceUrl("icons_models", safe);
  return img;
}

function applyIoSourceIconToInputNode(node: HTMLElement, row: CatalogPickerModelRow): void {
  const label = row.name.trim() || "Input";
  node.setAttribute("aria-label", label);
  node.title = label;
  const img = makeIoSourceIconImg(row);
  if (!img) return;
  const existing = node.querySelector(".hx-io-icon");
  if (existing) existing.replaceWith(img);
  else {
    node.querySelectorAll(".hx-io-icon").forEach((n) => n.remove());
    node.appendChild(img);
  }
}

async function refreshPath1InputMatrixIcon(): Promise<void> {
  const node = queryPath1InputMatrixNode();
  if (!node || !catalogPickerDataCache) return;
  const parentId = path1InputParentModelId();
  let row: CatalogPickerModelRow | null = null;

  if (path1InputSourceHighlightOverride) {
    row = findIoSourceRowById(catalogPickerDataCache, path1InputSourceHighlightOverride);
  }
  if (!row) {
    try {
      const liveWire = await invoke<number | null>("get_path1_input_source_wire_value");
      if (liveWire != null && Number.isFinite(liveWire)) {
        path1InputMatrixWire = liveWire;
        row = findIoSourceRowByWireValue(
          catalogPickerDataCache,
          parentId,
          liveWire,
          connectedDeviceName,
        );
      }
    } catch {
      /* wire live optionnel */
    }
  }
  if (!row && path1InputMatrixWire != null) {
    row = findIoSourceRowByWireValue(
      catalogPickerDataCache,
      parentId,
      path1InputMatrixWire,
      connectedDeviceName,
    );
  }
  if (!row) {
    row = findIoSourceRowByWireValue(catalogPickerDataCache, parentId, 1, connectedDeviceName);
  }
  if (!row?.image) return;
  if (typeof row.wireValue === "number") path1InputMatrixWire = row.wireValue;
  applyIoSourceIconToInputNode(node, row);
}

/** Nœuds d'extrémité façon HX Edit (icônes Input / Main L·R). */
function makeIoNode(kind: "input" | "output", inputSourceRow?: CatalogPickerModelRow | null): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-io hx-io--icon";
  if (kind === "input") {
    const row = inputSourceRow ?? resolvePath1InputIoSourceRow();
    const img = row ? makeIoSourceIconImg(row) : null;
    if (img) {
      el.appendChild(img);
      const label = row!.name.trim() || "Input";
      el.setAttribute("aria-label", label);
      el.title = label;
    } else {
      const fallback = document.createElement("img");
      fallback.className = "hx-io-icon";
      fallback.decoding = "async";
      fallback.src = IO_INPUT_ICON;
      fallback.alt = "Input";
      el.setAttribute("aria-label", "Input");
      el.title = "Input";
      el.appendChild(fallback);
    }
  } else {
    const img = document.createElement("img");
    img.className = "hx-io-icon hx-io-icon--output";
    img.decoding = "async";
    img.src = IO_OUTPUT_ICON;
    img.alt = "Main L/R";
    el.setAttribute("aria-label", "Main L/R");
    el.title = "Main L/R";
    el.appendChild(img);
  }
  return el;
}

/** Texte infobulle / aria pour une jonction split ou merge en matrice. */
function routingMatrixTooltip(kind: "split" | "merge", detailTitle: string): string {
  const label = kind === "split" ? "Split" : "Merge";
  const d = detailTitle.replace(/\n/g, " ").trim();
  if (!d || d === label) return label;
  return `${label} — ${d}`;
}

function path1SeparatorSlot(
  boundary: number,
  kind: "split" | "merge" | null,
  marker?: RoutingMarker,
): SlotDebug {
  const markerHex = (marker?.moduleHex ?? "").trim();
  const markerName = (marker?.name ?? "").trim();
  if (kind === "split") {
    return {
      category: "Split",
      name: markerName || `Split (Path 1 #${boundary})`,
      moduleHex: markerHex,
    };
  }
  if (kind === "merge") {
    return {
      category: "Merge",
      name: markerName || `Merge (Path 1 #${boundary})`,
      moduleHex: markerHex,
      catalogModelId: FLOW_JOIN_CATALOG_ID,
    };
  }
  return { category: "Routing", name: `Separator (Path 1 #${boundary})`, moduleHex: "" };
}

type FlowIoCatalogIds = {
  input: string;
  output: string;
};

const FLOW_IO_IDS_STOMP: FlowIoCatalogIds = {
  input: "HelixStomp_AppDSPFlowInput",
  output: "HelixStomp_AppDSPFlowOutputMain",
};

const FLOW_IO_IDS_HX_FX: FlowIoCatalogIds = {
  input: "HelixFx_AppDSPFlowInput",
  output: "HelixFx_AppDSPFlowOutput",
};

const FLOW_IO_IDS_HD2: FlowIoCatalogIds = {
  input: "HD2_AppDSPFlow1Input",
  output: "HD2_AppDSPFlowOutput",
};

/**
 * **Mixer / Merge** (`HD2_AppDSPFlowJoin`) : `presetMeta.chainHex` est vide dans le catalogue HX ;
 * le parseur flux n’infère pas encore d’hex `19…1a` pour le segment merge (`0x03`, fenêtre Kempline index 19).
 * Jointure par **id catalogue** fixe, comme l’I/O Path 1 (les valeurs chaîne viennent tout de même de
 * `get_active_preset_kempline_flow_chain_param_values`).
 */
const FLOW_JOIN_CATALOG_ID = "HD2_AppDSPFlowJoin";

/** Choix explicite de l'ID catalogue I/O selon la machine détectée (pas de chainHex pour ces slots). */
function flowIoCatalogIdsForConnectedDevice(name: string | null): FlowIoCatalogIds {
  const n = (name ?? "").toLowerCase();
  if (n.includes("stomp")) return FLOW_IO_IDS_STOMP;
  if (n.includes("effects") || n.includes("hx fx")) return FLOW_IO_IDS_HX_FX;
  return FLOW_IO_IDS_HD2;
}

function path1IoSlot(kind: "input" | "output"): SlotDebug {
  const flowIds = flowIoCatalogIdsForConnectedDevice(connectedDeviceName);
  if (kind === "input") {
    return {
      category: "Input",
      name: "Input",
      moduleHex: "",
      // Kempline `slot_type` Path 1: 00 = Input upper.
      slotTypeHex: "00",
      catalogModelId: flowIds.input,
    };
  }
  return {
    category: "Output",
    name: "Output",
    moduleHex: "",
    // Kempline `slot_type` Path 1: 01 = Output upper.
    slotTypeHex: "01",
    catalogModelId: flowIds.output,
  };
}

/** Colonne grille paire (2,4,…,18) pour une frontière split/merge 1..8 ; `0` → après Input (col 2). */
function matrixEvenColForRoutingBoundary(boundary: number): number {
  if (boundary === 0) return 2;
  if (boundary < 1 || boundary > 8) return -1;
  return 4 + 2 * (boundary - 1);
}

/** Split / merge sur « Path 1 » (L1) : cercle centré (CSS) ; `title` sur la cellule grille. */
function makePathRoutingNode(kind: "split" | "merge"): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = `hx-matrix-routing-marker hx-matrix-path1-separator hx-matrix-routing-marker--${kind}`;
  wrap.dataset.routingMarker = kind;
  return wrap;
}

function gridSlotNode(
  slot: SlotDebug,
  kemplineSlotIndex: number,
  allSlots?: SlotDebug[],
): HTMLElement {
  const empty = !slot.category && slot.name === "<empty>";
  if (empty) {
    const columnBlocked =
      allSlots !== undefined &&
      allSlots.length === 16 &&
      isColumnPairedSlotBlocked(allSlots, kemplineSlotIndex);
    const n = makeEmptySlotNode({ columnBlocked });
    n.dataset.kemplineSlotIndex = String(kemplineSlotIndex);
    if (!columnBlocked) {
      bindMatrixSlotDropTarget(n, kemplineSlotIndex);
    }
    return n;
  }
  /* Matrice : sur « Path 1 » / « Path 2 », la catégorie est sur la ligne Description ; la cellule slot = icône + infobulle nom. */
  const node = makeNode(slot, { showTypeAbbrev: false });
  node.dataset.kemplineSlotIndex = String(kemplineSlotIndex);
  bindMatrixSlotDragSource(node, slot, kemplineSlotIndex);
  return node;
}

/**
 * Grille 16 cases Kempline : matrice **2 lignes × 19 colonnes** (Path 1 + Path 2).
 */
function renderGrid16(
  slots: SlotDebug[],
  routing: RoutingMarker[],
  stompLayout: ActivePresetStompLayout | null,
) {
  const lastB = lastFilledSlotRowIndex(slots, 8, 8);
  const hasBranchB = lastB >= 0;
  /** Split/merge path1, path2 : seulement si au moins un bloc path2. */
  const showRoutingUi = hasBranchB;

  const routingCols =
    stompLayout != null && stompLayout.routing.kemplineGridOk === true
      ? {
          splitCol: stompLayout.routing.splitAfterCol,
          mergeCol: stompLayout.routing.mergeAfterCol,
        }
      : computeRoutingJunctionColumns(slots);

  const splitEntry = routing.find((m) => m.name.toLowerCase().includes("split"));
  const mergeEntry = routing.find((m) => m.name.toLowerCase().includes("merge"));
  const splitTip = splitEntry ? `${splitEntry.category}: ${splitEntry.name}` : "Split";
  const mergeTip = mergeEntry ? `${mergeEntry.category}: ${mergeEntry.name}` : "Merge";

  const root = document.createElement("div");
  root.className = "flow grid16 hx-edit-chain hx-matrix";

  const grid = document.createElement("div");
  grid.className = "hx-matrix-grid";

  const LINE_PATH_1 = 1;
  const LINE_PATH_2 = 2;
  const NUM_ROWS = 2;
  const NUM_COLS = 19;

  /*
   * ─── REVERT : matrice 5 lignes + rangée 3 « séparateur » ───
   * const LINE_SEPARATOR = 3; const NUM_ROWS = 5;
   * Puis rétablir la boucle `if (row === LINE_SEPARATOR) { ... }`,
   * `v.style.gridRow = "3 / 5"`, et dans wrapCell : `if (row === LINE_SEPARATOR) cls += " hx-matrix-cell--row-line-debug-sep"`.
   * CSS : 5 rangées + décommenter `.hx-matrix-separator-bar` et `.hx-matrix-cell--row-line-debug-sep`.
   */

  const path1Rail = document.createElement("div");
  path1Rail.className = "hx-matrix-path1-rail";
  path1Rail.setAttribute("role", "presentation");
  path1Rail.setAttribute("aria-hidden", "true");
  grid.appendChild(path1Rail);

  const splitG = showRoutingUi ? matrixEvenColForRoutingBoundary(routingCols.splitCol) : -1;
  const mergeG = showRoutingUi ? matrixEvenColForRoutingBoundary(routingCols.mergeCol) : -1;
  if (showRoutingUi && hasBranchB && splitG >= 2 && mergeG >= splitG) {
    const path2Rail = document.createElement("div");
    path2Rail.className = "hx-matrix-path2-rail";
    path2Rail.setAttribute("role", "presentation");
    path2Rail.setAttribute("aria-hidden", "true");
    path2Rail.style.gridRow = String(LINE_PATH_2);
    path2Rail.style.gridColumn = `${splitG} / ${mergeG + 1}`;
    grid.appendChild(path2Rail);
  }

  function wrapCell(row: number, col: number, inner: HTMLElement | null): HTMLElement {
    const w = document.createElement("div");
    const cls = "hx-matrix-cell" + (inner ? "" : " hx-matrix-cell--empty");
    w.className = cls;
    w.style.gridRow = String(row);
    w.style.gridColumn = String(col);
    if (inner) w.appendChild(inner);
    const rk = inner?.dataset?.routingMarker;
    if (rk === "split" || rk === "merge") {
      const tip = routingMatrixTooltip(rk, rk === "split" ? splitTip : mergeTip);
      w.title = tip;
      w.setAttribute("aria-label", tip);
    }
    return w;
  }

  function routingAtBoundary(boundary: number): HTMLElement | null {
    if (!showRoutingUi) return null;
    const { splitCol, mergeCol } = routingCols;
    if (boundary < 1 || boundary > 8) return null;
    if (mergeCol === boundary) return makePathRoutingNode("merge");
    if (splitCol === boundary) return makePathRoutingNode("split");
    return null;
  }

  function routingKindAtBoundary(boundary: number): "split" | "merge" | null {
    if (!showRoutingUi) return null;
    const { splitCol, mergeCol } = routingCols;
    if (boundary < 1 || boundary > 8) return null;
    if (mergeCol === boundary) return "merge";
    if (splitCol === boundary) return "split";
    return null;
  }

  const junctionDecoCols = new Set<number>();
  if (splitG >= 2) junctionDecoCols.add(splitG);
  if (mergeG >= 2) junctionDecoCols.add(mergeG);

  if (showRoutingUi) {
    for (const col of junctionDecoCols) {
      const junctionVRail = document.createElement("div");
      junctionVRail.className = "hx-matrix-junction-vrail";
      junctionVRail.setAttribute("role", "presentation");
      junctionVRail.setAttribute("aria-hidden", "true");
      junctionVRail.style.gridColumn = String(col);
      grid.appendChild(junctionVRail);
    }
  }

  for (let row = 1; row <= NUM_ROWS; row += 1) {
    /*
     * REVERT (rangée séparateur ligne 3) :
     * if (row === LINE_SEPARATOR) {
     *   const bar = document.createElement("div");
     *   bar.className = "hx-matrix-separator-bar";
     *   bar.setAttribute("role", "presentation");
     *   bar.setAttribute("aria-hidden", "true");
     *   bar.style.gridRow = String(LINE_SEPARATOR);
     *   bar.style.gridColumn = "1 / -1";
     *   grid.appendChild(bar);
     *   grid.appendChild(
     *     wrapCell(LINE_SEPARATOR, NUM_COLS, makeMatrixRowLineLabel(LINE_SEPARATOR), { rowLineDebug: true }),
     *   );
     *   continue;
     * }
     */

    for (let col = 1; col <= NUM_COLS; col += 1) {
      let inner: HTMLElement | null = null;

      if (col === 1) {
        if (row === LINE_PATH_1) {
          inner = makeIoNode("input", resolvePath1InputIoSourceRow());
          inner.dataset.hwSlotBus = "0";
          bindSlotParamsInteraction(inner, path1IoSlot("input"));
        }
      } else if (col === 2) {
        if (row === LINE_PATH_1) {
          const rk: "split" | "merge" | null =
            showRoutingUi && routingCols.splitCol === 0 ? "split" : null;
          if (rk === "split") {
            inner = makePathRoutingNode("split");
            inner.dataset.hwSlotBus = "10";
            bindSlotParamsInteraction(
              inner,
              path1SeparatorSlot(0, rk, splitEntry),
            );
          }
        }
      } else if (col === 19) {
        if (row === LINE_PATH_1) {
          inner = makeIoNode("output");
          inner.dataset.hwSlotBus = "9";
          bindSlotParamsInteraction(inner, path1IoSlot("output"));
        }
      } else if (col >= 3 && col <= 17 && (col - 3) % 2 === 0) {
        const i = (col - 3) / 2;
        if (i >= 0 && i <= 7) {
          if (row === LINE_PATH_1)
            inner = gridSlotNode(slots[i]!, i, slots);
          else if (row === LINE_PATH_2 && hasBranchB)
            inner = gridSlotNode(slots[8 + i]!, 8 + i, slots);
        }
      } else if (col >= 4 && col <= 18 && (col - 4) % 2 === 0) {
        const j = (col - 4) / 2;
        if (row === LINE_PATH_1 && j >= 0 && j <= 7) {
          const boundary = j + 1;
          const rk = routingKindAtBoundary(boundary);
          inner = routingAtBoundary(boundary);
          if (inner !== null && rk !== null) {
            if (rk === "split") inner.dataset.hwSlotBus = "10";
            else if (rk === "merge") inner.dataset.hwSlotBus = "19";
            bindSlotParamsInteraction(
              inner,
              path1SeparatorSlot(
                boundary,
                rk,
                rk === "split" ? splitEntry : rk === "merge" ? mergeEntry : undefined,
              ),
            );
          }
        }
      }

      grid.appendChild(wrapCell(row, col, inner));
    }
  }

  root.appendChild(grid);

  clearSlotSelectionVisual();
  emitModelsSyncTrace(
    `renderGrid16 innerHTML clear preset=${currentPresetIndex} slots=${slots.length} loaded=${loadedPresetIndex}`,
  );
  contentEl.innerHTML = "";
  contentEl.appendChild(root);
  const hadSelectedContext = hasSelectedParamsContextForCurrentPreset();
  if (
    !tryRestoreSelectedParamsPaneAfterRender() &&
    !hadSelectedContext
  ) {
    resetModelsParamsIdleHint();
    armAutoSelectFallbackParamsPaneAfterRender();
  }
  consumePendingHardwareSlotSelection();
  void refreshPath1InputMatrixIcon();
}

function buildFlowSections(slots: SlotDebug[]) {
  const splitIdx = slots.findIndex(
    (s) => normalizeCategory(s.category) === "routing" && s.name.toLowerCase().includes("split"),
  );
  const mergeIdx = slots.findIndex(
    (s, i) =>
      i > splitIdx &&
      normalizeCategory(s.category) === "routing" &&
      s.name.toLowerCase().includes("merge"),
  );

  if (splitIdx < 0 || mergeIdx < 0) {
    return {
      pre: slots,
      split: null as SlotDebug | null,
      branchA: [] as SlotDebug[],
      branchB: [] as SlotDebug[],
      merge: null as SlotDebug | null,
      post: [] as SlotDebug[],
      hasSplit: false,
    };
  }

  const pre = slots.slice(0, splitIdx).filter((s) => normalizeCategory(s.category) !== "routing");
  const split = slots[splitIdx];
  const beforeMerge = slots
    .slice(splitIdx + 1, mergeIdx)
    .filter((s) => normalizeCategory(s.category) !== "routing");
  const afterMerge = slots
    .slice(mergeIdx + 1)
    .filter((s) => normalizeCategory(s.category) !== "routing");
  const merge = slots[mergeIdx];

  const ampsBefore = beforeMerge.filter((s) => isAmpCategory(s.category));
  const ampsAfter = afterMerge.filter((s) => isAmpCategory(s.category));

  let branchAAnchor: SlotDebug | null = ampsBefore[0] ?? null;
  let branchBAnchor: SlotDebug | null = null;
  if (ampsBefore.length >= 2) {
    branchBAnchor = ampsBefore[1];
  } else if (ampsAfter.length >= 1) {
    // Cas fréquent : l'ampli B est sérialisé après le Merge avec la chaîne post-merge avant lui.
    branchBAnchor = ampsAfter[0];
  }

  let mainPost: SlotDebug[] = [];
  let branchBFromAfter: SlotDebug[] = [];
  if (branchBAnchor && afterMerge.includes(branchBAnchor)) {
    const bi = afterMerge.indexOf(branchBAnchor);
    mainPost = afterMerge.slice(0, bi);
    branchBFromAfter = afterMerge.slice(bi);
  } else {
    mainPost = afterMerge.slice();
  }

  const branchA: SlotDebug[] = [];
  const branchB: SlotDebug[] = [];

  const bBeforeIdx = branchBAnchor ? beforeMerge.indexOf(branchBAnchor) : -1;

  for (const slot of beforeMerge) {
    if (branchAAnchor && slot === branchAAnchor) {
      branchA.push(slot);
      continue;
    }
    if (branchBAnchor && slot === branchBAnchor) {
      branchB.push(slot);
      continue;
    }
    if (bBeforeIdx >= 0) {
      const idx = beforeMerge.indexOf(slot);
      if (idx > bBeforeIdx) branchB.push(slot);
      else branchA.push(slot);
    } else {
      branchA.push(slot);
    }
  }

  branchB.push(...branchBFromAfter);

  return { pre, split, branchA, branchB, merge, post: mainPost, hasSplit: true };
}

async function renderSlots(
  rawSlots: SlotDebug[],
  routingFromFlow: RoutingMarker[] = [],
  stompLayout: ActivePresetStompLayout | null = null,
) {
  if (rawSlots.length === 0) {
    if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length > 0) {
      emitModelsSyncTrace(
        `renderSlots skip renderEmpty keep snapshot len=${lastHwSyncNormalizedSlots.length}`,
      );
      return;
    }
    renderEmpty("Aucun bloc detecte dans ce preset.");
    return;
  }

  const slots: SlotDebug[] = rawSlots;
  if (isKemplineGrid16(slots)) {
    renderGrid16(slots, routingFromFlow, stompLayout);
    await refreshAllSlotTooltipsInContent();
    return;
  }

  const flow = buildFlowSections(slots);

  const root = document.createElement("div");
  root.className = "flow";

  const rowMain = document.createElement("div");
  rowMain.className = "flow-row";

  // Split/Merge viennent du backend comme marqueurs synthétiques (grille),
  // pas comme blocs HX Edit. S'il n'y a aucun bloc sur la branche B, on
  // affiche une chaîne linéaire comme dans l'UI Line 6.
  const showSyntheticRouting = flow.hasSplit && flow.branchB.length > 0;

  for (const slot of flow.pre) appendNode(rowMain, slot);
  if (showSyntheticRouting && flow.split) appendNode(rowMain, flow.split);
  for (const slot of flow.branchA) appendNode(rowMain, slot);
  if (showSyntheticRouting && flow.merge) appendNode(rowMain, flow.merge);
  for (const slot of flow.post) appendNode(rowMain, slot);

  root.appendChild(rowMain);

  if (flow.hasSplit && flow.branchB.length > 0) {
    const rowBranch = document.createElement("div");
    rowBranch.className = "flow-row";
    for (let i = 0; i < flow.pre.length + (flow.split ? 1 : 0); i += 1) {
      appendPlaceholder(rowBranch);
    }
    for (const slot of flow.branchB) appendNode(rowBranch, slot);
    root.appendChild(rowBranch);
  }

  clearSlotSelectionVisual();
  emitModelsSyncTrace(
    `renderSlots(flow) innerHTML clear preset=${currentPresetIndex} rawSlots=${rawSlots.length}`,
  );
  contentEl.innerHTML = "";
  contentEl.appendChild(root);
  const hadSelectedContext = hasSelectedParamsContextForCurrentPreset();
  if (
    !tryRestoreSelectedParamsPaneAfterRender() &&
    !hadSelectedContext
  ) {
    resetModelsParamsIdleHint();
    armAutoSelectFallbackParamsPaneAfterRender();
  }
  consumePendingHardwareSlotSelection();
  await refreshAllSlotTooltipsInContent();
}

async function requestLoadForPreset(index: number) {
  if (!ENABLE_PRESET_CONTENT) {
    hardwareSyncPausedForPresetLoad = false;
    loading = false;
    loadedPresetIndex = index;
    // On laisse la fenêtre models "inerte" mais on évite les appels backend.
    renderEmpty("Lecture du preset désactive (debug).");
    return;
  }
  if (loading) {
    pendingPresetIndex = index;
    console.log(`[PresetDebug][models] queued preset=${index} while loading`);
    return;
  }
  const cooldownRemainingMs = requestPresetCooldownRemainingMs();
  if (cooldownRemainingMs > 0) {
    pendingPresetIndex = index;
    armQueuedPresetLoadAfterCooldown();
    return;
  }
  const initSettling = await invoke<boolean>("is_helix_usb_init_settling").catch(() => false);
  if (initSettling) {
    pendingPresetIndex = index;
    emitModelsSyncTraceThrottled(
      "preset_load_defer_init_settle",
      `requestLoadForPreset reporté (init USB ~700 ms) preset=${index}`,
      2_000,
    );
    armQueuedPresetLoadAfterCooldown();
    return;
  }

  loading = true;
  if (presetLoadUiLockDepth === 0) {
    pushPresetLoadUiLock();
  }
  hardwareSyncPausedForPresetLoad = true;
  if (index !== currentPresetIndex) {
    clearPath1InputSourceHighlightOverride();
  }
  mergeProbeSlotModelUntil = null;
  suppressUsbPresetPollUntilMs = 0;
  pendingPresetIndex = -1;
  lastRequestedPresetIndex = index;
  // Sentinel : le premier soft-sync aligne la séquence HW sans déclencher un dump USB « artificiel »
  // (voir applyHardwareSlotStateFromBackend). Le chargement peut aussi écraser via hwSnap.
  lastSeenHardwareSlotSequence = -1;
  startLoadingHeartbeat("Lecture du preset actif");
  console.log(`[PresetDebug][models] request_preset_content preset=${index}`);
  emitModelsSyncTrace(
    `requestLoadForPreset start index=${index} currentPreset=${currentPresetIndex} loaded=${loadedPresetIndex}`,
  );

  await waitForMatrixUsbIdle();
  await settleUsbAfterMatrixProbe();

  try {
    lastRequestPresetInvokeAt = Date.now();
    await invoke("request_preset_content");
  } catch (e) {
    const msg = String(e);
    if (msg.includes("throttled")) {
      pendingPresetIndex = index;
      loading = false;
      popPresetLoadUiLock();
      hardwareSyncPausedForPresetLoad = false;
      emitModelsSyncTraceThrottled(
        "preset_load_defer_throttle",
        `requestLoadForPreset reporté (throttle Rust) preset=${index}`,
        1_000,
      );
      armQueuedPresetLoadAfterCooldown();
      return;
    }
    if (
      msg.includes("molette modèle") ||
      msg.includes("scroll/pull") ||
      msg.includes("Initialisation USB")
    ) {
      pendingPresetIndex = index;
      loading = false;
      popPresetLoadUiLock();
      hardwareSyncPausedForPresetLoad = false;
      emitModelsSyncTraceThrottled(
        msg.includes("Initialisation USB") ? "preset_load_defer_init_settle" : "preset_load_defer_hw_scroll",
        msg.includes("Initialisation USB")
          ? `requestLoadForPreset reporté (init USB) preset=${index} -> retry`
          : `requestLoadForPreset reporté (scroll HW) preset=${index} -> retry après cooldown`,
        2_000,
      );
      armQueuedPresetLoadAfterCooldown();
      return;
    }
    console.error("[PresetDebug][models] request_preset_content error", e);
    loading = false;
    pendingPresetIndex = currentPresetIndex >= 0 ? currentPresetIndex : index;
    hardwareSyncPausedForPresetLoad = true;
    armRecoveryPresetLoad("invoke request_preset_content");
    return;
  }

  let tries = 0;
  const timer = window.setInterval(async () => {
    tries += 1;
    if (tries === REQUEST_PRESET_SOFT_STALL_TRIES) {
      // Si la lecture n'a toujours pas livré de slots après plusieurs cycles,
      // on anticipe la récupération pour éviter l'impression de "freeze".
      window.clearInterval(timer);
      console.warn(`[PresetDebug][models] soft-stall preset=${index}`);
      loading = false;
      pendingPresetIndex = currentPresetIndex >= 0 ? currentPresetIndex : index;
      hardwareSyncPausedForPresetLoad = true;
      armRecoveryPresetLoad("lecture en attente");
      return;
    }
    if (tries > 45) {
      window.clearInterval(timer);
      console.warn(`[PresetDebug][models] timeout preset=${index}`);
      loading = false;
      pendingPresetIndex = currentPresetIndex >= 0 ? currentPresetIndex : index;
      hardwareSyncPausedForPresetLoad = true;
      armRecoveryPresetLoad("timeout lecture preset");
      return;
    }

    try {
      const slots = debugRoutingMode
        ? await invoke<[string, string, string, string][] | null>("get_active_preset_slots_debug")
        : await invoke<[string, string][] | null>("get_active_preset_slots");
      if (slots !== null) {
        const normalizedSlots = normalizeSlotsPayloadFromInvoke(slots as never);
        if (normalizedSlots.length === 0) {
          return;
        }
        window.clearInterval(timer);
        console.log(`[PresetDebug][models] slots ready preset=${index} count=${slots.length}`);
        emitModelsSyncTrace(
          `requestLoadForPreset slotsReady index=${index} count=${slots.length} tries=${tries}`,
        );
        recoveryAttemptCount = 0;
        stopLoadingHeartbeat();
        loadedPresetIndex = index;
        clearPath1InputSourceHighlightOverride();
        lastSoftUsbPresetReadAt = Date.now();
        // Evite d'afficher une vieille réponse si l'utilisateur a recliqué ailleurs.
        if (currentPresetIndex === index) {
          let routingFlow: RoutingMarker[] = [];
          let stompLayout: ActivePresetStompLayout | null = null;
          if (isKemplineGrid16(normalizedSlots)) {
            try {
              const r = await invoke<[string, string, string][] | null>("get_active_preset_routing_markers");
              routingFlow =
                r?.map(([category, name, moduleHex]) => ({
                  category,
                  name,
                  moduleHex: moduleHex?.trim() || undefined,
                })) ?? [];
            } catch {
              console.warn("[PresetDebug][models] get_active_preset_routing_markers error");
            }
            try {
              stompLayout = await invoke<ActivePresetStompLayout | null>("get_active_preset_stomp_layout");
            } catch {
              console.warn("[PresetDebug][models] get_active_preset_stomp_layout error");
            }
          }
          // Snapshot du slot HW actif juste avant le rendu :
          // renderSlots appelle consumePendingHardwareSlotSelection() en fin d'exécution.
          // Si pendingHardwareSelectedKemplineSlotIndex est renseigné ici, le bon slot
          // est sélectionné immédiatement sans passer par le fallback slot-1 à 240ms.
          if (pendingHardwareSelectedKemplineSlotIndex === null && pendingHardwareSelectedSlotBus === null) {
            try {
              const hwSnap = await invoke<HardwareActiveSlotState>("get_active_hardware_slot_state");
              if (hwSnap && Number.isInteger(hwSnap.slotIndex) && (hwSnap.slotIndex as number) >= 0) {
                pendingHardwareSelectedKemplineSlotIndex = hwSnap.slotIndex as number;
                lastSeenHardwareSlotSequence = hwSnap.sequence;
              } else if (hwSnap && Number.isInteger(hwSnap.slotBus)) {
                pendingHardwareSelectedSlotBus = hwSnap.slotBus as number;
                lastSeenHardwareSlotSequence = hwSnap.sequence;
              }
            } catch {
              // best effort : si l'invoke échoue, le fallback 240ms prend le relais
            }
          }
          try {
            await logCatalogChainHexDiffIfNeeded(normalizedSlots, index);
            await renderSlots(normalizedSlots, routingFlow, stompLayout);
            rememberHwSyncChainLayout(normalizedSlots);
            void hydrateSlotChainSessionFromPresetData(index);
            void hydrateSlotDualPartsSessionFromPresetData(index);
            const realBlocks = countRealBlocks(normalizedSlots);
            const singleDsp = isSingleDspDevice(connectedDeviceName);
            const dspSuffix = singleDsp ? ` (${realBlocks}/8 max)` : "";
            const overLimit =
              singleDsp && realBlocks > 8
                ? " - warning: parsed blocks exceed Stomp DSP budget"
                : "";
            setStatus(
              debugRoutingMode
                ? `${realBlocks} blocks detected${dspSuffix} (debug routing ON)${overLimit}`
                : `${realBlocks} blocks detected${dspSuffix}${overLimit}`,
            );
          } catch (e) {
            console.error("[PresetDebug][models] renderSlots error preset=", index, e);
            setStatus("Erreur affichage preset (voir console).");
          }

          // Le nom affiché est piloté par la liste presets backend (refresh global).
          // On évite ici un invoke additionnel request_active_preset_name qui peut
          // amplifier les rafales pendant les changements rapides de preset.
        }
        const chainNext = pendingPresetIndex >= 0 && pendingPresetIndex !== loadedPresetIndex;
        loading = false;
        hardwareSyncPausedForPresetLoad = chainNext;
        if (!chainNext) {
          popPresetLoadUiLock();
        }
        if (chainNext) {
          const next = pendingPresetIndex;
          pendingPresetIndex = -1;
          void requestLoadForPreset(next);
        }
      }
    } catch {
      console.warn("[PresetDebug][models] transient get_active_preset_slots error");
    }
  }, 200);
}

function scheduleLoadForPreset(index: number, force = false) {
  if (index < 0) return;
  if (!ENABLE_PRESET_CONTENT) {
    return;
  }
  // Evite les retriggers continus sur le meme preset.
  if (!force && (index === loadedPresetIndex || index === lastRequestedPresetIndex)) {
    return;
  }
  void requestLoadForPreset(index);
}

async function refresh() {
  try {
    connectedDeviceName = await invoke<string | null>("get_connected_device_name");
    if (!connectedDeviceName) {
      if (currentPresetIndex >= 0 || loadedPresetIndex >= 0) {
        purgeModelsUi();
      } else {
        connectedDeviceName = null;
        setStatus("HX non connecté.");
        presetLabelEl.textContent = "--";
        renderEmpty("En attente du HX...");
      }
      return;
    }
    const names = await invoke<string[]>("get_preset_names");
    const active = await invoke<number>("get_active_preset");

    if (active < 0 || active >= names.length) {
      console.warn("[PresetDebug][models] active preset out of range", active, names.length);
      stopLoadingHeartbeat();
      presetLabelEl.textContent = "--";
      renderEmpty("Aucun preset actif.");
      setStatus("En attente...");
      return;
    }

    const displayName = isEmpty(names[active]) ? "empty" : names[active];
    presetLabelEl.textContent = `${padNum(active)} ${displayName}`;
    lastPresetNamesSig = `${active}\n${names.join("\n")}`;

    if (active !== currentPresetIndex) {
      // Au premier `refresh` après ouverture Models, `currentPresetIndex` vaut encore **-1** :
      // attendre 2 constats « drift » bloquait tout `scheduleLoadForPreset` (return avant le bloc
      // du bas). Ce n’était pas lié au poll preset — le poll ne faisait que re-dumps plus tard.
      const presetUiUnset = currentPresetIndex < 0;
      if (!presetUiUnset) {
        mainWindowPresetDriftStreak += 1;
        if (mainWindowPresetDriftStreak < 2) {
          return;
        }
      }
      mainWindowPresetDriftStreak = 0;
      console.log(`[PresetDebug][models] active preset changed ${currentPresetIndex} -> ${active}`);
      emitModelsSyncTrace(
        `refresh presetDrift CONFIRMED uiWas=${currentPresetIndex} backendActive=${active} -> renderEmpty+scheduleLoad`,
      );
      currentPresetIndex = active;
      loadedPresetIndex = -1;
      clearSelectedParamsContext();
      renderEmpty("Chargement des modeles...");
      const initSettling = await invoke<boolean>("is_helix_usb_init_settling").catch(() => false);
      if (!initSettling) {
        scheduleLoadForPreset(active, true);
      } else {
        pendingPresetIndex = active;
        armQueuedPresetLoadAfterCooldown();
      }
    } else {
      mainWindowPresetDriftStreak = 0;
    }
    const initSettling = await invoke<boolean>("is_helix_usb_init_settling").catch(() => false);
    if (!initSettling && !loading && loadedPresetIndex !== currentPresetIndex) {
      scheduleLoadForPreset(currentPresetIndex, false);
    }
  } catch {
    console.warn("[PresetDebug][models] refresh failed (HX disconnected?)");
    stopLoadingHeartbeat();
    setStatus("HX non connecte.");
    presetLabelEl.textContent = "--";
    renderEmpty("En attente du HX...");
  }
}

/** Console : rejeu HX Edit replace Cab 2 (octets figés). */
async function change_cab2(slotIndex = 0): Promise<string> {
  return invoke<string>("hx_console_change_cab2", { slotIndex });
}

declare global {
  interface Window {
    change_cab2?: typeof change_cab2;
  }
}

window.change_cab2 = change_cab2;

window.addEventListener("DOMContentLoaded", () => {
  hwUi.configure({ setParamsBrowsingMode: setModelsParamsBrowsingMode });
  void mountModelsSlotPicker();
  initMatrixContextMenu();
  initMatrixDragDrop();

  window.addEventListener("keydown", (e) => {
    if (e.ctrlKey && e.altKey && (e.key === "d" || e.key === "D")) {
      debugRoutingMode = !debugRoutingMode;
      localStorage.setItem("models_debug_routing", debugRoutingMode ? "1" : "0");
      setStatus(debugRoutingMode ? "Mode debug routing active" : "Mode debug routing desactive");
      loadedPresetIndex = -1;
      if (currentPresetIndex >= 0) {
        renderEmpty("Rechargement des modeles...");
        scheduleLoadForPreset(currentPresetIndex, true);
      }
    }
  });

  void listen<{ index: number }>("models:load-preset", async (event) => {
    const index = event.payload?.index;
    if (typeof index !== "number" || index < 0) return;
    console.log(`[PresetDebug][models] event models:load-preset index=${index}`);
    if (index === currentPresetIndex) {
      // Polling/sync backend: ne pas casser la sélection locale si le preset n'a pas changé.
      if (!loading && loadedPresetIndex !== currentPresetIndex) {
        scheduleLoadForPreset(currentPresetIndex, false);
      }
      return;
    }
    currentPresetIndex = index;
    loadedPresetIndex = -1;
    mergeProbeSlotModelUntil = null;
    suppressUsbPresetPollUntilMs = 0;
    clearSelectedParamsContext();
    renderEmpty("Chargement des modeles...");
    scheduleLoadForPreset(index, true);
  });

  void listen<HardwareSlotChangedPayload>("models:hardware-slot-changed", (event) => {
    const p = event.payload;
    if (hwSlotDebugEnabled() || modelsSyncTraceEnabled()) {
      console.info("[HxLinux] models:hardware-slot-changed", p);
    }
    if (p && typeof p.sequence === "number") {
      emitModelsSyncTraceThrottled(
        "evt_hw_slot_changed",
        `event models:hardware-slot-changed seq=${p.sequence} slotIdx=${p.slotIndex} slotBus=${p.slotBus}`,
        2_000,
      );
    }
    const notifyBus =
      p && typeof p.slotBus === "number" && Number.isFinite(p.slotBus) ? p.slotBus : null;
    if (notifyBus !== null && pickerCategoryForHwSlotBus(notifyBus)) {
      selectedSpecialHwSlotBus = notifyBus;
      void mountModelsSlotPicker().then(() => {
        lockPickerCategoryFromHwSlotBus(notifyBus);
        if (notifyBus === HW_SLOT_BUS_SPLIT) {
          void refreshSplitPickerFromLiveWireDelayed();
        }
      });
    }
    scheduleHardwareSyncFromEvent();
  });

  void listen<SlotContentChangedPayload>("models:slot-content-changed", (event) => {
    const p = event.payload;
    if (!p || typeof p.slotIndex !== "number") return;
    if (hwSlotDebugEnabled() || modelsSyncTraceEnabled()) {
      console.info("[HxLinux] models:slot-content-changed", p);
    }
    emitModelsSyncTraceThrottled(
      "evt_slot_content_changed",
      `event models:slot-content-changed seq=${p.sequence} slot=${p.slotIndex} kind=${p.kind}`,
      2_000,
    );
    if (
      lastHwSyncNormalizedSlots &&
      lastHwSyncNormalizedSlots.length === 16 &&
      selectedParamsKemplineSlotIndex === p.slotIndex
    ) {
      scheduleSoftRefreshParamsPaneFromSlots(lastHwSyncNormalizedSlots);
    }
  });

  void listen<SlotParamChangedPayload>("models:slot-param-changed", (event) => {
    const p = event.payload;
    if (!p || typeof p.slotIndex !== "number" || typeof p.paramIndex !== "number") return;
    if (hwSlotDebugEnabled() || modelsSyncTraceEnabled()) {
      console.info("[HxLinux] models:slot-param-changed", p);
    }
    applyHardwareSlotParamChanged(p);
  });

  void listen<SlotModelHwChangedPayload>("models:slot-model-changed", (event) => {
    const p = event.payload;
    if (!p || typeof p.slotIndex !== "number") return;
    if (hwSlotDebugEnabled() || modelsSyncTraceEnabled()) {
      console.info("[HxLinux] models:slot-model-changed", p);
    }
    applyHardwareSlotModelChanged(p);
  });

  void listen<Path1InputSourceChangedPayload>("models:path1-input-source-changed", (event) => {
    const p = event.payload;
    if (!p || typeof p.wireValue !== "number") return;
    path1InputMatrixWire = p.wireValue;
    void refreshPath1InputMatrixIcon();
    if (
      slotPickerIoLock?.kind !== "io" ||
      slotPickerIoLock.category !== "Input" ||
      !catalogPickerDataCache
    ) {
      return;
    }
    const id = findIoSourceIdByWireValue(
      catalogPickerDataCache,
      slotPickerIoLock.parentModelId,
      p.wireValue,
      connectedDeviceName,
    );
    if (!id) return;
    clearPath1InputSourceHighlightOverride();
    applySlotPickerFromCatalogSelection("Input", "Source", id);
    if (hwSlotDebugEnabled() || modelsSyncTraceEnabled()) {
      console.info("[HxLinux] models:path1-input-source-changed", p, "ioSource=", id);
    }
  });

  void listen<Path1SplitTypeChangedPayload>("models:path1-split-type-changed", (event) => {
    void (async () => {
      const p = event.payload;
      if (!p || typeof p.wireValue !== "number") return;
      let hwOnSplit = selectedSpecialHwSlotBus === HW_SLOT_BUS_SPLIT;
      if (!hwOnSplit) {
        try {
          const hw = await invoke<HardwareActiveSlotState>("get_active_hardware_slot_state");
          hwOnSplit = hw?.slotBus === HW_SLOT_BUS_SPLIT;
        } catch {
          /* ignore */
        }
      }
      const splitActive =
        hwOnSplit ||
        (slotPickerIoLock?.kind === "routing" && slotPickerIoLock.category === "Split");
      if (!splitActive) return;

      await mountModelsSlotPicker();
      if (!catalogPickerDataCache) return;

      const id = findSplitSourceIdByWireValue(
        catalogPickerDataCache,
        p.wireValue,
        connectedDeviceName,
      );
      if (!id) return;

      clearPath1SplitTypeHighlightOverride();
      if (hwOnSplit) {
        selectedSpecialHwSlotBus = HW_SLOT_BUS_SPLIT;
      }
      applySlotPickerRoutingLock("Split", id);

      const chainHex = splitChainHexFromWire(p.wireValue);
      let catalogModelId = "";
      let name = "Split";
      for (const rows of catalogPickerDataCache.modelsByCategoryAndSub.values()) {
        const row = rows.find((r) => r.id === id);
        if (row) {
          catalogModelId = (row.catalogModelId ?? "").trim();
          name = row.name;
          break;
        }
      }
      if (chainHex && catalogModelId) {
        scheduleSplitScrollParamsReload({
          category: "Split",
          name,
          moduleHex: chainHex,
          catalogModelId,
        });
      }

      if (hwSlotDebugEnabled() || modelsSyncTraceEnabled()) {
        console.info("[HxLinux] models:path1-split-type-changed", p, "splitSource=", id);
      }
    })();
  });

  void listen<string>("helix-device-lost", () => {
    purgeModelsUi();
  });

  void refresh();
  window.setInterval(() => {
    void refresh();
  }, 300);
  startOptionalUsbPresetPollTimer();
});
