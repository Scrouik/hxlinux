import { invoke } from "@tauri-apps/api/core";

let activePreset = -1;

async function loadPresets() {
  const statusEl = document.querySelector("#status");
  const listEl   = document.querySelector("#preset-list");

  if (!statusEl || !listEl) return;

  try {
    statusEl.textContent = "Connexion au HX Stomp XL...";

    const names = await invoke<string[]>("get_preset_names");

    if (names.length === 0) {
      statusEl.textContent = "Chargement en cours...";
      setTimeout(loadPresets, 1000);
      return;
    }

    activePreset = await invoke<number>("get_active_preset");

    statusEl.textContent = `HX Stomp XL — preset actif : ${activePreset}`;

    listEl.innerHTML = names
      .map((name, i) => `
        <li class="${i === activePreset ? 'active' : ''}">
          <span class="preset-num">${String(i).padStart(3, '0')}</span>
          <span class="preset-name">${name}</span>
        </li>
      `)
      .join("");

    // Scroller vers le preset actif
    const activeEl = listEl.querySelector(".active");
    if (activeEl) activeEl.scrollIntoView({ block: "center" });

  } catch (e) {
    statusEl.textContent = "En attente du HX...";
    setTimeout(loadPresets, 1000);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  loadPresets();
  // Rafraîchir toutes les 2 secondes
  setInterval(loadPresets, 2000);
});