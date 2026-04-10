import { invoke } from "@tauri-apps/api/core";

async function loadPresets() {
  const statusEl = document.querySelector("#status");
  const listEl = document.querySelector("#preset-list");
  
  if (!statusEl || !listEl) {
    console.log("Elements non trouvés !");
    return;
  }
  
  statusEl.textContent = "Connexion au HX Stomp XL...";
  
  try {
    const presets = await invoke<string[]>("get_preset_names");
    statusEl.textContent = `${presets.length} presets trouvés`;
    listEl.innerHTML = presets
      .map((name, i) => `<li><span class="preset-num">${String(i).padStart(3, '0')}</span> ${name}</li>`)
      .join("");
  } catch (error) {
    statusEl.textContent = `Erreur: ${error}`;
    alert(`Erreur détaillée: ${error}`);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  document.querySelector("#btn-refresh")
    ?.addEventListener("click", loadPresets);
});