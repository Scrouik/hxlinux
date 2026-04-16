import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

let currentPresetIndex = -1;
let loadedPresetIndex = -1;
let loading = false;
let pendingPresetIndex = -1;
let lastRequestedPresetIndex = -1;
const ENABLE_PRESET_CONTENT = true;
let requestedPresetNameIndex = -1;
let debugRoutingMode = localStorage.getItem("models_debug_routing") === "1";
let connectedDeviceName: string | null = null;

const statusEl = document.getElementById("status") as HTMLElement;
const presetLabelEl = document.getElementById("preset-label") as HTMLElement;
const contentEl = document.getElementById("content") as HTMLElement;

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
  contentEl.innerHTML = `<div class="empty">${text}</div>`;
}

type SlotDebug = { category: string; name: string; gridX?: string; gridY?: string };

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

const CATEGORY_ICON_BY_KEY: Record<string, string> = {
  distortion: "FX_HX_Category_Distortion.png",
  dynamic: "FX_HX_Category_Dynamics.png",
  eq: "FX_HX_Category_EQ.png",
  modulation: "FX_HX_Category_Modulation.png",
  delay: "FX_HX_Category_Delay.png",
  reverb: "FX_HX_Category_Reverb.png",
  "pitch/synth": "FX_HX_Category_PitchSynth.png",
  filter: "FX_HX_Category_Filter.png",
  wah: "FX_HX_Category_Wah.png",
  "vol/pan": "FX_HX_Category_VolumePan.png",
  amp: "FX_HX_Category_Amp.png",
  preamp: "FX_HX_Category_Preamp.png",
  "amp+cab": "FX_HX_Category_Amp+Cab.png",
  cab: "FX_HX_Category_Cab.png",
  "impulse response": "FX_HX_Category_Impulse Response.png",
  routing: "FX_HX_Category_Split.png",
};

/** Diminutif du type d’effet (comme HX Edit : Dyn, Dis…), pas du nom du modèle. */
const EFFECT_TYPE_ABBREV: Record<string, string> = {
  distortion: "Dis",
  dynamic: "Dyn",
  dynamics: "Dyn",
  eq: "EQ",
  modulation: "Mod",
  delay: "Del",
  reverb: "Rev",
  "pitch/synth": "Pch",
  filter: "Flt",
  wah: "Wah",
  "vol/pan": "Vol",
  amp: "Amp",
  preamp: "Pre",
  "amp+cab": "AmC",
  cab: "Cab",
  "impulse response": "IR",
};

function normalizeCategory(category: string): string {
  return category.trim().toLowerCase();
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

/** Infobulle : nom du modèle uniquement (le type est déjà visible via l’icône). */
function slotTooltipText(slot: SlotDebug): string {
  const nm = slot.name.trim() || "—";
  if (!debugRoutingMode || (!slot.gridX && !slot.gridY)) return nm;
  return `${nm}\nGrille x:${slot.gridX ?? "—"} y:${slot.gridY ?? "—"}`;
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

function makeNode(slot: SlotDebug): HTMLElement {
  const item = document.createElement("div");
  item.className =
    "node node--hx-slot" + (normalizeCategory(slot.category) === "routing" ? " routing" : "");

  const tip = slotTooltipText(slot);
  item.setAttribute("aria-label", tip.replace(/\n/g, " — "));

  const iconWrap = document.createElement("div");
  iconWrap.className = "node-icon-wrap";
  iconWrap.title = tip;
  const iconPath = iconForCategory(slot.category, slot.name);
  if (iconPath) {
    const img = document.createElement("img");
    img.className = "node-icon-img";
    img.src = iconPath;
    img.alt = "";
    img.width = 20;
    img.height = 20;
    iconWrap.appendChild(img);
  } else {
    const ph = document.createElement("div");
    ph.className = "node-icon-fallback";
    ph.textContent = "?";
    iconWrap.appendChild(ph);
  }

  const abbr = document.createElement("div");
  abbr.className = "node-abbr";
  abbr.textContent = abbrevEffectType(slot.category, slot.name);

  item.appendChild(iconWrap);
  item.appendChild(abbr);

  if (debugRoutingMode && (slot.gridX || slot.gridY)) {
    const debug = document.createElement("div");
    debug.className = "node-debug-grid";
    debug.textContent = `x:${slot.gridX || "-"} y:${slot.gridY || "-"}`;
    item.appendChild(debug);
  }
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
  item.className = "node node-empty node--hx-slot";
  const iconWrap = document.createElement("div");
  iconWrap.className = "node-icon-wrap";
  const ph = document.createElement("div");
  ph.className = "node-icon-fallback node-icon-fallback--empty";
  ph.textContent = "—";
  ph.title = "Slot vide";
  iconWrap.appendChild(ph);
  const abbr = document.createElement("div");
  abbr.className = "node-abbr node-abbr--empty";
  abbr.textContent = "—";
  abbr.title = "Slot vide";
  item.appendChild(iconWrap);
  item.appendChild(abbr);
  return item;
}

/** Nœuds d'extrémité façon HX Edit (Input / Main L·R). */
function makeIoNode(label: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-io";
  el.textContent = label;
  return el;
}

/** Même carte que les slots (icône catégorie + libellé), pour split / merge dans la matrice. */
function makePathRoutingNode(kind: "split" | "merge", detailTitle: string): HTMLElement {
  const name = kind === "split" ? "Split" : "Merge";
  const item = makeNode({ category: "Routing", name });
  item.classList.add("hx-matrix-routing-node");
  item.setAttribute("aria-label", detailTitle.replace(/\n/g, " — "));
  const abbr = item.querySelector(".node-abbr");
  if (abbr) abbr.textContent = kind === "split" ? "split" : "merge";
  const iconWrap = item.querySelector(".node-icon-wrap");
  if (iconWrap) (iconWrap as HTMLElement).title = detailTitle;
  return item;
}

function gridSlotNode(slot: SlotDebug): HTMLElement {
  if (!slot.category && slot.name === "<empty>") return makeEmptySlotNode();
  return makeNode(slot);
}

/**
 * Grille 16 cases Kempline : matrice **2 lignes × 19 colonnes**.
 * Col 1 INPUT, col 2 split si `splitAfterCol === 0`, cols 3–17 modèles / routage alternés,
 * col 19 MAIN L/R ; ligne 2 = branche B si des blocs y sont présents.
 * Positions split/merge : stomp si `kemplineGridOk`, sinon heuristique d’occupation (pas
 * seulement `get_active_preset_routing_markers`, souvent vide alors que la branche B existe).
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

  const NUM_ROWS = 2;
  const NUM_COLS = 19;

  function wrapCell(row: number, col: number, inner: HTMLElement | null): HTMLElement {
    const w = document.createElement("div");
    w.className = "hx-matrix-cell" + (inner ? "" : " hx-matrix-cell--empty");
    w.style.gridRow = String(row);
    w.style.gridColumn = String(col);
    if (inner) w.appendChild(inner);
    return w;
  }

  function routingAtBoundary(boundary: number): HTMLElement | null {
    if (!showRoutingUi) return null;
    const { splitCol, mergeCol } = routingCols;
    if (boundary < 1 || boundary > 8) return null;
    if (mergeCol === boundary) return makePathRoutingNode("merge", mergeTip);
    if (splitCol === boundary) return makePathRoutingNode("split", splitTip);
    return null;
  }

  for (let row = 1; row <= NUM_ROWS; row += 1) {
    for (let col = 1; col <= NUM_COLS; col += 1) {
      let inner: HTMLElement | null = null;

      if (col === 1) {
        if (row === 1) inner = makeIoNode("INPUT");
      } else if (col === 2) {
        if (row === 1 && showRoutingUi && routingCols.splitCol === 0) {
          inner = makePathRoutingNode("split", splitTip);
        }
      } else if (col === 19) {
        if (row === 1) inner = makeIoNode("MAIN L/R");
      } else if (col >= 3 && col <= 17 && (col - 3) % 2 === 0) {
        const i = (col - 3) / 2;
        if (i >= 0 && i <= 7) {
          if (row === 1) inner = gridSlotNode(slots[i]!);
          else if (row === 2 && showRoutingUi && hasBranchB) inner = gridSlotNode(slots[8 + i]!);
        }
      } else if (col >= 4 && col <= 18 && (col - 4) % 2 === 0) {
        const j = (col - 4) / 2;
        if (row === 1 && j >= 0 && j <= 7) inner = routingAtBoundary(j + 1);
      }

      grid.appendChild(wrapCell(row, col, inner));
    }
  }

  root.appendChild(grid);

  contentEl.innerHTML = "";
  contentEl.appendChild(root);
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

function renderSlots(
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

  contentEl.innerHTML = "";
  contentEl.appendChild(root);
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
          ? (slots as [string, string, string, string][]).map(([category, name, gridX, gridY]) => ({
              category,
              name,
              gridX,
              gridY,
            }))
          : (slots as [string, string][]).map(([category, name]) => ({ category, name }));
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
          renderSlots(normalizedSlots, routingFlow, stompLayout);
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
