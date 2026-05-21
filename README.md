# HXLinux

**EN:** Open-source HX Stomp XL editor for Linux (Tauri — Rust + TypeScript).  
**FR:** Éditeur open source pour HX Stomp XL sous Linux (Tauri — Rust + TypeScript).

> **Development in progress · Développement toujours en cours.**

---

## Status · État du projet

### What works (HX Edit parity) · Ce qui fonctionne (aligné HX Edit)

**English**

- Native USB connection to HX Stomp XL on Linux (handshake, mode machine, keep-alive)
- Reading preset names (125 slots), active preset, rename from the UI
- Preset activation from the UI and hardware-driven sync (slot / model notifications)
- Signal-chain view: preset content load, blocks displayed from device data
- Hardware changes reflected when driven by the Stomp (preset, slot, model) — same class of behaviour as listening with HX Edit connected
- Real-time parameter editing where the USB path is implemented
- Model metadata and USB assign payloads from **`HX_ModelUsbAssign.json`** (see [Model data](#model-data--données-modèles) below)

**Français**

- Connexion USB native au HX Stomp XL sous Linux (handshake, machine à états, keep-alive)
- Lecture des noms de presets (125 emplacements), preset actif, renommage depuis l’UI
- Activation de preset depuis l’UI et synchro pilotée par le matériel (slot / modèle)
- Vue chaîne : chargement du contenu preset, blocs affichés à partir des données device
- Changements matériel pris en compte quand le Stomp pilote (preset, slot, modèle) — même famille de comportement qu’avec HX Edit en écoute
- Édition de paramètres en temps réel là où le chemin USB est implémenté
- Métadonnées et trames d’assign USB via **`HX_ModelUsbAssign.json`** (voir [Données modèles](#model-data--données-modèles) ci-dessous)

### UI model assignment · Assignation de modèles depuis l’UI

**English**

| Category | From UI |
|----------|---------|
| **Distortions** | Assignable |
| **Dynamics** | Assignable |
| **All other model families** | Under verification — **not available yet** for UI assignment |

Other blocks may **appear** in the UI if they were assigned on the **hardware** first; displaying them does not mean UI assignment is supported for that family.

**Français**

| Famille | Depuis l’UI |
|---------|-------------|
| **Distortions** | Assignable |
| **Dynamics** | Assignable |
| **Toutes les autres familles** | En vérification — **pas encore disponibles** pour assignation UI |

D’autres blocs peuvent **s’afficher** s’ils ont été assignés sur le **matériel** avant ; l’affichage ne signifie pas que l’assignation UI est prête pour cette famille.

### Not available yet · Pas encore disponible

**English**

- Preset **save** to device / disk (HXLinux workflow)
- **Footswitch / push button** assignment
- Preset **LED colorization**
- **Snapshots**

**Français**

- **Sauvegarde** de preset vers le device / disque (workflow HXLinux)
- Assignation des **boutons poussoirs** (footswitch)
- **Colorisation** des presets (LED)
- **Snapshots**

### Roadmap (short) · Suite prévue (bref)

**English:** Broader UI model assignment, preset persistence, footswitches, colors, snapshots, export/import.  
**Français :** Élargir l’assignation UI par famille, persistance preset, footswitches, couleurs, snapshots, export/import.

---

## Model data · Données modèles

**English**

| File | Role |
|------|------|
| **`src-tauri/resources/HX_ModelUsbAssign.json`** | **Reference file** — per-model USB assign (`bulkHex`), picker labels, and fields we align with HX Edit over time |
| **`src-tauri/resources/HX_ModelCatalog.json`** | **Deprecated for HXLinux** — legacy Line 6 export; **do not extend**; new work goes only into `HX_ModelUsbAssign.json` |

Parameter definitions remain in bundled **`models/*.models`** files (`.models` JSON per family).

**Français**

| Fichier | Rôle |
|---------|------|
| **`src-tauri/resources/HX_ModelUsbAssign.json`** | **Fichier de référence** — assign USB par modèle (`bulkHex`), libellés picker, champs alignés HX Edit au fil du temps |
| **`src-tauri/resources/HX_ModelCatalog.json`** | **Déprécié pour HXLinux** — export Line 6 historique ; **ne plus enrichir** ; tout le nouveau travail va dans `HX_ModelUsbAssign.json` |

Les définitions de paramètres restent dans les fichiers **`models/*.models`** bundlés (JSON `.models` par famille).

---

## Tech stack · Stack technique

- **Backend:** Rust, Tauri 2, `rusb`
- **Frontend:** TypeScript, Vite (main UI + models pane)
- **Protocol:** USB reverse engineering (captures HX Edit / HXLinux — see `docs/`)

---

## Requirements · Prérequis

- Linux (Ubuntu/Debian family tested)
- Line 6 **HX Stomp XL** on USB
- Line 6 **HX Edit** on the machine is useful for captures and comparison; **model assign data** ships in-repo as **`HX_ModelUsbAssign.json`** (not `HX_ModelCatalog.json`)

---

## Run · Lancement

```bash
# Frontend dev server only
npm run dev

# Full desktop app (frontend + Tauri backend)
npm run tauri dev
```

---

## Build · Compilation

```bash
npm run build
npm run tauri build
```

---

## Project notes · Notes projet

- Rust mode state machine for USB protocol phases
- Async USB (listener / writer threads + channels)
- Protocol reference: [`docs/Référence protocole USB HX Stomp XL.md`](docs/Référence%20protocole%20USB%20HX%20Stomp%20XL.md)

---

## Important — after closing the app · Après fermeture

**English**

When you quit HXLinux while the Stomp XL is connected, the device may stay in a **degraded or stuck editor USB state**. HX Edit on Windows likely uses the **Line 6 proprietary driver** to release the session with traffic we have **not yet captured** on Linux.

**Workaround:** after closing HXLinux, **power off the Stomp XL**, then turn it back on before the next session (USB replug alone is less reliable).

Details: protocol doc §12.3. The UI shows an amber banner while connected and a confirmation on window close.

**Français**

En quittant HXLinux alors que le Stomp XL est connecté, le boîtier peut rester en **mode dégradé ou bloqué** (session USB éditeur non libérée). Sous Windows, HX Edit s’appuie sans doute sur le **driver propriétaire Line 6** pour envoyer une libération que nous n’avons **pas encore reproduite** sous Linux.

**Contournement :** après fermeture de HXLinux, **éteindre le Stomp XL** (alimentation), puis le rallumer avant la prochaine session (rebrancher l’USB seul est moins fiable).

Détails : doc protocole §12.3. Bandeau ambre à la connexion + confirmation à la fermeture de fenêtre.

---

## Credits · Crédits

USB protocol reverse engineering inspired by [kempline/helix_usb](https://github.com/kempline/helix_usb).
