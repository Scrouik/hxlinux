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

function isAmpCategory(category: string): boolean {
  const c = normalizeCategory(category);
  return c === "amp" || c === "preamp" || c === "amp+cab";
}

function makeNode(slot: SlotDebug): HTMLElement {
  const item = document.createElement("div");
  item.className = "node" + (normalizeCategory(slot.category) === "routing" ? " routing" : "");

  const title = document.createElement("div");
  title.className = "node-title";
  const iconPath = iconForCategory(slot.category, slot.name);
  if (iconPath) {
    const img = document.createElement("img");
    img.src = iconPath;
    img.alt = "";
    title.appendChild(img);
  }
  const titleText = document.createElement("span");
  titleText.textContent = slot.category.toUpperCase();
  title.appendChild(titleText);

  const name = document.createElement("div");
  name.className = "node-name";
  name.textContent = slot.name;

  item.appendChild(title);
  item.appendChild(name);
  if (debugRoutingMode && (slot.gridX || slot.gridY)) {
    const debug = document.createElement("div");
    debug.className = "node-name";
    debug.style.opacity = "0.7";
    debug.style.fontSize = "11px";
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
  item.className = "node node-empty";
  const title = document.createElement("div");
  title.className = "node-title";
  title.textContent = "—";
  const name = document.createElement("div");
  name.className = "node-name";
  name.textContent = "vide";
  item.appendChild(title);
  item.appendChild(name);
  return item;
}

/** Nœuds d'extrémité façon HX Edit (Input / Main L·R). */
function makeIoNode(label: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-io";
  el.textContent = label;
  return el;
}

/** Espace réservé même largeur qu'un nœud I/O pour aligner la rangée du bas. */
function makeIoSpacer(): HTMLElement {
  const el = document.createElement("div");
  el.className = "hx-io hx-io-spacer";
  el.setAttribute("aria-hidden", "true");
  return el;
}

/** Split / Merge entre les deux paths (colonne alignée sur les effets, pas sur INPUT). */
function renderRoutingJunction(routing: [string, string][]) {
  const wrap = document.createElement("div");
  wrap.className = "hx-routing-junction";
  const gutter = document.createElement("div");
  gutter.className = "hx-routing-gutter";
  const body = document.createElement("div");
  body.className = "hx-routing-junction-body";
  const rail = document.createElement("div");
  rail.className = "hx-routing-rail";
  const inner = document.createElement("div");
  inner.className = "hx-routing-junction-inner";
  for (const [, name] of routing) {
    const n = document.createElement("div");
    n.className = "routing-chip hx-routing-node";
    n.textContent = name;
    inner.appendChild(n);
  }
  body.appendChild(rail);
  body.appendChild(inner);
  wrap.appendChild(gutter);
  wrap.appendChild(body);
  return wrap;
}

function appendSlotCell(row: HTMLElement, slot: SlotDebug) {
  if (row.childElementCount > 0) appendPipe(row);
  if (!slot.category && slot.name === "<empty>") {
    row.appendChild(makeEmptySlotNode());
  } else {
    row.appendChild(makeNode(slot));
  }
}

/**
 * Grille 16 cases : présentation inspirée de HX Edit (flux gauche → droite,
 * Path 1A / 1B, I/O, zone Split–Merge entre les rangées).
 */
function renderGrid16(slots: SlotDebug[], routing: [string, string][]) {
  const root = document.createElement("div");
  root.className = "flow grid16 hx-edit-chain";

  const mkPathBlock = (
    pathLabel: string,
    slice: SlotDebug[],
    opts: { lead: HTMLElement | null; tail: HTMLElement | null },
  ) => {
    const wrap = document.createElement("div");
    wrap.className = "grid16-row-wrap hx-path-block";
    const lab = document.createElement("div");
    lab.className = "grid16-branch-label";
    lab.textContent = pathLabel;
    const row = document.createElement("div");
    row.className = "flow-row grid16-row hx-path-row";
    if (opts.lead) {
      row.appendChild(opts.lead);
      appendPipe(row);
    }
    for (const slot of slice) {
      appendSlotCell(row, slot);
    }
    if (opts.tail) {
      appendPipe(row);
      row.appendChild(opts.tail);
    }
    wrap.appendChild(lab);
    wrap.appendChild(row);
    return wrap;
  };

  root.appendChild(
    mkPathBlock("Path 1A — slots 1 à 8", slots.slice(0, 8), {
      lead: makeIoNode("INPUT"),
      tail: null,
    }),
  );

  if (routing.length > 0) {
    root.appendChild(renderRoutingJunction(routing));
  }

  root.appendChild(
    mkPathBlock("Path 1B — slots 11 à 18", slots.slice(8, 16), {
      lead: makeIoSpacer(),
      tail: makeIoNode("MAIN L/R"),
    }),
  );

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

function renderSlots(rawSlots: SlotDebug[], routingFromFlow: [string, string][] = []) {
  if (rawSlots.length === 0) {
    renderEmpty("Aucun bloc detecte dans ce preset.");
    return;
  }

  const slots: SlotDebug[] = rawSlots;
  if (isKemplineGrid16(slots)) {
    renderGrid16(slots, routingFromFlow);
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
          if (isKemplineGrid16(normalizedSlots)) {
            try {
              const r = await invoke<[string, string][] | null>("get_active_preset_routing_markers");
              routingFlow = r ?? [];
            } catch {
              console.warn("[PresetDebug][models] get_active_preset_routing_markers error");
            }
          }
          renderSlots(normalizedSlots, routingFlow);
          setStatus(
            debugRoutingMode
              ? `${normalizedSlots.length} blocs detectes (debug routing ON)`
              : `${normalizedSlots.length} blocs detectes`,
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
