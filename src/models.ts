import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  catalogPickerRowKey,
  findUsbAssignPickerLocation,
  formatSubCategoryForHeader,
  getCatalogModelIdForHex,
  getCatalogModelImageForId,
  getCatalogModelNameForId,
  getCatalogParamOrderForId,
  getUsbAssignPickerData,
  getPresetMetaForId,
  moduleHexFromCatalogForUsbVariant,
  pickBasedOn,
  pickSignal,
  usbAssignVariantFromPresetMeta,
  type CatalogPickerData,
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

const statusEl = document.getElementById("status") as HTMLElement;
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
/** `localStorage.setItem("models_hw_slot_focus_await_chain", "1")` : avant lecture chaîne, attendre `sync_hardware_slot_focus_usb` (buffer preset vide / tests IN). */
const HW_SLOT_FOCUS_AWAIT_CHAIN_KEY = "models_hw_slot_focus_await_chain";
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
/**
 * Après `probe_slot_model_usb` réussi sans re-dump preset : le parse `get_active_preset_slots` peut
 * encore refléter l’ancien `preset_data` Rust. On garde la ligne optimiste pour ce slot un court instant.
 */
let mergeProbeSlotModelUntil: {
  ki: number;
  deadline: number;
  /** Évite de spammer `emitModelsSyncTrace` sur soft-sync répétés. */
  mergeTraceEmitted?: boolean;
} | null = null;
const PROBE_SLOT_MERGE_GRACE_MS = 20_000;
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
};
const pendingLiveWrites = new Map<string, PendingLiveWrite>();
/**
 * Paramètres modifiés en live write alors que `preset_data` (RAM) n’est pas encore à jour :
 * fusion au prochain rendu / patch du panneau pour éviter l’affichage de vieilles valeurs au retour sur le slot.
 */
const liveChainParamOverridesByPresetSlot = new Map<string, Map<string, ChainParamValueJson>>();

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
}

function clearAllLiveChainParamOverrides(): void {
  liveChainParamOverridesByPresetSlot.clear();
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
let lastHwPickerCatalogId: string | null = null;
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

async function slotDebugFromHwModelPayload(p: SlotModelHwChangedPayload): Promise<{
  slot: SlotDebug;
  catalogModelIdTrimmed: string;
  hex: string;
}> {
  const hex = (p.moduleHex ?? "").trim();
  let catalogModelIdTrimmed = "";
  if (hex) {
    const cached = hwModelHexCatalogCache.get(hex);
    if (cached) {
      return { hex, catalogModelIdTrimmed: cached.catalogModelIdTrimmed, slot: cached.slot };
    }
    const id = await getCatalogModelIdForHex(hex);
    catalogModelIdTrimmed = (id ?? "").trim();
    const meta = catalogModelIdTrimmed ? await getPresetMetaForId(catalogModelIdTrimmed) : null;
    const catalogName = catalogModelIdTrimmed
      ? await getCatalogModelNameForId(catalogModelIdTrimmed)
      : null;
    const displayName = (catalogName ?? "").trim() || hex;
    const categoryName = (meta?.categoryName ?? "").trim() || "?";
    const slot: SlotDebug = {
      category: categoryName,
      name: displayName,
      moduleHex: hex,
      catalogModelId: catalogModelIdTrimmed || undefined,
    };
    hwModelHexCatalogCache.set(hex, { catalogModelIdTrimmed, slot });
    return { hex, catalogModelIdTrimmed, slot };
  }
  return {
    hex,
    catalogModelIdTrimmed,
    slot: { category: "", name: "<empty>" },
  };
}

/** Scroll modèle : matrice + titre seulement (pas de picker / pas de catalogue). */
function applyHardwareSlotModelVisualLight(ki: number, slot: SlotDebug): void {
  hwUi.runImmediate("grid", () => {
    if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
      const next = lastHwSyncNormalizedSlots.map((s, i) => (i === ki ? { ...slot } : { ...s }));
      lastHwSyncNormalizedSlots = next;
      lastHwSyncChainSignature = chainLayoutSignature(next);
    }
    patchMatrixSlotVisualFromSlot(ki, slot);
    patchMatrixCategoryDescFromSlot(ki, slot);
    const titleEl = document.getElementById("models-params-pane-title");
    if (titleEl) {
      const label = slot.name?.trim();
      titleEl.textContent = label ? `${label} …` : "Modèle…";
    }
  });
}

/** Après settle : grille + picker + noms catalogue. */
function applyHardwareSlotModelVisualFast(
  ki: number,
  slot: SlotDebug,
  catalogModelIdTrimmed: string,
): void {
  hwUi.runImmediate("grid", () => {
    if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
      const next = lastHwSyncNormalizedSlots.map((s, i) => (i === ki ? { ...slot } : { ...s }));
      lastHwSyncNormalizedSlots = next;
      lastHwSyncChainSignature = chainLayoutSignature(next);
    }
    patchMatrixSlotVisualFromSlot(ki, slot);
    patchMatrixCategoryDescFromSlot(ki, slot);
    const titleEl = document.getElementById("models-params-pane-title");
    if (titleEl) {
      const label = slot.name?.trim();
      titleEl.textContent = label ? `${label} …` : "Modèle…";
    }
  });

  if (catalogModelIdTrimmed) {
    if (catalogModelIdTrimmed === lastHwPickerCatalogId) return;
    lastHwPickerCatalogId = catalogModelIdTrimmed;
    hwUi.runImmediate("picker", () => {
      void mountModelsSlotPicker().then(async () => {
        const meta = await getPresetMetaForId(catalogModelIdTrimmed);
        syncModelsSlotPickerFromLoadedModel(
          catalogModelIdTrimmed,
          meta,
          slot.moduleHex,
          slot.category,
        );
      });
    });
  } else {
    lastHwPickerCatalogId = null;
  }
}

async function applyHardwareSlotModelParamsHeavy(
  ki: number,
  slot: SlotDebug,
  catalogModelIdTrimmed: string,
  hex: string,
): Promise<void> {
  if (catalogModelIdTrimmed) {
    await loadAndShowModelsParamsFromCatalogDefaults(slot, catalogModelIdTrimmed, ki);
  } else if (!hex) {
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
    const { slot, catalogModelIdTrimmed, hex: settledHex } =
      await slotDebugFromHwModelPayload(pending.payload);
    const tCatalog = hwModelFastDebugEnabled() ? performance.now() : 0;

    applyHardwareSlotModelVisualFast(ki, slot, catalogModelIdTrimmed);
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

/** `request_preset_content` vide tout de suite `preset_data` côté Rust : l’invoke renvoie `null` pendant toute la relecture (souvent plusieurs secondes). */
const SLOT_CHAIN_VALUES_DEFAULT_MAX_WAIT_MS = 14_000;
const SLOT_CHAIN_VALUES_DEFAULT_POLL_MS = 120;
/** Soft-sync : même plafond que le chargement manuel — un dump USB peut dépasser 5 s sans qu’il y ait anomalie. */
const SLOT_CHAIN_VALUES_SOFT_REFRESH_MAX_WAIT_MS = 14_000;

/**
 * Boucle jusqu’à ce que `get_active_preset_slot_chain_param_values` renvoie autre chose que `null`,
 * ou jusqu’à `maxWaitMs` (fenêtre couvrant un `request_preset_content` + attente slots du soft-sync).
 */
async function fetchSlotChainParamValuesReliable(
  slotIndex: number,
  abortIfLoadSeq: number | null,
  opts?: { maxWaitMs?: number; pollMs?: number },
): Promise<ChainParamValueJson[] | null> {
  const maxWaitMs = Math.max(200, opts?.maxWaitMs ?? SLOT_CHAIN_VALUES_DEFAULT_MAX_WAIT_MS);
  const pollMs = Math.max(30, opts?.pollMs ?? SLOT_CHAIN_VALUES_DEFAULT_POLL_MS);
  const deadline = Date.now() + maxWaitMs;
  let last: ChainParamValueJson[] | null = null;
  while (true) {
    if (abortIfLoadSeq !== null && abortIfLoadSeq !== modelsParamsLoadSeq) return last;
    try {
      last = await invoke<ChainParamValueJson[] | null>("get_active_preset_slot_chain_param_values", {
        slotIndex,
      });
    } catch {
      last = null;
    }
    if (last !== null) return last;
    if (Date.now() >= deadline) {
      emitModelsSyncTrace(`chainFetch TIMEOUT null slot=${slotIndex} after ${maxWaitMs}ms`);
      return last;
    }
    await delayMs(pollMs);
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
    return normalized;
  }
  if (normalized.length !== 16) return normalized;
  if (!lastHwSyncNormalizedSlots || lastHwSyncNormalizedSlots.length !== 16) return normalized;
  const ki = m.ki;
  const optimistic = lastHwSyncNormalizedSlots[ki];
  if (!optimistic) return normalized;
  if (!m.mergeTraceEmitted) {
    mergeProbeSlotModelUntil = { ...m, mergeTraceEmitted: true };
    emitModelsSyncTrace(
      `softSync merge probe slot=${ki} (stale preset_data parse vs optimistic row; no post-probe dump)`,
    );
  }
  return normalized.map((s, i) => (i === ki ? { ...optimistic } : { ...s }));
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
  const chainValues = await fetchSlotChainParamValuesReliable(idx, null, {
    maxWaitMs: SLOT_CHAIN_VALUES_SOFT_REFRESH_MAX_WAIT_MS,
    pollMs: SLOT_CHAIN_VALUES_DEFAULT_POLL_MS,
  });
  const nextSig = `${currentPresetIndex}|${idx}|${chainValuesSignature(chainValues)}`;
  if (selectedParamsValuesSig === nextSig) return;
  const slotKey = makeSlotSelectionKey(slot, idx);
  // Même slot + updater actif => patch des widgets in-place sans repasser par l'état "Chargement".
  if (
    selectedParamsInPlaceUpdater &&
    selectedParamsInPlaceSlotKey &&
    selectedParamsInPlaceSlotKey === slotKey
  ) {
    selectedParamsInPlaceUpdater(chainValues);
    selectedParamsValuesSig = nextSig;
    return;
  }
  // Ne pas mettre `selectedParamsValuesSig` avant la fin du load : sinon les polls suivants croient
  // que l'UI est à jour alors que le panneau est encore sur « Chargement » → retour immédiat, chargement bloqué, flash vide.
  await loadAndShowModelsParamsForSlot(slot, idx);
  selectedParamsValuesSig = `${currentPresetIndex}|${idx}|${chainValuesSignature(chainValues)}`;
}

function liveWriteProbeEnabled(): boolean {
  return localStorage.getItem(LIVE_WRITE_PROBE_FLAG) === "1";
}

function liveWriteEnabled(): boolean {
  return localStorage.getItem(LIVE_WRITE_ENABLED_FLAG) === "1";
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
): void {
  if (!liveWriteProbeEnabled()) return;
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
  });
}

function liveWriteParamIndexForRow(
  paramsForDisplay: ModelParamDefJson[],
  rowIndex: number,
  catalogSignal: string | null | undefined,
): number {
  const target = paramsForDisplay[rowIndex];
  if (!target) return rowIndex;
  // Le write suit la variante signal (mono/stereo) : en mono, les `stereo-only`
  // ne doivent pas compter dans l'index envoyé.
  const writeOrder = paramsVisibleForSignal(paramsForDisplay, catalogSignal);
  const idxByRef = writeOrder.indexOf(target);
  if (idxByRef >= 0) return idxByRef;
  return rowIndex;
}

/** Inverse de `liveWriteParamIndexForRow` : index wire `PP` → `symbolicID` catalogue. */
function symbolicIdForWireParamIndex(
  paramsForDisplay: ModelParamDefJson[],
  wireParamIndex: number,
  catalogSignal: string | null | undefined,
): string | null {
  const writeOrder = paramsVisibleForSignal(paramsForDisplay, catalogSignal);
  const p = writeOrder[wireParamIndex];
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
      ? symbolicIdForWireParamIndex(ctx.paramsForDisplay, p.paramIndex, ctx.catalogSignal)
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
      selectedParamsPresetIndex !== currentPresetIndex ||
      !selectedParamsInPlaceUpdater
    ) {
      return;
    }
    selectedParamsInPlaceUpdater(null);
    selectedParamsValuesSig = `${currentPresetIndex}|${slotIndex}|hw:${sequence}:${paramIndex}:${String(cv)}`;
    emitModelsSyncTraceThrottled(
      "evt_slot_param_changed",
      `hw param slot=${slotIndex} pp=${paramIndex} ${sid}=${String(cv)}`,
      400,
    );
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

function refillSlotPickerSubcategories(): void {
  if (!slotPickerCategoryEl || !slotPickerSubEl || !catalogPickerDataCache) return;
  const cat = slotPickerCategoryEl.value.trim();
  slotPickerSubEl.replaceChildren();
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
  slotPickerSubEl.value = "";
}

function refillSlotPickerModelList(highlightModelId: string | null | undefined): void {
  if (!slotPickerListEl || !catalogPickerDataCache || !slotPickerCategoryEl || !slotPickerSubEl) return;
  const cat = slotPickerCategoryEl.value.trim();
  const sub = slotPickerSubEl.value;
  slotPickerListEl.replaceChildren();
  if (!cat || !sub) return;
  const key = catalogPickerRowKey(cat, sub);
  const rows = catalogPickerDataCache.modelsByCategoryAndSub.get(key) ?? [];
  if (rows.length === 0) return;
  const hi = (highlightModelId ?? "").trim();
  for (const row of rows) {
    const li = document.createElement("li");
    li.className = "models-slot-picker-model-item";
    li.textContent = row.name;
    li.title =
      row.assignVariant !== undefined ? `${row.id} · ${row.assignVariant}` : row.id;
    li.dataset.modelId = row.id;
    if (hi && row.id === hi) li.classList.add("models-slot-picker-model-item--active");
    li.addEventListener("click", () => {
      slotPickerListEl
        ?.querySelectorAll(".models-slot-picker-model-item")
        .forEach((n) => n.classList.remove("models-slot-picker-model-item--active"));
      li.classList.add("models-slot-picker-model-item--active");
      void applySlotModelFromPickerListClick(row.id, row.name, row.assignVariant);
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
  slotPickerCategoryEl.value = "";
  refillSlotPickerSubcategories();
  slotPickerSubEl.value = "";
  refillSlotPickerModelList(null);
}

/** Variante USB pour `HX_ModelUsbAssign.json` : alignée sur la sous-catégorie du picker (Mono / Stereo / Legacy). */
function usbAssignVariantFromPickerSub(sub: string): string {
  const t = sub.trim().toLowerCase();
  if (t.includes("legacy")) return "legacy";
  if (t.includes("stereo") || t.includes("stéréo")) return "stereo";
  if (t.includes("mono")) return "mono";
  return "mono";
}

function patchMatrixSlotVisualFromSlot(ki: number, slot: SlotDebug): void {
  const nodes = contentEl.querySelectorAll<HTMLElement>(`[data-kempline-slot-index="${ki}"]`);
  for (const old of nodes) {
    const inner = gridSlotNode(slot, ki, { path: ki >= 8 ? 2 : 1 });
    old.replaceWith(inner);
  }
}

function patchMatrixCategoryDescFromSlot(ki: number, slot: SlotDebug): void {
  const el = contentEl.querySelector<HTMLElement>(`[data-kempline-slot-desc-index="${ki}"]`);
  if (!el) return;
  const empty = !slot.category && slot.name === "<empty>";
  if (empty) {
    el.textContent = "";
    el.removeAttribute("title");
    return;
  }
  const cat = slot.category.trim();
  el.textContent = cat;
  if (cat) el.title = cat;
  else el.removeAttribute("title");
}

/**
 * Clic sur une ligne du picker : MAJ immédiate pastille + paramètres (catalogue / défauts `.models`),
 * puis ordre USB `probe_slot_model_usb`.
 */
async function applySlotModelFromPickerListClick(
  catalogModelId: string,
  displayName: string,
  assignVariantFromRow?: string,
): Promise<void> {
  const ki = selectedParamsKemplineSlotIndex;
  if (ki === null || ki < 0 || ki > 15) {
    console.warn(
      "[SlotModelProbe] aucun slot grille sélectionné — cliquez d’abord un slot sur la matrice.",
    );
    return;
  }
  if (selectedParamsPresetIndex !== currentPresetIndex) return;
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
    usbAssignVariantFromPickerSub(slotPickerSubEl?.value ?? "");
  const categoryName = (slotPickerCategoryEl?.value ?? "").trim();
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
    (moduleHexFromCatalogForUsbVariant(metaEarly, assignVariant) ?? "").trim() || undefined;
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
  patchMatrixCategoryDescFromSlot(ki, optimisticSlot);
  if (selectedParamsKemplineSlotIndex === ki) {
    selectParamsPaneByKemplineIndex(ki);
  }
  if (currentPresetIndex >= 0) {
    clearLiveChainOverridesForKemplineSlot(currentPresetIndex, ki);
  }
  markLiveWriteUiInteraction();
  slotModelUsbProbeInFlight = ki;
  try {
    await loadAndShowModelsParamsFromCatalogDefaults(optimisticSlot, idTrim, ki);

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
  } catch (e) {
    console.warn("[SlotModelProbe]", e);
    if (prevSnapshot) {
      const old = prevSnapshot[ki]!;
      patchMatrixSlotVisualFromSlot(ki, old);
      patchMatrixCategoryDescFromSlot(ki, old);
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
function applySlotPickerFromCatalogSelection(
  categoryName: string,
  subKey: string,
  highlightModelId: string | null,
): void {
  if (!slotPickerCategoryEl || !slotPickerSubEl || !catalogPickerDataCache) return;
  const cats = catalogPickerDataCache.categories;
  if (cats.length === 0) return;
  let cat = categoryName.trim();
  if (!cat || !cats.includes(cat)) cat = cats[0] ?? "";
  slotPickerCategoryEl.value = cat;
  refillSlotPickerSubcategories();
  const subs = catalogPickerDataCache.subcategoriesByCategory.get(cat) ?? [];
  let sub = subKey;
  if (!subs.includes(sub)) sub = subs[0] ?? "";
  slotPickerSubEl.value = sub;
  refillSlotPickerModelList(highlightModelId);
}

function syncModelsSlotPickerFromLoadedModel(
  catalogModelId: string,
  meta: PresetMetaJson | null,
  moduleHex?: string,
  slotCategory?: string,
): void {
  if (!catalogPickerDataCache) return;
  const assignVariant = usbAssignVariantFromPresetMeta(meta, moduleHex, slotCategory);
  const loc = findUsbAssignPickerLocation(catalogPickerDataCache, catalogModelId, assignVariant);
  if (!loc) {
    const catFallback = catalogPickerDataCache.categories[0] ?? "";
    const subFallback = catalogPickerDataCache.subcategoriesByCategory.get(catFallback)?.[0] ?? "";
    applySlotPickerFromCatalogSelection(catFallback, subFallback, null);
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
      refillSlotPickerSubcategories();
      refillSlotPickerModelList(null);
    });
    subSel.addEventListener("change", () => {
      refillSlotPickerModelList(null);
    });

    resetSlotPickerToIdle();
  })();
  return slotPickerMountPromise;
}

function getModelsParamsPaneTitleEl(): HTMLElement | null {
  return document.getElementById("models-params-pane-title");
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

/** En-tête du panneau : catégorie + chainHex lu (ou `—`) ; `slotTypeHex` en info debug I/O. */
function setModelsParamsPaneCategory(
  category: string,
  moduleHex?: string,
  slotTypeHex?: string,
) {
  const el = getModelsParamsPaneTitleEl();
  if (!el) return;
  const cat = category.trim();
  const hex = (moduleHex ?? "").trim();
  if (!cat && !hex) {
    el.textContent = "";
    return;
  }
  el.replaceChildren();
  const catSpan = document.createElement("span");
  catSpan.className = "models-params-pane-title-main";
  catSpan.textContent = cat;
  el.appendChild(catSpan);
  const hexSpan = document.createElement("span");
  hexSpan.className = "models-params-pane-title-module-hex";
  if (hex) {
    const hexLower = hex.toLowerCase();
    hexSpan.textContent = `chainHex: ${hexLower}`;
    hexSpan.title = `chainHex lu depuis le HX : ${hexLower}`;
  } else {
    const st = (slotTypeHex ?? "").trim().toUpperCase();
    hexSpan.textContent = st ? `chainHex: — (slotType ${st})` : "chainHex: —";
    hexSpan.title = st
      ? `chainHex non détecté pour ce slot (slotType Kempline: ${st})`
      : "chainHex non détecté pour ce slot";
  }
  el.appendChild(hexSpan);
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
    void (async () => {
      const emptySlot: SlotDebug = { category: "", name: "<empty>" };
      slotModelUsbProbeInFlight = slotIndex;
      try {
        const out = await invoke<string>("probe_slot_model_usb", {
          op: "remove",
          slotIndex,
        });
        console.info("[SlotModelProbe]", "remove", `slot=${slotIndex}`, out);
        selectedParamsValuesSig = null;
        // Comme `add`/`replace` : `probe_slot_model_usb` n’écrit pas dans `preset_data` Rust.
        // Sans MAJ optimiste du snapshot, le soft-sync (~200 ms) re-parse l’ancien dump → fantôme.
        if (lastHwSyncNormalizedSlots && lastHwSyncNormalizedSlots.length === 16) {
          const next = lastHwSyncNormalizedSlots.map((s, i) =>
            i === slotIndex ? { ...emptySlot } : { ...s },
          );
          lastHwSyncNormalizedSlots = next;
          lastHwSyncChainSignature = chainLayoutSignature(next);
          patchMatrixSlotVisualFromSlot(slotIndex, emptySlot);
          patchMatrixCategoryDescFromSlot(slotIndex, emptySlot);
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
        if (selectedParamsKemplineSlotIndex === slotIndex) {
          suppressNextUiSlotHardwareSwitch = true;
          selectParamsPaneByKemplineIndex(slotIndex);
        }
      } catch (e) {
        console.warn("[SlotModelProbe][remove]", e);
      } finally {
        slotModelUsbProbeInFlight = null;
      }
    })();
  });
  el.appendChild(btn);
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
    `[data-kempline-slot-index="${kemplineSlotIndex}"]`,
  );
  if (!node) return false;
  suppressNextUiSlotHardwareSwitch = true;
  node.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  return true;
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
  setModelsParamsPaneCategory("");
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
  setModelsParamsPaneCategory("");
  clearModelsParamsSubheadAndIcon();
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  resetSlotPickerToIdle();
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
    selectedParamsPresetIndex = currentPresetIndex;
    selectedParamsValuesSig = null;
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
      if (shouldSwitchHardware && Number.isFinite(kemplineSlotIndex)) {
        await waitUntilHardwareSyncIdle(15_000);
        await enqueueHardwareSlotSwitch(kemplineSlotIndex as number);
      }
      if (slot === null) {
        suppressNextUiSlotHardwareSwitch = false;
        clearModelsParamsPaneContent();
        return;
      }
      if (!userInitiated && hwUi.blockSyntheticParamsLoad) {
        return;
      }
      await loadAndShowModelsParamsForSlot(
        slot,
        Number.isFinite(kemplineSlotIndex) ? kemplineSlotIndex : undefined,
      );
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
}

function padNum(n: number): string {
  return String(n).padStart(3, "0");
}

function isEmpty(name: string): boolean {
  return !name || name === "<empty>";
}

function setStatus(text: string) {
  statusEl.textContent = text;
}

function purgeModelsUi() {
  connectedDeviceName = null;
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
  "amp+cab": "FX_HX_Category_Amp+Cab.png",
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
  return category.trim().toLowerCase();
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

/** Bases de fichiers `.models` à essayer dans l’ordre (sans extension). */
function modelsDefinitionFileBasesForCategory(category: string): string[] {
  const k = normalizeCategory(category);
  const m: Record<string, string[]> = {
    amp: ["amp"],
    preamp: ["preamp"],
    "amp+cab": ["amp", "cab", "preamp"],
    cab: [...CAB_MODEL_DEFINITION_BASES],
    ir: ["cabmicirs", "cabmicirswithpan"],
    "impulse response": ["cabmicirs", "cabmicirswithpan"],
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
  const bases = modelsDefinitionFileBasesForCategory(categoryForFiles);
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
type LinkedCabInfoJson = [string, string, string, string];
type LinkedCabWithParamsJson = {
  cab: LinkedCabInfoJson;
  values: ChainParamValueJson[];
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

function filterParamsByCatalogOrder(
  params: ModelParamDefJson[],
  catalogParamOrder: string[] | null | undefined,
): ModelParamDefJson[] {
  void catalogParamOrder;
  // Source de vérité UI: le fichier `.models`.
  return params;
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
  catalogParamOrder?: string[] | null,
): Array<ChainParamValueJson | undefined> | null | undefined {
  if (chainValues == null) return chainValues;
  void catalogParamOrder;
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
          if (liveWriteProbeEnabled() && liveWriteEnabled()) {
            markLiveWriteUiInteraction();
          }
          const writeParamIndex = liveWriteParamIndexForRow(
            params,
            j,
            catalogSignal,
          );
          scheduleLiveParamWriteProbe(liveWriteSlotIndex, writeParamIndex, p, clamped);
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
        if (liveWriteProbeEnabled() && liveWriteEnabled()) {
          markLiveWriteUiInteraction();
        }
        const writeParamIndex = liveWriteParamIndexForRow(
          params,
          j,
          catalogSignal,
        );
        scheduleLiveParamWriteProbe(liveWriteSlotIndex, writeParamIndex, p, v);
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
        if (liveWriteProbeEnabled() && liveWriteEnabled()) {
          markLiveWriteUiInteraction();
        }
        const writeParamIndex = liveWriteParamIndexForRow(
          params,
          j,
          catalogSignal,
        );
        scheduleLiveParamWriteProbe(liveWriteSlotIndex, writeParamIndex, p, nextB ? 1 : 0);
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

function pickCabIndexForLinkedCab(cabs: ModelDefinitionJson[], linkedCab: LinkedCabInfoJson | null): number {
  if (!linkedCab || cabs.length === 0) return -1;
  const linkedModelId = linkedCab[3]?.trim() ?? "";
  if (!linkedModelId) return -1;
  return cabs.findIndex((c) => (c.symbolicID ?? "").trim() === linkedModelId);
}

function renderCabSection(
  inner: HTMLElement,
  cabs: ModelDefinitionJson[],
  linkedCab: LinkedCabInfoJson | null,
  linkedCabValues: ChainParamValueJson[] | null,
  helixControlsMap?: Map<string, HelixControlDefJson>,
): void {
  if (cabs.length === 0) return;
  const title = document.createElement("div");
  title.className = "models-params-cab-section-title";
  title.append("Cab / IR (association locale)");
  const cabChainHex = (linkedCab?.[0] ?? "").trim();
  if (cabChainHex) {
    title.append(" — chainHex cab ");
    const hexSpan = document.createElement("span");
    hexSpan.className = "models-params-cab-section-chain-hex";
    hexSpan.textContent = cabChainHex.toUpperCase();
    title.appendChild(hexSpan);
    title.title =
      "Hex du module cab lu dans le preset (à comparer à presetMeta.chainHex dans HX_ModelCatalog.json pour ce modèle).";
  }
  inner.appendChild(title);

  const linkedIdx = pickCabIndexForLinkedCab(cabs, linkedCab);
  let cabIdx = linkedIdx >= 0 ? linkedIdx : 0;
  const renderList = () => {
    const prev = inner.querySelector(".models-params-cab-list");
    if (prev) prev.remove();
    const list = document.createElement("ul");
    list.className = "models-params-list models-params-cab-list";

    const selectRow = document.createElement("li");
    selectRow.className = "models-params-row";
    const label = document.createElement("span");
    label.className = "models-params-row-name";
    label.textContent = "Cab / IR Model";
    const minEl = document.createElement("span");
    minEl.className = "models-params-row-min";
    minEl.textContent = "—";
    const maxEl = document.createElement("span");
    maxEl.className = "models-params-row-max";
    maxEl.textContent = "—";
    const sliderCell = document.createElement("div");
    sliderCell.className = "models-params-slider-cell";
    const cabItems: ModelsComboItem[] = [];
    for (const def of cabs) {
      const sid = (def.symbolicID ?? "").trim();
      if (!sid) continue;
      cabItems.push({ value: sid, label: `${(def.name ?? sid).trim()} (${sid})` });
    }
    const currentSid = (cabs[cabIdx]?.symbolicID ?? "").trim();
    mountModelsCombo(
      sliderCell,
      cabItems,
      currentSid || cabItems[0]?.value || "",
      (sidRaw) => {
        const sid = sidRaw.trim();
        const idx = cabs.findIndex((c) => (c.symbolicID ?? "").trim() === sid);
        if (idx < 0) return;
        cabIdx = idx;
        renderList();
      },
      "Cab ou IR (menu déroulant) — aperçu local, non envoyé au Helix",
    );
    selectRow.append(label, minEl, sliderCell, maxEl);
    list.appendChild(selectRow);

    if (linkedCab && linkedIdx < 0) {
      const info = document.createElement("li");
      info.className = "models-params-row";
      const t = document.createElement("span");
      t.className = "models-params-row-name";
      t.textContent = "Cab lié (ID)";
      const l = document.createElement("span");
      l.className = "models-params-row-min";
      l.textContent = "—";
      const c = document.createElement("span");
      c.className = "models-params-row-chain";
      c.textContent = linkedCab[3]?.trim()
        ? `ID introuvable dans cab / cabmicirs / cabmicirswithpan: ${linkedCab[3].trim()}`
        : "ID cab manquant (chainHex non renseigné)";
      const r = document.createElement("span");
      r.className = "models-params-row-max";
      r.textContent = "—";
      const cell = document.createElement("div");
      cell.className = "models-params-slider-cell";
      cell.append(c);
      info.append(t, l, cell, r);
      list.appendChild(info);
    }

    const cabDef = cabs[cabIdx];
    const cabParams = cabDef?.params ?? [];
    const cabDefaults = cabParams.map(chainValueFromParamDefault);
    const cabValues =
      linkedIdx >= 0 && cabIdx === linkedIdx && (linkedCabValues?.length ?? 0) > 0
        ? linkedCabValues
        : cabDefaults;
    appendModelsParamRows(
      list,
      cabParams,
      cabValues,
      helixControlsMap,
      null,
      " (cab)",
    );
    inner.appendChild(list);
  };
  renderList();
}

function showModelsParamsLoading() {
  clearModelsParamsSubheadAndIcon();
  selectedParamsInPlaceUpdater = null;
  selectedParamsInPlaceSlotKey = null;
  selectedParamsHwWireContext = null;
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  resetSlotPickerToIdle();
  const p = document.createElement("p");
  p.className = "models-params-placeholder";
  p.textContent = "Chargement des paramètres…";
  inner.appendChild(p);
}

function renderModelsParamsPane(
  slot: SlotDebug,
  params: ModelParamDefJson[],
  catalogParamOrder: string[] | null,
  resolvedCatalogModelName?: string,
  chainValues?: ChainParamValueJson[] | null,
  linkedCab?: LinkedCabInfoJson | null,
  linkedCabValues?: ChainParamValueJson[] | null,
  allCabDefinitions?: ModelDefinitionJson[] | null,
  catalogBasedOn?: string | null,
  catalogSubcategoryLabel?: string | null,
  catalogRoutingSignal?: string | null,
  helixControlsMap?: Map<string, HelixControlDefJson>,
  catalogModelImage?: string | null,
  kemplineSlotIndex?: number,
) {
  setModelsParamsPaneCategory(slot.category, slot.moduleHex, slot.slotTypeHex);
  const inner = getModelsParamsInner();
  if (!inner) return;
  const head = document.createElement("div");
  head.className = "models-params-model-head";
  const baseName = (resolvedCatalogModelName ?? slot.name).trim() || "—";
  const bo = (catalogBasedOn ?? "").trim();
  const sub = (catalogSubcategoryLabel ?? "").trim();
  const titleRow = document.createElement("div");
  titleRow.className = "models-params-model-title-row";
  const leftTitle = document.createElement("div");
  leftTitle.className = "models-params-model-name-sub";
  const title = document.createElement("div");
  title.className = "models-params-model-title";
  title.textContent = baseName;
  leftTitle.appendChild(title);
  if (sub) {
    const subInline = document.createElement("div");
    subInline.className = "models-params-model-sub-inline";
    subInline.textContent = sub;
    subInline.title = sub;
    leftTitle.appendChild(subInline);
  }
  titleRow.appendChild(leftTitle);
  const lines: HTMLElement[] = [titleRow];
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
  if (linkedCab && linkedCab[0] && linkedCab[2]) {
    const cab = document.createElement("div");
    cab.className = "models-params-model-sub";
    const cabId = linkedCab[3]?.trim();
    cab.textContent = cabId
      ? `Cab: ${linkedCab[2]} · ${linkedCab[0].toUpperCase()} · ${cabId}`
      : `Cab: ${linkedCab[2]} · ${linkedCab[0].toUpperCase()}`;
    lines.push(cab);
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

  const list = document.createElement("ul");
  list.className = "models-params-list";
  const paramsForDisplay = filterParamsByCatalogOrder(params, catalogParamOrder);
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
      catalogParamOrder,
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
  );
  const applyRawChainValuesInPlace = (rawChainValues: ChainParamValueJson[] | null): void => {
    const aligned = mergeLiveChainOverridesIntoAligned(
      currentPresetIndex,
      kemplineSlotIndex,
      paramsForDisplay,
      alignChainValuesToModelParamOrder(
        rawChainValues,
        paramsForDisplay,
        params,
        catalogRoutingSignal,
        catalogParamOrder,
      ),
    );
    updateAlignedValues(aligned ?? null);
  };
  if (subhead) {
    inner.replaceChildren(list);
  } else {
    inner.replaceChildren(head, list);
  }
  if (normalizeCategory(slot.category) === "amp+cab" && (allCabDefinitions?.length ?? 0) > 0) {
    renderCabSection(
      inner,
      allCabDefinitions!,
      linkedCab ?? null,
      linkedCabValues ?? null,
      helixControlsMap,
    );
  }
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
    selectedParamsHwWireContext = {
      paramsForDisplay,
      catalogSignal: catalogRoutingSignal,
    };
  }
}

function showModelsParamsNotFound(slot: SlotDebug, resolvedCatalogId?: string | null) {
  setModelsParamsPaneCategory(slot.category, slot.moduleHex, slot.slotTypeHex);
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
  setModelsParamsPaneCategory("");
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

function showModelsParamsRawFallback(
  slot: SlotDebug,
  chainValues: ChainParamValueJson[] | null | undefined,
) {
  setModelsParamsPaneCategory(slot.category || "Unknown", slot.moduleHex, slot.slotTypeHex);
  clearModelsParamsSubheadAndIcon();
  resetSlotPickerToIdle();
  setModelsParamsHeaderIcon(slot, null);
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();

  const note = document.createElement("p");
  note.className = "models-params-placeholder";
  const hex = (slot.moduleHex ?? "").trim();
  note.textContent = hex
    ? `Jointure catalogue absente pour ${hex.toUpperCase()} — affichage brut des valeurs de chaîne.`
    : "Jointure catalogue absente — affichage brut des valeurs de chaîne.";
  inner.appendChild(note);

  const list = document.createElement("ul");
  list.className = "models-params-list";
  const vals = chainValues ?? [];
  for (let i = 0; i < vals.length; i += 1) {
    const v = vals[i];
    if (v === undefined) continue;
    const li = document.createElement("li");
    li.className = "models-params-row";

    const name = document.createElement("span");
    name.className = "models-params-row-name";
    name.textContent = `Param ${i + 1}`;

    const min = document.createElement("span");
    min.className = "models-params-row-min";
    min.textContent = "—";

    const max = document.createElement("span");
    max.className = "models-params-row-max";
    max.textContent = "—";

    const cell = document.createElement("div");
    cell.className = "models-params-slider-cell";
    const chain = document.createElement("span");
    chain.className = "models-params-row-chain";
    chain.textContent = formatChainParamValueJson(v);
    const raw = formatRawChainParamValueJson(v);
    li.title = raw;
    cell.title = raw;
    cell.appendChild(chain);
    li.append(name, min, cell, max);
    list.appendChild(li);
  }
  inner.appendChild(list);
}

async function loadAndShowModelsParamsForSlot(
  slot: SlotDebug,
  kemplineSlotIndex?: number,
) {
  const seq = ++modelsParamsLoadSeq;
  setModelsParamsPaneCategory(slot.category, slot.moduleHex, slot.slotTypeHex);
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
    innerBefore.querySelector("ul.models-params-list") !== null;
  if (!preserveParamsChrome) {
    showModelsParamsLoading();
  }
  const nk = normalizeCategory(slot.category);
  if (nk === "routing" || nk === "none" || nk === "favorites") {
    if (seq === modelsParamsLoadSeq) showModelsParamsNotFound(slot, null);
    return;
  }
  try {
    let chainValues: ChainParamValueJson[] | null = null;
    let linkedCab: LinkedCabInfoJson | null = null;
    let linkedCabValues: ChainParamValueJson[] | null = null;
    let allCabDefinitions: ModelDefinitionJson[] | null = null;
    if (kemplineSlotIndex !== undefined && Number.isInteger(kemplineSlotIndex)) {
      if (
        typeof localStorage !== "undefined" &&
        localStorage.getItem(HW_SLOT_FOCUS_AWAIT_CHAIN_KEY) === "1" &&
        slotFocusUsbSyncEnabled()
      ) {
        try {
          await invoke("sync_hardware_slot_focus_usb", { slotIndex: kemplineSlotIndex });
        } catch {
          /* focus USB optionnel */
        }
      }
      chainValues = await fetchSlotChainParamValuesReliable(kemplineSlotIndex, seq);
      try {
        const linkedCabFull = await invoke<LinkedCabWithParamsJson | null>(
          "get_active_preset_slot_linked_cab_with_params",
          { slotIndex: kemplineSlotIndex },
        );
        linkedCab = linkedCabFull?.cab ?? null;
        linkedCabValues = linkedCabFull?.values ?? null;
      } catch {
        linkedCabValues = null;
        try {
          linkedCab = await invoke<LinkedCabInfoJson | null>(
            "get_active_preset_slot_linked_cab",
            { slotIndex: kemplineSlotIndex },
          );
        } catch {
          linkedCab = null;
        }
      }
    }
    if ((nk === "input" || nk === "output") && (chainValues?.length ?? 0) === 0) {
      try {
        chainValues = await invoke<ChainParamValueJson[] | null>(
          "get_active_preset_path1_io_chain_param_values",
          { ioKind: nk === "input" ? "input" : "output" },
        );
      } catch {
        chainValues = chainValues ?? null;
      }
    }
    // Split / Merge : pas de `data-kempline-slot-index` sur les jonctions matrice — lire le segment flow 0x02 / 0x03.
    if ((nk === "split" || nk === "merge") && (chainValues?.length ?? 0) === 0) {
      try {
        chainValues = await invoke<ChainParamValueJson[] | null>(
          "get_active_preset_kempline_flow_chain_param_values",
          { flowKind: nk },
        );
      } catch {
        chainValues = chainValues ?? null;
      }
    }
    let catalogModelId =
      (slot.catalogModelId ?? "").trim() || (await getCatalogModelIdForHex(slot.moduleHex));
    let catalogModelIdTrimmed = (catalogModelId ?? "").trim();
    const mergeLike =
      nk === "merge" ||
      (nk === "routing" && slot.name.toLowerCase().includes("merge"));
    if (!catalogModelIdTrimmed && mergeLike) {
      catalogModelIdTrimmed = FLOW_JOIN_CATALOG_ID;
    }
    if (!catalogModelIdTrimmed) {
      if (seq !== modelsParamsLoadSeq) return;
      const hex = (slot.moduleHex ?? "").trim();
      if ((chainValues?.length ?? 0) > 0) {
        showModelsParamsRawFallback(slot, chainValues);
        return;
      }
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
    const [catalogImage, catalogParamOrder] = await Promise.all([
      getCatalogModelImageForId(catalogModelIdTrimmed),
      getCatalogParamOrderForId(catalogModelIdTrimmed),
    ]);
    if (seq !== modelsParamsLoadSeq) return;
    const catalogBasedOn = pickBasedOn(meta);
    const catalogSubcategoryLabel = formatSubCategoryForHeader(meta, slot.moduleHex);
    const catalogRoutingSignal = pickSignal(meta, slot.moduleHex);
    if (nk === "amp+cab") {
      try {
        const byId = new Map<string, ModelDefinitionJson>();
        for (const base of CAB_MODEL_DEFINITION_BASES) {
          const defs = await loadModelsDefinitionArray(base);
          for (const d of defs) {
            const sid = (d.symbolicID ?? "").trim();
            if (!sid || byId.has(sid)) continue;
            byId.set(sid, d);
          }
        }
        allCabDefinitions = [...byId.values()];
      } catch {
        allCabDefinitions = null;
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
    }
    const kempMatchesNow =
      kemplineSlotIndex === undefined ||
      (selectedParamsKemplineSlotIndex !== null && selectedParamsKemplineSlotIndex === kemplineSlotIndex);
    const canPatchValuesOnly =
      !modelChangedAtSlot &&
      kempMatchesNow &&
      selectedParamsPresetIndex === currentPresetIndex &&
      selectedParamsSlotKey === slotKeyNow &&
      selectedParamsInPlaceUpdater !== null &&
      selectedParamsInPlaceSlotKey === slotKeyNow;

    if (canPatchValuesOnly) {
      const patchFn = selectedParamsInPlaceUpdater;
      if (patchFn) patchFn(chainValues);
      paramsPaneCatalogBySlotKey.set(slotKeyNow, catalogModelIdTrimmed);
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
      catalogParamOrder,
      short,
      chainValues,
      linkedCab,
      linkedCabValues,
      allCabDefinitions,
      catalogBasedOn,
      catalogSubcategoryLabel,
      catalogRoutingSignal,
      helixControlsMap,
      catalogImage,
      kemplineSlotIndex,
    );
    void mountModelsSlotPicker().then(() => {
      syncModelsSlotPickerFromLoadedModel(
        catalogModelIdTrimmed,
        meta,
        slot.moduleHex,
        slot.category,
      );
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
): Promise<void> {
  const seq = ++modelsParamsLoadSeq;
  setModelsParamsPaneCategory(slot.category, slot.moduleHex, slot.slotTypeHex);
  const nk = normalizeCategory(slot.category);
  if (nk === "routing" || nk === "none" || nk === "favorites") {
    if (seq === modelsParamsLoadSeq) showModelsParamsNotFound(slot, null);
    return;
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
    const [catalogImage, catalogParamOrder] = await Promise.all([
      getCatalogModelImageForId(catalogModelIdTrimmed),
      getCatalogParamOrderForId(catalogModelIdTrimmed),
    ]);
    if (seq !== modelsParamsLoadSeq) return;
    const catalogBasedOn = pickBasedOn(meta);
    const catalogSubcategoryLabel = formatSubCategoryForHeader(meta, slot.moduleHex);
    const catalogRoutingSignal = pickSignal(meta, slot.moduleHex);
    let allCabDefinitions: ModelDefinitionJson[] | null = null;
    if (nk === "amp+cab") {
      try {
        const byId = new Map<string, ModelDefinitionJson>();
        for (const base of CAB_MODEL_DEFINITION_BASES) {
          const defs = await loadModelsDefinitionArray(base);
          for (const d of defs) {
            const sid = (d.symbolicID ?? "").trim();
            if (!sid || byId.has(sid)) continue;
            byId.set(sid, d);
          }
        }
        allCabDefinitions = [...byId.values()];
      } catch {
        allCabDefinitions = null;
      }
    }
    const helixControlsMap = await getHelixControlsMap();
    if (seq !== modelsParamsLoadSeq) return;
    const defaultChain = buildDefaultChainValuesForSourceOrder(
      found.entry.params ?? [],
      catalogRoutingSignal,
    );
    renderModelsParamsPane(
      slot,
      found.entry.params ?? [],
      catalogParamOrder,
      short,
      defaultChain,
      null,
      null,
      allCabDefinitions,
      catalogBasedOn,
      catalogSubcategoryLabel,
      catalogRoutingSignal,
      helixControlsMap,
      catalogImage,
      kemplineSlotIndex,
    );
    void mountModelsSlotPicker().then(() => {
      syncModelsSlotPickerFromLoadedModel(
        catalogModelIdTrimmed,
        meta,
        slot.moduleHex,
        slot.category,
      );
    });
    const slotKeyNow = makeSlotSelectionKey(slot, kemplineSlotIndex);
    paramsPaneCatalogBySlotKey.set(slotKeyNow, catalogModelIdTrimmed);
    if (
      selectedParamsPresetIndex === currentPresetIndex &&
      selectedParamsKemplineSlotIndex !== null &&
      selectedParamsKemplineSlotIndex === kemplineSlotIndex
    ) {
      selectedParamsValuesSig = `${currentPresetIndex}|${kemplineSlotIndex}|${chainValuesSignature(defaultChain)}`;
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

/** Fichier `image` du catalogue : uniquement un nom de fichier PNG sûr pour `icons_models/`. */
function sanitizeIconsModelsFilename(name: string): string | null {
  const t = name.trim();
  if (!t || t.includes("/") || t.includes("\\") || t.includes("..")) return null;
  if (!/^[a-zA-Z0-9_.-]+\.png$/i.test(t)) return null;
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
    img.width = 56;
    img.height = 56;
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

/** Slot vide matrice : icône « empty » (path1 partout ; path2 seulement entre split/merge via `gridSlotNode`). */
function makeEmptySlotNode(withIcon: boolean): HTMLElement {
  const item = document.createElement("div");
  item.className =
    "node node-empty node--hx-slot node-empty-flat" +
    (withIcon ? " node-empty-slot-icon node--icon-only" : "");
  item.title = "Slot vide";
  item.setAttribute("aria-label", "Slot vide");
  if (withIcon) {
    const iconWrap = document.createElement("div");
    iconWrap.className = "node-icon-wrap";
    const img = document.createElement("img");
    img.className = "node-icon-img";
    img.src = MATRIX_EMPTY_SLOT_ICON;
    img.alt = "";
    img.width = 56;
    img.height = 56;
    img.decoding = "async";
    iconWrap.appendChild(img);
    item.appendChild(iconWrap);
  }
  bindSlotParamsInteraction(item, null);
  return item;
}

/** Path2 slot row index 0..7 : entre split et merge (même logique que le trait L3 entre jonctions). */
function path2SlotBetweenSplitAndMerge(
  slotRowIndex: number,
  splitCol: number,
  mergeCol: number,
): boolean {
  const b = slotRowIndex + 1;
  // `merge_after_col` est la frontière où se dessine le merge : le dernier slot path2 *avant*
  // cette frontière a `b === mergeCol` (ex. merge en 7 → slot vide en 6 avec b=7). Un `<` strict
  // masquait l’icône `Icons_empty_slot.png` sur ce slot.
  return b > splitCol && b <= mergeCol;
}

const IO_INPUT_ICON = "/src-tauri/resources/icons_category/icon-input-category.png";
const IO_OUTPUT_ICON = "/src-tauri/resources/icons_category/icon-output-category.png";
const MATRIX_PATH1_LINE_ICON = "/src-tauri/resources/icons_category/Icons_line.png";
const MATRIX_PATH1_SPLIT_MERGE_ICON =
  "/src-tauri/resources/icons_category/Icons_split_merge.png";
const MATRIX_ICON_VERTICAL = "/src-tauri/resources/icons_category/Icons_vertical_line.png";
const MATRIX_ICON_LINK_SPLIT = "/src-tauri/resources/icons_category/Icons_link_split.png";
const MATRIX_ICON_LINK_MERGE = "/src-tauri/resources/icons_category/Icons_link_merge.png";
const MATRIX_EMPTY_SLOT_ICON =
  "/src-tauri/resources/icons_category/Icons_empty_slot.png";

/** Nœuds d'extrémité façon HX Edit (icônes Input / Main L·R). */
function makeIoNode(kind: "input" | "output"): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-io hx-io--icon";
  const img = document.createElement("img");
  img.className = "hx-io-icon";
  img.decoding = "async";
  if (kind === "input") {
    img.src = IO_INPUT_ICON;
    img.alt = "Input";
    el.setAttribute("aria-label", "Input");
    el.title = "Input";
  } else {
    img.src = IO_OUTPUT_ICON;
    img.alt = "Main L/R";
    el.setAttribute("aria-label", "Main L/R");
    el.title = "Main L/R";
  }
  el.appendChild(img);
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
  const markerCategory = (marker?.category ?? "").trim();
  const markerName = (marker?.name ?? "").trim();
  if (kind === "split") {
    return {
      category: markerCategory || "Split",
      name: markerName || `Split (Path 1 #${boundary})`,
      moduleHex: markerHex,
    };
  }
  if (kind === "merge") {
    return {
      category: markerCategory || "Merge",
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

/** Trait vertical (overlay optionnel sur « Path 2 » L3) — désactivé par défaut (chevauchement avec `Icons_link_*`). */
function makeMatrixVerticalSpanIcon(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "hx-matrix-vspan";
  wrap.setAttribute("aria-hidden", "true");
  const img = document.createElement("img");
  img.className = "hx-matrix-vspan-img";
  img.src = MATRIX_ICON_VERTICAL;
  img.alt = "";
  img.decoding = "async";
  wrap.appendChild(img);
  return wrap;
}

/** Même PNG que le vspan, dans la cellule « Description Path 1 » (L2), colonnes split/merge. */
function makeMatrixDescRowVerticalIcon(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "hx-matrix-r2-vline";
  wrap.setAttribute("aria-hidden", "true");
  const img = document.createElement("img");
  img.className = "hx-matrix-vspan-img";
  img.src = MATRIX_ICON_VERTICAL;
  img.alt = "";
  img.decoding = "async";
  wrap.appendChild(img);
  return wrap;
}

/** Jonction sur « Path 2 » (L3) : coin split ou merge (`Icons_link_*`). */
function makeMatrixPath2LinkIcon(src: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "hx-matrix-path2-link";
  wrap.setAttribute("aria-hidden", "true");
  const img = document.createElement("img");
  img.className = "hx-matrix-junction-line-img";
  img.src = src;
  img.alt = "";
  img.decoding = "async";
  wrap.appendChild(img);
  return wrap;
}

/** Colonnes paires « Path 1 » (L1) sans split/merge : icône ligne horizontale. */
function makeMatrixPath1LineIcon(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "hx-matrix-junction-line hx-matrix-path1-separator";
  wrap.setAttribute("aria-hidden", "true");
  const img = document.createElement("img");
  img.className = "hx-matrix-junction-line-img";
  img.src = MATRIX_PATH1_LINE_ICON;
  img.alt = "";
  img.decoding = "async";
  wrap.appendChild(img);
  return wrap;
}

/** Ligne horizontale « Path 2 » (L3), même asset `Icons_line.png` que Path 1. */
function makeMatrixPath2LineIcon(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "hx-matrix-junction-line";
  wrap.setAttribute("aria-hidden", "true");
  const img = document.createElement("img");
  img.className = "hx-matrix-junction-line-img";
  img.src = MATRIX_PATH1_LINE_ICON;
  img.alt = "";
  img.decoding = "async";
  wrap.appendChild(img);
  return wrap;
}

/** Split / merge sur « Path 1 » (L1) : icône jonction ; `title` sur la cellule grille. */
function makePathRoutingNode(kind: "split" | "merge"): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = `hx-matrix-routing-marker hx-matrix-path1-separator hx-matrix-routing-marker--${kind}`;
  wrap.dataset.routingMarker = kind;
  const img = document.createElement("img");
  img.className = "hx-matrix-routing-marker-img";
  img.src = MATRIX_PATH1_SPLIT_MERGE_ICON;
  img.alt = "";
  img.decoding = "async";
  wrap.appendChild(img);
  return wrap;
}

function gridSlotNode(
  slot: SlotDebug,
  kemplineSlotIndex: number,
  opts?: {
    path?: 1 | 2;
    /** Index 0..7 sur la rangée path2 (ignoré si path !== 2). */
    path2SlotRowIndex?: number;
    splitCol?: number;
    mergeCol?: number;
  },
): HTMLElement {
  const empty = !slot.category && slot.name === "<empty>";
  if (empty) {
    const onPath2 = opts?.path === 2;
    const i = opts?.path2SlotRowIndex ?? 0;
    const sc = opts?.splitCol ?? 0;
    const mc = opts?.mergeCol ?? 8;
    const showIcon =
      !onPath2 || (onPath2 && path2SlotBetweenSplitAndMerge(i, sc, mc));
    const n = makeEmptySlotNode(showIcon);
    n.dataset.kemplineSlotIndex = String(kemplineSlotIndex);
    return n;
  }
  /* Matrice : sur « Path 1 » / « Path 2 », la catégorie est sur la ligne Description ; la cellule slot = icône + infobulle nom. */
  const node = makeNode(slot, { showTypeAbbrev: false });
  node.dataset.kemplineSlotIndex = String(kemplineSlotIndex);
  return node;
}

/** Libellé catégorie : « Description Path 1 » (L2) ou « Description Path 2 » (L4) sous un slot. */
function makeMatrixCategoryCell(slot: SlotDebug, kemplineDescIndex: number): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-matrix-category";
  el.dataset.kemplineSlotDescIndex = String(kemplineDescIndex);
  const empty = !slot.category && slot.name === "<empty>";
  if (empty) {
    el.textContent = "";
    el.removeAttribute("title");
    return el;
  }
  const cat = slot.category.trim();
  el.textContent = cat;
  if (cat) el.title = cat;
  return el;
}

/** Texte fixe sur « Description Path 1 » (L2) : sous Input / Main L·R. */
function makeMatrixDescriptionLabel(text: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-matrix-category";
  el.textContent = text;
  el.title = text;
  return el;
}

/** Colonne debug : numéro de ligne grille (1–4). */
function makeMatrixRowLineLabel(line: number): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-matrix-row-line-debug";
  el.textContent = String(line);
  el.title = `Ligne ${line}`;
  return el;
}

/**
 * Grille 16 cases Kempline : matrice **4 lignes × 20 colonnes** (sans rangée séparateur — essai).
 * - **Ligne 1** = Path 1
 * - **Ligne 2** = Description Path 1
 * - **Ligne 3** = Path 2
 * - **Ligne 4** = Description Path 2
 * Col 20 = numéro de ligne grille (debug). Pour revenir à 5 lignes + séparateur : blocs `REVERT` (TS + CSS).
 */
function renderGrid16(
  slots: SlotDebug[],
  routing: RoutingMarker[],
  stompLayout: ActivePresetStompLayout | null,
) {
  const lastB = lastFilledSlotRowIndex(slots, 8, 8);
  const hasBranchB = lastB >= 0;
  /** Split/merge path1, traits verticaux L2, path2 : seulement si au moins un bloc path2 (ignore Split/Merge parsés sans branche B). */
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

  /**
   * Nomenclature `grid-row` (1-based) :
   * L1 = Path 1, L2 = Description Path 1, L3 = Path 2, L4 = Description Path 2 (sans rangée séparateur).
   */
  const LINE_PATH_1 = 1;
  const LINE_DESC_PATH_1 = 2;
  const LINE_PATH_2 = 3;
  const LINE_DESC_PATH_2 = 4;
  const NUM_ROWS = 4;
  const NUM_COLS = 20;
  /** Pistes grille en px (doublon volontaire avec le CSS : certains WebView n’appliquent pas bien `repeat()` seul). */
  const CELL_PX = 56;
  const MATRIX_ROW_HEIGHTS = `${CELL_PX}px `.repeat(NUM_ROWS).trimEnd();
  const MATRIX_GRID_HEIGHT_PX = NUM_ROWS * CELL_PX;

  /*
   * ─── REVERT : matrice 5 lignes + rangée 3 « séparateur » (piste 0 px + barre + pastille debug) ───
   * const LINE_PATH_1 = 1;
   * const LINE_DESC_PATH_1 = 2;
   * const LINE_SEPARATOR = 3;
   * const LINE_PATH_2 = 4;
   * const LINE_DESC_PATH_2 = 5;
   * const NUM_ROWS = 5;
   * const MATRIX_ROW_HEIGHTS = `${CELL_PX}px ${CELL_PX}px 0px ${CELL_PX}px ${CELL_PX}px`;
   * const MATRIX_GRID_HEIGHT_PX = 4 * CELL_PX;
   * Puis rétablir la boucle `if (row === LINE_SEPARATOR) { ... bar ... wrapCell(...,3)... continue }`,
   * `v.style.gridRow = "3 / 5"`, et dans wrapCell : `if (row === LINE_SEPARATOR) cls += " hx-matrix-cell--row-line-debug-sep"`.
   * CSS : `grid-template-rows: 56px 56px 0px 56px 56px` + décommenter `.hx-matrix-separator-bar` et
   * `.hx-matrix-cell--row-line-debug-sep`.
   */
  grid.style.display = "grid";
  grid.style.gridTemplateColumns = `repeat(${NUM_COLS}, ${CELL_PX}px)`;
  grid.style.gridTemplateRows = MATRIX_ROW_HEIGHTS;
  grid.style.width = `${NUM_COLS * CELL_PX}px`;
  grid.style.minWidth = `${NUM_COLS * CELL_PX}px`;
  grid.style.maxWidth = `${NUM_COLS * CELL_PX}px`;
  grid.style.height = `${MATRIX_GRID_HEIGHT_PX}px`;
  grid.style.minHeight = `${MATRIX_GRID_HEIGHT_PX}px`;
  grid.style.maxHeight = `${MATRIX_GRID_HEIGHT_PX}px`;
  grid.style.boxSizing = "border-box";

  function wrapCell(
    row: number,
    col: number,
    inner: HTMLElement | null,
    opts?: { descriptionPathRow?: boolean; r2JunctionVertical?: boolean; rowLineDebug?: boolean },
  ): HTMLElement {
    const w = document.createElement("div");
    let cls = "hx-matrix-cell" + (inner ? "" : " hx-matrix-cell--empty");
    if (opts?.descriptionPathRow) cls += " hx-matrix-cell--description-path";
    if (opts?.r2JunctionVertical) cls += " hx-matrix-cell--r2-junction-vline";
    if (opts?.rowLineDebug) {
      cls += " hx-matrix-cell--row-line-debug";
      // REVERT (5 lignes + séparateur) : if (row === LINE_SEPARATOR) cls += " hx-matrix-cell--row-line-debug-sep";
    }
    w.className = cls;
    w.style.gridRow = String(row);
    w.style.gridColumn = String(col);
    w.style.boxSizing = "border-box";
    w.style.width = `${CELL_PX}px`;
    w.style.height = `${CELL_PX}px`;
    w.style.maxWidth = `${CELL_PX}px`;
    w.style.maxHeight = `${CELL_PX}px`;
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

  const splitG = showRoutingUi ? matrixEvenColForRoutingBoundary(routingCols.splitCol) : -1;
  const mergeG = showRoutingUi ? matrixEvenColForRoutingBoundary(routingCols.mergeCol) : -1;
  const junctionDecoCols = new Set<number>();
  if (splitG >= 2) junctionDecoCols.add(splitG);
  if (mergeG >= 2) junctionDecoCols.add(mergeG);

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

    const descriptionPathRow = row === LINE_DESC_PATH_1 || row === LINE_DESC_PATH_2;
    for (let col = 1; col <= NUM_COLS; col += 1) {
      let inner: HTMLElement | null = null;

      if (col === NUM_COLS) {
        inner = makeMatrixRowLineLabel(row);
      } else if (row === LINE_DESC_PATH_1 && col === 1) {
        inner = makeMatrixDescriptionLabel("input");
      } else if (row === LINE_DESC_PATH_1 && col === 19) {
        inner = makeMatrixDescriptionLabel("Main L/R");
      } else if (row === LINE_DESC_PATH_1 && junctionDecoCols.has(col)) {
        inner = makeMatrixDescRowVerticalIcon();
      } else if (col === 1) {
        if (row === LINE_PATH_1) {
          inner = makeIoNode("input");
          inner.dataset.hwSlotBus = "0";
          bindSlotParamsInteraction(inner, path1IoSlot("input"));
        }
      } else if (col === 2) {
        if (row === LINE_PATH_1) {
          const rk: "split" | "merge" | null =
            showRoutingUi && routingCols.splitCol === 0 ? "split" : null;
          if (rk === "split") inner = makePathRoutingNode("split");
          else inner = makeMatrixPath1LineIcon();
          if (rk === "split") inner.dataset.hwSlotBus = "10";
          else if (rk === "merge") inner.dataset.hwSlotBus = "19";
          // Frontière 0: séparateur immédiatement après l'Input (Path 1).
          bindSlotParamsInteraction(
            inner,
            rk === null ? null : path1SeparatorSlot(0, rk, rk === "split" ? splitEntry : undefined),
          );
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
            inner = gridSlotNode(slots[i]!, i, { path: 1 });
          else if (row === LINE_DESC_PATH_1) inner = makeMatrixCategoryCell(slots[i]!, i);
          else if (row === LINE_PATH_2 && hasBranchB)
            inner = gridSlotNode(slots[8 + i]!, 8 + i, {
              path: 2,
              path2SlotRowIndex: i,
              splitCol: routingCols.splitCol,
              mergeCol: routingCols.mergeCol,
            });
          else if (row === LINE_DESC_PATH_2) inner = makeMatrixCategoryCell(slots[8 + i]!, 8 + i);
        }
      } else if (col >= 4 && col <= 18 && (col - 4) % 2 === 0) {
        const j = (col - 4) / 2;
        if (row === LINE_PATH_1 && j >= 0 && j <= 7) {
          const boundary = j + 1;
          const rk = routingKindAtBoundary(boundary);
          inner = routingAtBoundary(boundary);
          if (inner === null) inner = makeMatrixPath1LineIcon();
          if (rk === "split") inner.dataset.hwSlotBus = "10";
          else if (rk === "merge") inner.dataset.hwSlotBus = "19";
          // Étape 1 UX: tous les séparateurs Path 1 sont cliquables comme les slots.
          bindSlotParamsInteraction(
            inner,
            rk === null
              ? null
              : path1SeparatorSlot(
                  boundary,
                  rk,
                  rk === "split" ? splitEntry : rk === "merge" ? mergeEntry : undefined,
                ),
          );
        } else if (row === LINE_PATH_2 && hasBranchB && j >= 0 && j <= 7) {
          // Path 2 (L3): réafficher `Icons_line.png` entre split et merge.
          const boundary = j + 1;
          if (boundary > routingCols.splitCol && boundary < routingCols.mergeCol) {
            inner = makeMatrixPath2LineIcon();
          }
        }
      }

      const r2JunctionVertical =
        showRoutingUi && row === LINE_DESC_PATH_1 && junctionDecoCols.has(col) && inner !== null;
      const rowLineDebug = col === NUM_COLS;
      grid.appendChild(
        wrapCell(row, col, inner, {
          descriptionPathRow: descriptionPathRow && !rowLineDebug,
          r2JunctionVertical,
          rowLineDebug,
        }),
      );
    }
  }

  if (showRoutingUi) {
    /**
     * `false` : pas d’overlay trait sur « Path 2 » (L3) dans les colonnes split/merge — même grille
     * que `Icons_link_*` → superposition. Le trait vertical reste en « Description Path 1 » (L2).
     * Mettre à `true` pour réactiver (REVERT visuel) ; avec 5 lignes séparateur, utiliser `grid-row` adapté.
     */
    const ENABLE_MATRIX_VSPAN_ON_PATH2 = false;
    if (ENABLE_MATRIX_VSPAN_ON_PATH2 && hasBranchB) {
      for (const col of junctionDecoCols) {
        const v = makeMatrixVerticalSpanIcon();
        v.style.gridRow = "3 / 4";
        v.style.gridColumn = String(col);
        v.style.boxSizing = "border-box";
        v.style.zIndex = "2";
        v.style.pointerEvents = "none";
        grid.appendChild(v);
      }
    }

    function placePath2Link(col: number, src: string) {
      const el = makeMatrixPath2LinkIcon(src);
      el.style.gridRow = String(LINE_PATH_2);
      el.style.gridColumn = String(col);
      el.style.boxSizing = "border-box";
      el.style.zIndex = "2";
      el.style.pointerEvents = "none";
      grid.appendChild(el);
    }
    // Sans bloc sur le path 2, pas d’icônes split/merge sur L3 (évite split/merge « orphelins »).
    if (hasBranchB) {
      if (splitG >= 2) placePath2Link(splitG, MATRIX_ICON_LINK_SPLIT);
      if (mergeG >= 2) placePath2Link(mergeG, MATRIX_ICON_LINK_MERGE);
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
  hardwareSyncPausedForPresetLoad = true;
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

  try {
    lastRequestPresetInvokeAt = Date.now();
    await invoke("request_preset_content");
  } catch (e) {
    const msg = String(e);
    if (
      msg.includes("molette modèle") ||
      msg.includes("scroll/pull") ||
      msg.includes("Initialisation USB")
    ) {
      pendingPresetIndex = index;
      loading = false;
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
          await logCatalogChainHexDiffIfNeeded(normalizedSlots, index);
          await renderSlots(normalizedSlots, routingFlow, stompLayout);
          rememberHwSyncChainLayout(normalizedSlots);
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

          // Le nom affiché est piloté par la liste presets backend (refresh global).
          // On évite ici un invoke additionnel request_active_preset_name qui peut
          // amplifier les rafales pendant les changements rapides de preset.
        }
        loading = false;
        hardwareSyncPausedForPresetLoad = pendingPresetIndex >= 0;
        if (pendingPresetIndex >= 0 && pendingPresetIndex !== loadedPresetIndex) {
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

window.addEventListener("DOMContentLoaded", () => {
  hwUi.configure({ setParamsBrowsingMode: setModelsParamsBrowsingMode });
  void mountModelsSlotPicker();

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

  void listen<string>("helix-device-lost", () => {
    purgeModelsUi();
  });

  void refresh();
  window.setInterval(() => {
    void refresh();
  }, 300);
  startOptionalUsbPresetPollTimer();
});
