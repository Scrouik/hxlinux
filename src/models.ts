import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  getCatalogModelIdForModel,
  getCatalogModelImageForModel,
  getPresetMetaForModel,
  pickChannel,
  pickEmulationName,
  pickSignal,
} from "./hxModelCatalogMeta";
import "./styles.css";

let currentPresetIndex = -1;
let loadedPresetIndex = -1;
let loading = false;
let pendingPresetIndex = -1;
let lastRequestedPresetIndex = -1;
const ENABLE_PRESET_CONTENT = true;
const DEBUG_MODEL_ID_JOIN_FALLBACK =
  localStorage.getItem("models_debug_id_join") === "1";
let requestedPresetNameIndex = -1;
let debugRoutingMode = localStorage.getItem("models_debug_routing") === "1";
let connectedDeviceName: string | null = null;

const statusEl = document.getElementById("status") as HTMLElement;
const presetLabelEl = document.getElementById("preset-label") as HTMLElement;
const contentEl = document.getElementById("content") as HTMLElement;

type SlotDebug = {
  category: string;
  name: string;
  gridX?: string;
  gridY?: string;
  /** Hex module preset (Rust), pour presetMeta.signal / chainHex parallèles. */
  moduleHex?: string;
};

const MODELS_PARAMS_IDLE_PLACEHOLDER =
  "Les paramètres du bloc sélectionné s'afficheront ici.";

function getModelsParamsInner(): HTMLElement | null {
  return document.querySelector("#models-params-pane .models-params-inner");
}

function getModelsParamsPaneTitleEl(): HTMLElement | null {
  return document.getElementById("models-params-pane-title");
}

function getModelsParamsSubheadEl(): HTMLElement | null {
  return document.getElementById("models-params-pane-subhead");
}

function getModelsParamsModelIconWrapEl(): HTMLElement | null {
  return document.getElementById("models-params-pane-model-icon-wrap");
}

function getModelsParamsEmulationNameEl(): HTMLElement | null {
  return document.getElementById("models-params-pane-emulation-name");
}

function setModelsParamsPaneEmulationName(text: string | null): void {
  const el = getModelsParamsEmulationNameEl();
  if (!el) return;
  if (!text) {
    el.textContent = "";
    el.removeAttribute("title");
    el.hidden = true;
    return;
  }
  el.textContent = text;
  el.title = text;
  el.hidden = false;
}

/** Vide le sous-titre modèle, `emulationName` et l’icône sous le bandeau titre. */
function clearModelsParamsSubheadAndIcon(): void {
  getModelsParamsSubheadEl()?.replaceChildren();
  setModelsParamsPaneEmulationName(null);
  disposeModelsParamsIconLivePreview();
  hideModelsParamsIconPreviewPopover();
  getModelsParamsModelIconWrapEl()?.replaceChildren();
}

/** En-tête du panneau : nom de catégorie du modèle (ex. « Amp »), vide si aucun bloc ciblé. */
function setModelsParamsPaneCategory(category: string) {
  const el = getModelsParamsPaneTitleEl();
  if (!el) return;
  el.textContent = category.trim();
}

let selectedParamsSlotEl: HTMLElement | null = null;

function clearSlotSelectionVisual() {
  if (selectedParamsSlotEl) {
    selectedParamsSlotEl.classList.remove("node--selected");
    selectedParamsSlotEl = null;
  }
}

function resetModelsParamsIdleHint() {
  setModelsParamsPaneCategory("");
  clearModelsParamsSubheadAndIcon();
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
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
}

/**
 * `slot === null` : slot vide (clic → rien dans le panneau).
 * Sinon : bloc avec modèle (définitions `.models` + liste paramètre / valeur).
 */
function bindSlotParamsInteraction(el: HTMLElement, slot: SlotDebug | null) {
  el.classList.add("node--params-clickable");
  el.tabIndex = 0;
  el.setAttribute("role", "button");
  const activate = () => {
    if (slot === null) {
      clearSlotSelectionVisual();
      clearModelsParamsPaneContent();
      return;
    }
    clearSlotSelectionVisual();
    selectedParamsSlotEl = el;
    el.classList.add("node--selected");
    const kRaw = el.dataset.kemplineSlotIndex;
    const kemplineSlotIndex =
      kRaw !== undefined && kRaw !== "" ? Number.parseInt(kRaw, 10) : undefined;
    void loadAndShowModelsParamsForSlot(
      slot,
      Number.isFinite(kemplineSlotIndex) ? kemplineSlotIndex : undefined,
    );
  };
  el.addEventListener("click", (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    activate();
  });
  el.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter" || ev.key === " ") {
      ev.preventDefault();
      activate();
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

function renderEmpty(text: string) {
  clearSlotSelectionVisual();
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
  displayType?: string;
  /** 0 = entier, 1 = float, 2 = bool (Line 6). */
  valueType?: number;
  /** JSON Line 6 : souvent nombres ; bool pour `off_on` (ex. Bright / Contour). */
  min?: number | boolean;
  max?: number | boolean;
  default?: number | string | boolean;
  "stereo-only"?: boolean;
};

type ModelDefinitionJson = {
  symbolicID?: string;
  name: string;
  params?: ModelParamDefJson[];
};

const modelsDefinitionsCache = new Map<string, ModelDefinitionJson[]>();
let modelsParamsLoadSeq = 0;

/** Bases de fichiers `.models` à essayer dans l’ordre (sans extension). */
function modelsDefinitionFileBasesForCategory(category: string): string[] {
  const k = normalizeCategory(category);
  const m: Record<string, string[]> = {
    amp: ["amp"],
    preamp: ["preamp"],
    "amp+cab": ["amp", "cab", "preamp"],
    cab: ["cab"],
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
  };
  return m[k] ?? [];
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
  const parsed = JSON.parse(raw) as unknown;
  if (!Array.isArray(parsed)) {
    throw new Error("Format .models invalide (tableau attendu).");
  }
  modelsDefinitionsCache.set(fileBase, parsed as ModelDefinitionJson[]);
  return parsed as ModelDefinitionJson[];
}

/**
 * Le nom de slot côté USB / `parse_preset_slots` vient de `MODULES_BY_ID`
 * (Rust : `HX_ModelCatalog.json` uniquement via `presetMeta.chainHex`) : libellé slot = **nom court**
 * du modèle dans le catalogue pour les hex connus ; sinon le preset n’a pas d’entrée et le slot
 * peut rester « Unknown » jusqu’à ce que `chainHex` soit renseigné dans le catalogue.
 * « Ampeg SVT Brt Bass Ampeg SVT (bright channel) (mono) ».
 * Les fichiers `*.models` utilisent le `name` court (« Ampeg SVT Brt ») + `symbolicID`.
 */
function stripKemplineMonoStereoSuffix(s: string): string {
  return s.replace(/\s*\((mono|stereo)\)\s*$/i, "").trim();
}

/**
 * Ex. catalogue `SVT-4 Pro` vs USB `Ampeg SVT-4 PRO (mono)` → après strip, le libellé USB se termine
 * par le `name` court du `.models` (préfixe marque / casse différente).
 */
function kemplineStrippedEndsWithCatalogName(knStrip: string, catalogName: string): boolean {
  const k = knStrip.trim();
  const c = catalogName.trim();
  if (!k || !c || c.length > k.length) return false;
  if (k.toLowerCase() === c.toLowerCase()) return true;
  if (!k.toLowerCase().endsWith(c.toLowerCase())) return false;
  if (k.length === c.length) return true;
  const before = k[k.length - c.length - 1];
  return before === " " || before === "\t" || before === "(";
}

function modelCatalogNameMatchesKemplineSlot(catalogName: string, kemplineSlotName: string): boolean {
  const cn = catalogName.trim();
  const kn = kemplineSlotName.trim();
  if (!cn || !kn) return false;
  if (kn === cn || kn.toLowerCase() === cn.toLowerCase()) return true;
  if (kn.startsWith(`${cn} `) || kn.startsWith(`${cn}(`)) return true;
  const knStrip = stripKemplineMonoStereoSuffix(kn);
  if (knStrip === cn || knStrip.toLowerCase() === cn.toLowerCase()) return true;
  if (knStrip.startsWith(`${cn} `) || knStrip.startsWith(`${cn}(`)) return true;
  if (kemplineStrippedEndsWithCatalogName(knStrip, cn)) return true;
  return false;
}

/** Associe le libellé preset Kempline à une entrée du JSON `.models` (meilleur préfixe = `name` le plus long). */
function pickModelDefinitionForKemplineName(
  list: ModelDefinitionJson[],
  kemplineSlotName: string,
): ModelDefinitionJson | null {
  const kn = kemplineSlotName.trim();
  if (!kn) return null;
  const exact = list.find((e) => e.name.trim() === kn);
  if (exact) return exact;
  const stripped = stripKemplineMonoStereoSuffix(kn);
  const exactStrip = list.find((e) => e.name.trim() === stripped);
  if (exactStrip) return exactStrip;
  const exactStripI = list.find(
    (e) => e.name.trim().toLowerCase() === stripped.toLowerCase(),
  );
  if (exactStripI) return exactStripI;

  let best: ModelDefinitionJson | null = null;
  let bestLen = -1;
  for (const e of list) {
    const n = (e.name || "").trim();
    if (!n) continue;
    if (!modelCatalogNameMatchesKemplineSlot(n, kn)) continue;
    if (n.length > bestLen) {
      bestLen = n.length;
      best = e;
    }
  }
  return best;
}

async function findModelDefinitionForSlot(
  slot: SlotDebug,
  catalogModelId?: string | null,
): Promise<{ entry: ModelDefinitionJson; fileBase: string } | null> {
  const nameTarget = slot.name.trim();
  if (!nameTarget || nameTarget === "<empty>") return null;
  const idTarget = (catalogModelId ?? "").trim();
  const bases = modelsDefinitionFileBasesForCategory(slot.category);
  if (bases.length === 0) return null;
  for (const fileBase of bases) {
    let list: ModelDefinitionJson[];
    try {
      list = await loadModelsDefinitionArray(fileBase);
    } catch {
      continue;
    }
    if (idTarget) {
      const byId = list.find((e) => (e.symbolicID || "").trim() === idTarget);
      if (byId) return { entry: byId, fileBase };
    }
    const entry = pickModelDefinitionForKemplineName(list, nameTarget);
    if (entry) {
      if (idTarget && DEBUG_MODEL_ID_JOIN_FALLBACK) {
        console.warn(
          `[models] Fallback nom utilise (ID introuvable): id=${idTarget} category="${slot.category}" slot="${nameTarget}" matched="${entry.name}" file="${fileBase}.models"`,
        );
      }
      return { entry, fileBase };
    }
  }
  if (idTarget && DEBUG_MODEL_ID_JOIN_FALLBACK) {
    console.warn(
      `[models] Aucun match par ID ni par nom: id=${idTarget} category="${slot.category}" slot="${nameTarget}" tried=${bases.join(",")}`,
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

  const formatted = format ? formatWithPrintfFloat(value, format) : null;
  if (formatUnits) {
    if (formatted !== null && HELIX_PRINTF_TOKEN_RE.test(formatUnits)) {
      return helixUnescapePercentMarks(formatUnits.replace(HELIX_PRINTF_TOKEN_RE, formatted));
    }
    return helixUnescapePercentMarks(formatUnits);
  }
  if (formatted !== null) return formatted;
  if (control.isDiscrete) {
    return String(Math.round(rawValue));
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
        return formatHelixFromControl(v, def, controlKey);
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

/** `true` / `false` pour bool ; `0` / `1` pour entiers discrets ; sinon pas d’UI bool. */
function chainValueAsBool(cv: ChainParamValueJson): boolean | null {
  if (typeof cv === "boolean") return cv;
  if (typeof cv === "number" && Number.isFinite(cv)) {
    if (cv === 0 || cv === 1) return cv !== 0;
    return null;
  }
  return null;
}

function isOffOnDisplayType(displayType: string | undefined): boolean {
  const t = (displayType ?? "").trim().toLowerCase();
  return t === "off_on";
}

function boolToggleLabels(
  p: ModelParamDefJson,
  helixControlsMap?: Map<string, HelixControlDefJson>,
): [string, string] {
  const dt = (p.displayType ?? "").trim();
  const def = dt ? helixControlsMap?.get(dt) : undefined;
  const fmt = def?.format;
  if (Array.isArray(fmt) && fmt.length >= 2 && typeof fmt[0] === "string" && typeof fmt[1] === "string") {
    return [fmt[0] as string, fmt[1] as string];
  }
  return ["Off", "On"];
}

function canModelsParamsBoolToggle(p: ModelParamDefJson, cv: ChainParamValueJson | undefined): boolean {
  if (cv === undefined) return false;
  if (chainValueAsBool(cv) === null) return false;
  if (p.valueType === 2) return true;
  return isOffOnDisplayType(p.displayType);
}

function showModelsParamsLoading() {
  clearModelsParamsSubheadAndIcon();
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
  catalogChannel?: string | null,
  catalogSignal?: string | null,
  catalogEmulationName?: string | null,
  helixControlsMap?: Map<string, HelixControlDefJson>,
  catalogModelImage?: string | null,
) {
  setModelsParamsPaneCategory(slot.category);
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  const head = document.createElement("div");
  head.className = "models-params-model-head";
  const title = document.createElement("div");
  title.className = "models-params-model-title";
  const baseName = (resolvedCatalogModelName ?? slot.name).trim() || "—";
  const parts: string[] = [baseName];
  const ch = (catalogChannel ?? "").trim();
  const sig = (catalogSignal ?? "").trim();
  if (ch) parts.push(ch);
  if (sig) parts.push(sig);
  title.textContent = parts.join(" · ");
  if (resolvedCatalogModelName && resolvedCatalogModelName.trim() !== slot.name.trim()) {
    const usb = document.createElement("div");
    usb.className = "models-params-model-usb-name";
    usb.textContent = slot.name.trim();
    head.append(title, usb);
  } else {
    head.append(title);
  }
  const subhead = getModelsParamsSubheadEl();
  if (subhead) {
    subhead.replaceChildren(head);
  } else {
    inner.appendChild(head);
  }
  setModelsParamsPaneEmulationName(
    catalogEmulationName && catalogEmulationName.trim()
      ? catalogEmulationName.trim()
      : null,
  );
  setModelsParamsHeaderIcon(slot, catalogModelImage);

  const list = document.createElement("ul");
  list.className = "models-params-list";
  for (let j = 0; j < params.length; j += 1) {
    const p = params[j];
    if (paramHiddenForMonoStereoOnly(p, catalogSignal)) continue;
    const li = document.createElement("li");
    li.className = "models-params-row";
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
    const rawTitleStr =
      cv !== undefined ? formatRawChainParamValueJson(cv) : "—";
    li.title = rawTitleStr;
    sliderCell.title = rawTitleStr;
    li.append(label, minEl, sliderCell, maxEl);

    const minN = p.min;
    const maxN = p.max;
    const canSlider =
      typeof cv === "number" &&
      Number.isFinite(cv) &&
      typeof minN === "number" &&
      typeof maxN === "number" &&
      Number.isFinite(minN) &&
      Number.isFinite(maxN) &&
      maxN > minN;
    if (canSlider) {
      sliderCell.append(chainEl);
      const dt = (p.displayType ?? "").trim();
      const helixDef =
        dt && helixControlsMap?.has(dt) ? helixControlsMap.get(dt)! : undefined;
      let inc = helixDef ? helixRawIncrementFromStep(cv, helixDef) : null;
      if (inc === null || !Number.isFinite(inc) || inc <= 0) {
        inc = fallbackRawIncrement(p, minN, maxN);
      }
      const init = snapRawToIncrement(cv, minN, maxN, inc, p.valueType);
      const input = document.createElement("input");
      input.type = "range";
      input.className = "models-params-slider";
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
      }
      input.title = rawTitleStr;
      input.setAttribute(
        "aria-label",
        `${(p.name || p.symbolicID || "").trim()} — aperçu local, non envoyé au Helix`,
      );
      input.addEventListener("input", () => {
        let v = Number(input.value);
        if (!Number.isFinite(v)) return;
        v = snapRawToIncrement(v, minN, maxN, inc, p.valueType);
        if (Number(input.value) !== v) input.value = String(v);
        chainEl.textContent = formatChainParamValueJson(v, p, helixControlsMap);
        const s = formatRawChainParamValueJson(v);
        li.title = s;
        sliderCell.title = s;
        input.title = s;
      });
      sliderCell.append(input);
    } else if (cv !== undefined && canModelsParamsBoolToggle(p, cv)) {
      sliderCell.append(chainEl);
      let currentB = chainValueAsBool(cv)!;
      const [labOff, labOn] = boolToggleLabels(p, helixControlsMap);
      const rowWrap = document.createElement("div");
      rowWrap.className = "models-params-bool-toggle";
      const bOff = document.createElement("button");
      bOff.type = "button";
      bOff.className = "models-params-bool-btn";
      bOff.textContent = labOff;
      const bOn = document.createElement("button");
      bOn.type = "button";
      bOn.className = "models-params-bool-btn";
      bOn.textContent = labOn;
      const paramLabel = `${(p.name || p.symbolicID || "").trim()} — aperçu local, non envoyé au Helix`;
      bOff.setAttribute("aria-label", `${paramLabel} : ${labOff}`);
      bOn.setAttribute("aria-label", `${paramLabel} : ${labOn}`);
      const syncButtons = (): void => {
        bOff.classList.toggle("models-params-bool-btn--selected", !currentB);
        bOn.classList.toggle("models-params-bool-btn--selected", currentB);
      };
      const applyBool = (nextB: boolean): void => {
        currentB = nextB;
        const v: ChainParamValueJson = typeof cv === "boolean" ? nextB : nextB ? 1 : 0;
        chainEl.textContent = formatChainParamValueJson(v, p, helixControlsMap);
        const s = formatRawChainParamValueJson(v);
        li.title = s;
        sliderCell.title = s;
        syncButtons();
      };
      bOff.addEventListener("click", () => {
        if (!currentB) return;
        applyBool(false);
      });
      bOn.addEventListener("click", () => {
        if (currentB) return;
        applyBool(true);
      });
      syncButtons();
      rowWrap.append(bOff, bOn);
      sliderCell.append(rowWrap);
    } else {
      sliderCell.append(chainEl);
    }

    list.appendChild(li);
  }
  inner.appendChild(list);
}

function showModelsParamsNotFound(slot: SlotDebug) {
  setModelsParamsPaneCategory(slot.category);
  clearModelsParamsSubheadAndIcon();
  const inner = getModelsParamsInner();
  if (!inner) return;
  inner.replaceChildren();
  const p = document.createElement("p");
  p.className = "models-params-placeholder";
  p.textContent = `Aucune définition « ${slot.name.trim()} » pour la catégorie « ${slot.category.trim()} ».`;
  inner.appendChild(p);
}

function showModelsParamsError(message: string) {
  setModelsParamsPaneCategory("");
  clearModelsParamsSubheadAndIcon();
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
  const seq = ++modelsParamsLoadSeq;
  setModelsParamsPaneCategory(slot.category);
  showModelsParamsLoading();
  const nk = normalizeCategory(slot.category);
  if (nk === "routing" || nk === "none" || nk === "favorites") {
    if (seq === modelsParamsLoadSeq) showModelsParamsNotFound(slot);
    return;
  }
  try {
    let chainValues: ChainParamValueJson[] | null = null;
    if (kemplineSlotIndex !== undefined && Number.isInteger(kemplineSlotIndex)) {
      try {
        chainValues = await invoke<ChainParamValueJson[] | null>(
          "get_active_preset_slot_chain_param_values",
          { slotIndex: kemplineSlotIndex },
        );
      } catch {
        chainValues = null;
      }
    }
    const catalogModelId = await getCatalogModelIdForModel(slot.category, slot.name);
    if (seq !== modelsParamsLoadSeq) return;
    const found = await findModelDefinitionForSlot(slot, catalogModelId);
    if (seq !== modelsParamsLoadSeq) return;
    if (!found) {
      showModelsParamsNotFound(slot);
      return;
    }
    const short = found.entry.name.trim();
    kemplineTooltipCache.set(tooltipCacheKey(slot), short);
    applyShortNameToSlotNodes(slot, short);
    const [meta, catalogImage] = await Promise.all([
      getPresetMetaForModel(slot.category, short),
      getCatalogModelImageForModel(slot.category, short),
    ]);
    if (seq !== modelsParamsLoadSeq) return;
    const catalogChannel = pickChannel(meta);
    const catalogSignal = pickSignal(meta, slot.moduleHex);
    const catalogEmulationName = pickEmulationName(meta);
    const helixControlsMap = await getHelixControlsMap();
    if (seq !== modelsParamsLoadSeq) return;
    renderModelsParamsPane(
      slot,
      found.entry.params ?? [],
      short,
      chainValues,
      catalogChannel,
      catalogSignal,
      catalogEmulationName,
      helixControlsMap,
      catalogImage,
    );
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
  const catalogModelId = await getCatalogModelIdForModel(slot.category, slot.name);
  const found = await findModelDefinitionForSlot(slot, catalogModelId);
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

function iconForCategory(category: string, name: string): string | null {
  const key = normalizeCategory(category);
  if (key === "routing" && name.toLowerCase().includes("merge")) {
    return "/src-tauri/resources/icons_category/FX_HX_Category_Merge.png";
  }
  const filename = CATEGORY_ICON_BY_KEY[key];
  if (!filename) return null;
  return `/src-tauri/resources/icons_category/${filename}`;
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
  if (safe) src = `/src-tauri/resources/icons_models/${safe}`;
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
    img.width = 22;
    img.height = 22;
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

function makeEmptySlotNode(): HTMLElement {
  const item = document.createElement("div");
  item.className = "node node-empty node--hx-slot node-empty-flat";
  item.title = "Slot vide";
  item.setAttribute("aria-label", "Slot vide");
  bindSlotParamsInteraction(item, null);
  return item;
}

const IO_INPUT_ICON = "/src-tauri/resources/icons_category/icon-input-category.png";
const IO_OUTPUT_ICON = "/src-tauri/resources/icons_category/icon-output-category.png";
const MATRIX_PATH1_LINE_ICON = "/src-tauri/resources/icons_category/Icons_line.png";
const MATRIX_PATH1_SPLIT_MERGE_ICON =
  "/src-tauri/resources/icons_category/Icons_split_merge.png";
const MATRIX_ICON_VERTICAL = "/src-tauri/resources/icons_category/Icons_vertical_line.png";
const MATRIX_ICON_LINK_SPLIT = "/src-tauri/resources/icons_category/Icons_link_split.png";
const MATRIX_ICON_LINK_MERGE = "/src-tauri/resources/icons_category/Icons_link_merge.png";

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
  wrap.className = `hx-matrix-routing-marker hx-matrix-routing-marker--${kind}`;
  wrap.dataset.routingMarker = kind;
  const img = document.createElement("img");
  img.className = "hx-matrix-routing-marker-img";
  img.src = MATRIX_PATH1_SPLIT_MERGE_ICON;
  img.alt = "";
  img.decoding = "async";
  wrap.appendChild(img);
  return wrap;
}

function gridSlotNode(slot: SlotDebug, kemplineSlotIndex: number): HTMLElement {
  if (!slot.category && slot.name === "<empty>") {
    const n = makeEmptySlotNode();
    n.dataset.kemplineSlotIndex = String(kemplineSlotIndex);
    return n;
  }
  /* Matrice : sur « Path 1 » / « Path 2 », la catégorie est sur la ligne Description ; la cellule slot = icône + infobulle nom. */
  const node = makeNode(slot, { showTypeAbbrev: false });
  node.dataset.kemplineSlotIndex = String(kemplineSlotIndex);
  return node;
}

/** Libellé catégorie : « Description Path 1 » (L2) ou « Description Path 2 » (L4) sous un slot. */
function makeMatrixCategoryCell(slot: SlotDebug): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-matrix-category";
  const empty = !slot.category && slot.name === "<empty>";
  if (empty) {
    el.textContent = "empty";
    el.title = "empty";
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
  routing: [string, string][],
  stompLayout: ActivePresetStompLayout | null,
) {
  const lastB = lastFilledSlotRowIndex(slots, 8, 8);
  const hasBranchB = lastB >= 0;
  /** Marqueurs routage affichés : API et/ou branche B réelle (sans quoi routingCols restait null). */
  const showRoutingUi = routing.length > 0 || hasBranchB;

  const routingCols =
    stompLayout != null && stompLayout.routing.kemplineGridOk === true
      ? {
          splitCol: stompLayout.routing.splitAfterCol,
          mergeCol: stompLayout.routing.mergeAfterCol,
        }
      : computeRoutingJunctionColumns(slots);

  const splitEntry = routing.find(([, name]) => name.toLowerCase().includes("split"));
  const mergeEntry = routing.find(([, name]) => name.toLowerCase().includes("merge"));
  const splitTip = splitEntry ? `${splitEntry[0]}: ${splitEntry[1]}` : "Split";
  const mergeTip = mergeEntry ? `${mergeEntry[0]}: ${mergeEntry[1]}` : "Merge";

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
        if (row === LINE_PATH_1) inner = makeIoNode("input");
      } else if (col === 2) {
        if (row === LINE_PATH_1) {
          if (showRoutingUi && routingCols.splitCol === 0) inner = makePathRoutingNode("split");
          else inner = makeMatrixPath1LineIcon();
        }
      } else if (col === 19) {
        if (row === LINE_PATH_1) inner = makeIoNode("output");
      } else if (col >= 3 && col <= 17 && (col - 3) % 2 === 0) {
        const i = (col - 3) / 2;
        if (i >= 0 && i <= 7) {
          if (row === LINE_PATH_1) inner = gridSlotNode(slots[i]!, i);
          else if (row === LINE_DESC_PATH_1) inner = makeMatrixCategoryCell(slots[i]!);
          else if (row === LINE_PATH_2 && showRoutingUi && hasBranchB)
            inner = gridSlotNode(slots[8 + i]!, 8 + i);
          else if (row === LINE_DESC_PATH_2) inner = makeMatrixCategoryCell(slots[8 + i]!);
        }
      } else if (col >= 4 && col <= 18 && (col - 4) % 2 === 0) {
        const j = (col - 4) / 2;
        if (row === LINE_PATH_1 && j >= 0 && j <= 7) {
          inner = routingAtBoundary(j + 1);
          if (inner === null) inner = makeMatrixPath1LineIcon();
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
    if (ENABLE_MATRIX_VSPAN_ON_PATH2) {
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
    if (splitG >= 2) placePath2Link(splitG, MATRIX_ICON_LINK_SPLIT);
    if (mergeG >= 2) placePath2Link(mergeG, MATRIX_ICON_LINK_MERGE);
  }

  root.appendChild(grid);

  clearSlotSelectionVisual();
  contentEl.innerHTML = "";
  contentEl.appendChild(root);
  resetModelsParamsIdleHint();
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
  routingFromFlow: [string, string][] = [],
  stompLayout: ActivePresetStompLayout | null = null,
) {
  if (rawSlots.length === 0) {
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
  contentEl.innerHTML = "";
  contentEl.appendChild(root);
  resetModelsParamsIdleHint();
  await refreshAllSlotTooltipsInContent();
}

async function requestLoadForPreset(index: number) {
  if (!ENABLE_PRESET_CONTENT) {
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

  loading = true;
  pendingPresetIndex = -1;
  lastRequestedPresetIndex = index;
  setStatus("Lecture du preset actif...");
  console.log(`[PresetDebug][models] request_preset_content preset=${index}`);

  try {
    await invoke("request_preset_content");
  } catch (e) {
    console.error("[PresetDebug][models] request_preset_content error", e);
    setStatus(`Erreur: ${e}`);
    loading = false;
    if (pendingPresetIndex >= 0) {
      const next = pendingPresetIndex;
      pendingPresetIndex = -1;
      void requestLoadForPreset(next);
    }
    return;
  }

  let tries = 0;
  const timer = window.setInterval(async () => {
    tries += 1;
    if (tries > 45) {
      window.clearInterval(timer);
      console.warn(`[PresetDebug][models] timeout preset=${index}`);
      setStatus("Timeout lecture preset.");
      loading = false;
      if (pendingPresetIndex >= 0) {
        const next = pendingPresetIndex;
        pendingPresetIndex = -1;
        void requestLoadForPreset(next);
      }
      return;
    }

    try {
      const slots = debugRoutingMode
        ? await invoke<[string, string, string, string][] | null>("get_active_preset_slots_debug")
        : await invoke<[string, string][] | null>("get_active_preset_slots");
      if (slots !== null) {
        window.clearInterval(timer);
        console.log(`[PresetDebug][models] slots ready preset=${index} count=${slots.length}`);
        loadedPresetIndex = index;
        const normalizedSlots: SlotDebug[] = debugRoutingMode
          ? (slots as unknown as [string, string, string, string, string][]).map(
              ([category, name, gridX, gridY, moduleHex]) => ({
                category,
                name,
                gridX,
                gridY,
                moduleHex: moduleHex?.trim() || undefined,
              }),
            )
          : (slots as unknown as [string, string, string][]).map(([category, name, moduleHex]) => ({
              category,
              name,
              moduleHex: moduleHex?.trim() || undefined,
            }));
        // Evite d'afficher une vieille réponse si l'utilisateur a recliqué ailleurs.
        if (currentPresetIndex === index) {
          let routingFlow: [string, string][] = [];
          let stompLayout: ActivePresetStompLayout | null = null;
          if (isKemplineGrid16(normalizedSlots)) {
            try {
              const r = await invoke<[string, string][] | null>("get_active_preset_routing_markers");
              routingFlow = r ?? [];
            } catch {
              console.warn("[PresetDebug][models] get_active_preset_routing_markers error");
            }
            try {
              stompLayout = await invoke<ActivePresetStompLayout | null>("get_active_preset_stomp_layout");
            } catch {
              console.warn("[PresetDebug][models] get_active_preset_stomp_layout error");
            }
          }
          await renderSlots(normalizedSlots, routingFlow, stompLayout);
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

          // Corrige le nom affiché (liste et label) en demandant le nom réel du preset actif.
          // Utile quand `get_preset_names()` peut être temporairement désaligné à cause de presets "vides".
          if (requestedPresetNameIndex !== index) {
            requestedPresetNameIndex = index;
            try {
              await invoke("request_active_preset_name");
            } catch {
              // Best-effort : l'UI sera corrigée au prochain refresh si possible.
            }
          }
        }
        loading = false;
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
    const names = await invoke<string[]>("get_preset_names");
    const active = await invoke<number>("get_active_preset");

    if (active < 0 || active >= names.length) {
      console.warn("[PresetDebug][models] active preset out of range", active, names.length);
      presetLabelEl.textContent = "--";
      renderEmpty("Aucun preset actif.");
      setStatus("En attente...");
      return;
    }

    const displayName = isEmpty(names[active]) ? "empty" : names[active];
    presetLabelEl.textContent = `${padNum(active)} ${displayName}`;

    if (active !== currentPresetIndex) {
      console.log(`[PresetDebug][models] active preset changed ${currentPresetIndex} -> ${active}`);
      currentPresetIndex = active;
      loadedPresetIndex = -1;
      requestedPresetNameIndex = -1;
      renderEmpty("Chargement des modeles...");
      scheduleLoadForPreset(active, true);
    }
    if (!loading && loadedPresetIndex !== currentPresetIndex) {
      scheduleLoadForPreset(currentPresetIndex, false);
    }
  } catch {
    console.warn("[PresetDebug][models] refresh failed (HX disconnected?)");
    setStatus("HX non connecte.");
    presetLabelEl.textContent = "--";
    renderEmpty("En attente du HX...");
  }
}

window.addEventListener("DOMContentLoaded", () => {
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
    currentPresetIndex = index;
    loadedPresetIndex = -1;
    requestedPresetNameIndex = -1;
    renderEmpty("Chargement des modeles...");
    scheduleLoadForPreset(index, true);
  });

  void refresh();
  window.setInterval(() => {
    void refresh();
  }, 300);
});
