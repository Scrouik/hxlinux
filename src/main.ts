import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";

// ─── State ───────────────────────────────────────────────────────────────────

let presetNames: string[] = [];
let activePreset = -1;
let selectedIndex = -1;
let ctxTargetIndex = -1;
let dragSrcIndex = -1;

// ─── DOM refs ─────────────────────────────────────────────────────────────────

const list        = document.getElementById("preset-list")!;
const ctxMenu     = document.getElementById("ctx-menu")!;
const ctxRename   = document.getElementById("ctx-rename")!;
const ctxSave     = document.getElementById("ctx-save")!;
const ctxLoad     = document.getElementById("ctx-load")!;
const statusDot   = document.getElementById("status-dot")!;
const statusText  = document.getElementById("status-text")!;
const barActive   = document.getElementById("bar-active")!;
const barHint     = document.getElementById("bar-hint")!;
const presetCount = document.getElementById("preset-count")!;
const appRoot     = document.querySelector(".app") as HTMLElement;

// ─── Helpers ──────────────────────────────────────────────────────────────────

function setStatus(state: "waiting" | "loading" | "connected", text: string) {
  statusDot.className = "status-dot";
  if (state === "connected") statusDot.classList.add("connected");
  if (state === "loading")   statusDot.classList.add("loading");
  statusText.textContent = text;
}

function padNum(n: number): string {
  return String(n).padStart(3, "0");
}

function isEmpty(name: string): boolean {
  return !name || name === "<empty>";
}

function computeLongestPresetWidth(names: string[]): number {
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  if (!ctx) return 0;
  ctx.font = "500 13px Barlow";

  let longest = 0;
  names.forEach((name) => {
    const displayName = isEmpty(name) ? "empty" : name;
    longest = Math.max(longest, ctx.measureText(displayName).width);
  });

  return Math.ceil(longest);
}

function updateAppWidth(names: string[]) {
  // Mode fenêtre unique: la largeur est gérée par le layout split (list + models).
  if (document.querySelector(".models-pane")) {
    appRoot.style.width = "";
    return;
  }
  // Liens fixes d'une ligne : paddings + handle + numéro + gaps + marge.
  const listChromeWidth = 32 + 16 + 28 + 24 + 20;
  const longestPresetWidth = computeLongestPresetWidth(names);
  const listTargetWidth = listChromeWidth + longestPresetWidth;

  // Garde une largeur mini lisible et évite de dépasser la fenêtre.
  const minWidth = 280;
  const maxWidth = Math.max(minWidth, window.innerWidth - 24);
  const targetWidth = Math.min(maxWidth, Math.max(minWidth, listTargetWidth));

  appRoot.style.width = `${targetWidth}px`;
}

// ─── Render ───────────────────────────────────────────────────────────────────

function render(names: string[], active: number) {
  updateAppWidth(names);
  const container = document.getElementById("list-container")!;
  const scrollY = container.scrollTop;

  list.innerHTML = "";

  names.forEach((name, i) => {
    const li = document.createElement("li");
    li.className = "preset-item";
    li.dataset.index = String(i);

    if (i === active)        li.classList.add("active");
    if (i === selectedIndex) li.classList.add("selected");

    const handle = document.createElement("div");
    handle.className = "drag-handle";
    handle.innerHTML = "<span></span><span></span><span></span>";
    handle.title = "Drag to reorder";

    const num = document.createElement("span");
    num.className = "preset-num";
    num.textContent = padNum(i);

    const nameEl = document.createElement("span");
    nameEl.className = "preset-name" + (isEmpty(name) ? " empty" : "");
    nameEl.textContent = isEmpty(name) ? "empty" : name;

    li.appendChild(handle);
    li.appendChild(num);
    li.appendChild(nameEl);

    li.addEventListener("click",       ()  => onItemClick(i));
    li.addEventListener("contextmenu", (e) => onContextMenu(e, i));
    li.addEventListener("dblclick",    ()  => onItemDblClick(i));

    li.draggable = true;
    li.addEventListener("dragstart", (e) => onDragStart(e, i));
    li.addEventListener("dragover",  (e) => onDragOver(e, i));
    li.addEventListener("dragleave", ()  => onDragLeave(i));
    li.addEventListener("drop",      (e) => onDrop(e, i));
    li.addEventListener("dragend",   ()  => onDragEnd());

    list.appendChild(li);
  });

  container.scrollTop = scrollY;

  if (active >= 0 && active < names.length && !isEmpty(names[active])) {
    barActive.textContent = `▶  ${padNum(active)}  ${names[active]}`;
  } else {
    barActive.textContent = "— no preset —";
  }
  presetCount.textContent = `${names.filter(n => !isEmpty(n)).length} / ${names.length} presets`;
}

// ─── Load presets ─────────────────────────────────────────────────────────────

async function loadPresets() {
  try {
    const names = await invoke<string[]>("get_preset_names");

    if (names.length === 0) {
      setStatus("loading", "Loading...");
      return;
    }

    const active = await invoke<number>("get_active_preset");

    if (JSON.stringify(names) !== JSON.stringify(presetNames) || active !== activePreset) {
      presetNames = names;
      activePreset = active;
      render(presetNames, activePreset);
    }

    setStatus("connected", "HX Stomp XL");

    // Scroll vers le preset actif au premier chargement
    const activeEl = list.querySelector(".active") as HTMLElement | null;
    if (activeEl && activeEl.dataset.scrolled !== "1") {
      activeEl.dataset.scrolled = "1";
      activeEl.scrollIntoView({ block: "center", behavior: "smooth" });
    }

  } catch {
    setStatus("waiting", "En attente du HX...");
    presetNames = [];
    activePreset = -1;
    render([], -1);
  }
}

// ─── Item interactions ────────────────────────────────────────────────────────

async function onItemClick(index: number) {
  selectedIndex = index;
  hideContextMenu();
  try {
    console.log(`[PresetDebug][main] click preset index=${index}`);
    await invoke("activate_preset", { index });
    activePreset = index;
    await emit("models:load-preset", { index });
  } catch (e) {
    barHint.textContent = `Erreur activation : ${e}`;
    setTimeout(() => { barHint.textContent = "Right-click for options · Drag to reorder"; }, 2000);
  }
  render(presetNames, activePreset);
}

async function onItemDblClick(index: number) {
  selectedIndex = index;
  try {
    await invoke("activate_preset", { index });
    activePreset = index;
    render(presetNames, activePreset);
    barHint.textContent = `✓ Preset ${padNum(index)} activé`;
  } catch (e) {
    barHint.textContent = `Erreur activation : ${e}`;
  }
  setTimeout(() => { barHint.textContent = "Right-click for options · Drag to reorder"; }, 2000);
}

// ─── Context menu ─────────────────────────────────────────────────────────────

function onContextMenu(e: MouseEvent, index: number) {
  e.preventDefault();
  ctxTargetIndex = index;
  selectedIndex  = index;
  render(presetNames, activePreset);

  ctxLoad.classList.add("disabled");

  const x = Math.min(e.clientX, window.innerWidth  - 200);
  const y = Math.min(e.clientY, window.innerHeight - 120);
  ctxMenu.style.left = x + "px";
  ctxMenu.style.top  = y + "px";
  ctxMenu.classList.add("visible");
}

function hideContextMenu() {
  ctxMenu.classList.remove("visible");
  ctxTargetIndex = -1;
}

document.addEventListener("click", hideContextMenu);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    hideContextMenu();
    cancelRename();
  }
  if (e.key === "F2" && selectedIndex >= 0) {
    startRename(selectedIndex);
  }
});

// ─── Rename ───────────────────────────────────────────────────────────────────

let renameIndex = -1;

function startRename(index: number) {
  hideContextMenu();
  renameIndex = index;

  const items = list.querySelectorAll(".preset-item");
  const li = items[index] as HTMLElement;
  if (!li) return;

  const nameEl = li.querySelector(".preset-name") as HTMLElement;
  const currentName = isEmpty(presetNames[index]) ? "" : presetNames[index];

  const input = document.createElement("input");
  input.type = "text";
  input.className = "rename-input";
  input.value = currentName;
  // Limite UI alignée avec la contrainte produit: 16 caractères max.
  input.maxLength = 16;
  input.spellcheck = false;

  nameEl.replaceWith(input);
  input.focus();
  input.select();

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter")  confirmRename(input.value);
    if (e.key === "Escape") cancelRename();
    e.stopPropagation();
  });

  input.addEventListener("click", (e) => e.stopPropagation());
  input.addEventListener("mousedown", (e) => e.stopPropagation());

  input.addEventListener("blur", () => {
    setTimeout(() => {
      if (renameIndex >= 0) cancelRename();
    }, 100);
  });
}

async function confirmRename(newName: string) {
  const index = renameIndex;
  renameIndex = -1;

  if (!newName.trim()) {
    cancelRename();
    return;
  }

  const trimmed = newName.trim();

  // Mise à jour optimiste
  presetNames[index] = trimmed;
  render(presetNames, activePreset);

  try {
    await invoke("rename_preset", { index, name: trimmed });
    barHint.textContent = `✓  Preset ${padNum(index)} renommé`;
  } catch (e) {
    barHint.textContent = `✗  Erreur : ${e}`;
  }
  setTimeout(() => { barHint.textContent = "Right-click for options · Drag to reorder"; }, 3000);
}

function cancelRename() {
  if (renameIndex < 0) return;
  renameIndex = -1;
  render(presetNames, activePreset);
}

// ─── Save to disk ─────────────────────────────────────────────────────────────

async function savePreset(_index: number) {
  hideContextMenu();
  barHint.textContent = `Save to disk → à implémenter`;
  setTimeout(() => { barHint.textContent = "Right-click for options · Drag to reorder"; }, 3000);
  // TODO: invoke("save_preset_to_disk", { index })
}

// ─── Load from disk ───────────────────────────────────────────────────────────

async function loadPreset(_index: number) {
  hideContextMenu();
  barHint.textContent = `Load from disk → à implémenter`;
  setTimeout(() => { barHint.textContent = "Right-click for options · Drag to reorder"; }, 3000);
  // TODO: invoke("load_preset_from_disk", { index })
}

// ─── Context menu actions ─────────────────────────────────────────────────────

ctxRename.addEventListener("click", (e) => {
  e.stopPropagation();
  if (ctxTargetIndex >= 0) startRename(ctxTargetIndex);
});

ctxSave.addEventListener("click", (e) => {
  e.stopPropagation();
  if (ctxTargetIndex >= 0) savePreset(ctxTargetIndex);
});

ctxLoad.addEventListener("click", (e) => {
  e.stopPropagation();
  if (ctxTargetIndex >= 0 && !ctxLoad.classList.contains("disabled")) {
    loadPreset(ctxTargetIndex);
  }
});

// ─── Drag & Drop ──────────────────────────────────────────────────────────────

function getItem(index: number): HTMLElement | null {
  return list.querySelectorAll(".preset-item")[index] as HTMLElement || null;
}

function clearDragClasses() {
  list.querySelectorAll(".preset-item").forEach(el => {
    el.classList.remove("drag-over-top", "drag-over-bottom", "dragging");
  });
}

function onDragStart(e: DragEvent, index: number) {
  dragSrcIndex = index;
  e.dataTransfer!.effectAllowed = "move";
  e.dataTransfer!.setData("text/plain", String(index));

  const li = getItem(index);
  if (li) {
    li.classList.add("dragging");
    const ghost = li.cloneNode(true) as HTMLElement;
    ghost.style.cssText = `
      position:fixed; top:-999px; left:-999px;
      background: var(--bg-raised);
      border: 1px solid var(--amber);
      border-radius: 4px;
      padding: 4px 16px;
      opacity: 0.9;
      width: ${li.offsetWidth}px;
    `;
    document.body.appendChild(ghost);
    e.dataTransfer!.setDragImage(ghost, 16, 18);
    setTimeout(() => document.body.removeChild(ghost), 0);
  }
}

function onDragOver(e: DragEvent, index: number) {
  e.preventDefault();
  e.dataTransfer!.dropEffect = "move";

  if (dragSrcIndex === index) return;

  clearDragClasses();
  const li = getItem(dragSrcIndex);
  if (li) li.classList.add("dragging");

  const target = getItem(index);
  if (!target) return;

  const rect = target.getBoundingClientRect();
  const midY = rect.top + rect.height / 2;

  if (e.clientY < midY) {
    target.classList.add("drag-over-top");
  } else {
    target.classList.add("drag-over-bottom");
  }
}

function onDragLeave(index: number) {
  const target = getItem(index);
  if (target) target.classList.remove("drag-over-top", "drag-over-bottom");
}

function onDrop(e: DragEvent, targetIndex: number) {
  e.preventDefault();
  if (dragSrcIndex < 0 || dragSrcIndex === targetIndex) return;

  const target = getItem(targetIndex);
  const isTop  = target?.classList.contains("drag-over-top");

  clearDragClasses();

  const insertBefore = isTop ? targetIndex : targetIndex + 1;
  const actualInsert = insertBefore > dragSrcIndex ? insertBefore - 1 : insertBefore;

  const moved = presetNames.splice(dragSrcIndex, 1)[0];
  presetNames.splice(actualInsert, 0, moved);

  if (activePreset === dragSrcIndex) activePreset = actualInsert;
  if (selectedIndex === dragSrcIndex) selectedIndex = actualInsert;

  render(presetNames, activePreset);

  barHint.textContent = `Move preset ${padNum(dragSrcIndex)} → ${padNum(actualInsert)} (à envoyer au HX)`;
  setTimeout(() => { barHint.textContent = "Right-click for options · Drag to reorder"; }, 3000);

  // TODO: invoke("move_preset", { from: dragSrcIndex, to: actualInsert })

  dragSrcIndex = -1;
}

function onDragEnd() {
  clearDragClasses();
  dragSrcIndex = -1;
}

// ─── Init ─────────────────────────────────────────────────────────────────────

window.addEventListener("DOMContentLoaded", () => {
  setStatus("waiting", "En attente...");
  updateAppWidth([]);
  loadPresets();
  window.setInterval(loadPresets, 1500);
});

window.addEventListener("resize", () => {
  updateAppWidth(presetNames);
});