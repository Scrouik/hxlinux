import { invoke } from "@tauri-apps/api/core";

let isLoading = false;

async function checkAndLoad() {
  console.log(`checkAndLoad appelé — isLoading=${isLoading}`);
  if (isLoading) {
    console.log("Chargement en cours — on skip");
    return;
  }

  isLoading = true;  // ← ICI en premier !
  
  const statusEl = document.querySelector("#status");
  if (!statusEl) { isLoading = false; return; }

  try {
    statusEl.textContent = "Recherche du HX Stomp XL...";
    const connected = await invoke<boolean>("check_device");
    console.log(`HX détecté: ${connected}`);
    
    if (!connected) {
      isLoading = false;
      statusEl.textContent = "HX Stomp XL non détecté — vérifiez la connexion USB";
      setTimeout(checkAndLoad, 3000);
      return;
    }

    statusEl.textContent = "Connexion au HX Stomp XL...";
    const presets = await invoke<string[]>("get_preset_names");
    isLoading = false;
    console.log(`isLoading = false — ${presets.length} presets reçus`);
    
    if (presets.length === 0) {
      statusEl.textContent = "HX Stomp XL en cours d'initialisation...";
      setTimeout(checkAndLoad, 2000);
      return;
    }

    statusEl.textContent = `${presets.length} presets trouvés`;
    const listEl = document.querySelector("#preset-list");
    if (listEl) {
      listEl.innerHTML = presets
        .map((name, i) => `<li><span class="preset-num">${String(i).padStart(3, '0')}</span> ${name}</li>`)
        .join("");
    }

  } catch (error) {
    isLoading = false;
    console.log(`Erreur: ${error}`);
    const errMsg = String(error);
    if (errMsg.includes("Resource busy")) {
      statusEl.textContent = "Reconnexion en cours...";
      setTimeout(checkAndLoad, 1000);
    } else {
      statusEl.textContent = "HX Stomp XL en cours d'initialisation...";
      setTimeout(checkAndLoad, 2000);
    }
  }
}

window.addEventListener("DOMContentLoaded", () => {
  document.querySelector("#btn-refresh")
    ?.addEventListener("click", () => {
      const listEl = document.querySelector("#preset-list");
      const statusEl = document.querySelector("#status");
      if (listEl) listEl.innerHTML = "";
      if (statusEl) statusEl.textContent = "Actualisation...";
      setTimeout(checkAndLoad, 1000);
    });
  checkAndLoad();
});
