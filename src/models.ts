import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import "./styles.css";

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

/** Infobulle : nom du modèle uniquement. */
function slotTooltipText(slot: SlotDebug): string {
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

function gridSlotNode(slot: SlotDebug): HTMLElement {
  if (!slot.category && slot.name === "<empty>") return makeEmptySlotNode();
  /* Matrice : sur « Path 1 » / « Path 2 », la catégorie est sur la ligne Description ; la cellule slot = icône + infobulle nom. */
  return makeNode(slot, { showTypeAbbrev: false });
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
          if (row === LINE_PATH_1) inner = gridSlotNode(slots[i]!);
          else if (row === LINE_DESC_PATH_1) inner = makeMatrixCategoryCell(slots[i]!);
          else if (row === LINE_PATH_2 && showRoutingUi && hasBranchB) inner = gridSlotNode(slots[8 + i]!);
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
