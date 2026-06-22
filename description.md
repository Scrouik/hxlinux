# HXLinux — guide de reprise de session

Ce fichier est le **mémo technique** pour reprendre le développement sans l’historique de chat. Il ne remplace pas le README (install, statut produit) ni le backlog (**[`TODO.md`](TODO.md)**).

| Document | Contenu |
|----------|---------|
| [`README.md`](README.md) | Présentation, prérequis, commandes de base |
| [`TODO.md`](TODO.md) | Backlog priorisé (scroll, bulkHex, LT, DSP, UI grille…) |
| [`docs/models-hardware-sync.md`](docs/models-hardware-sync.md) | Soft-sync UI, flags `localStorage`, events USB |
| [`docs/matrix-edit-handoff.md`](docs/matrix-edit-handoff.md) | Matrice : copier/coller, DnD Pointer Events, cache session, bugs |
| [`docs/Scroll_model_pull_handoff.md`](docs/Scroll_model_pull_handoff.md) | Scroll modèle HW (pull `1b`/`19`, lanes, pièges) |
| [`docs/preset_bootstrap_analysis_traps.md`](docs/preset_bootstrap_analysis_traps.md) | Bootstrap preset / phase 4 |
| [`captures/usb-wireshark/README.md`](captures/usb-wireshark/README.md) | Workflow captures → `bulkHex` |

---

## État actuel (juin 2026)

### Validé terrain — HX Stomp XL

- Connexion USB, liste / activation / renommage presets, lecture dump preset.
- Grille 16 + matrice stomp (`renderGrid16`), panneau paramètres (`.models` + valeurs chaîne).
- **Cache session params** : `preset_data` n’est lu qu’**au chargement / changement de preset** ; en session, valeurs via `slotChainSessionByKey` + overrides live write — voir [`docs/matrix-edit-handoff.md`](docs/matrix-edit-handoff.md).
- Sync slot actif hardware → UI (`models:hardware-slot-changed`), soft-sync sans re-parse grille entre deux dumps.
- **Scroll modèle FX** (~92 %) : pull USB sans `request_preset_content` à chaque cran — voir handoff scroll.
- **Live write paramètres** : `write_live_param` (float / bool / discret).
- **Assign FX** via picker + `bulkHex` dans [`HX_ModelUsbAssign.json`](src-tauri/resources/HX_ModelUsbAssign.json) (campagne en cours, voir TODO).
- **Path 1 Input** : `ioSources[]`, live write `1d`, scroll molette → picker (`write_path1_input_source`, event `models:path1-input-source-changed`).
- **Path 1 Split** : `splitSources[]`, live write `25`, scroll molette → picker (`write_path1_split_type`, event `models:path1-split-type-changed`). **Piège Line 6** : encodage Y/A/B **inversé** entre select HX Edit (`0=Y`, `1=A/B`) et scroll hardware Stomp (`1=Y`, `0=A/B`) — normalisé dans `path1_split_live_write.rs`.

### Partiel / à faire

- **Path 1 Output / Merge** : picker verrouillé + focus USB ; pas encore live write / scroll HW (LT à identifier — voir TODO § Path 1 structurel).
- **Cab dual IR** (onglets Cab 1 / Cab 2) : **Cab 1 = picker libre** ; **Cab 2 = picker verrouillé** Cab / Single IR (affichage). Surbrillance Cab 2 = id **single** (`dualTabPanes[1].catalogModelId`). **USB replace cab2** : entrée assign `dual` / WithPan (hint `c319`), via `resolveCabDualCab2UsbWireFromPicker` + `build_cab_dual_replace_cab_bulk` (`variant=dual`). Implémentation : `syncPickerForCabDualTab` dans `models.ts`. **Doc :** [Cab_dual_fonctionnement_no_legacy.md](docs/Cab_dual_fonctionnement_no_legacy.md) · legacy : [Cab_dual_fonctionnement_legacy.md](docs/Cab_dual_fonctionnement_legacy.md).
- **Amp+Cab IR** (onglets Amp / Cab) : un bulk `c319` + `<amp> 1a <cab>` ; `dualPart` `amp`/`cab`, index param **local** ; focus cab IR = `1d` `cd:03` `1a:01` ; live write cab `pp=03`. **Doc :** [Amp_cab_fonctionnement_no_legacy.md](docs/Amp_cab_fonctionnement_no_legacy.md).
- **Amp+Cab Legacy** (`amp+cab-legacy`) : focus cab `1b`, params cab `pp=08`, tables sélecteurs guitar/compact. **Doc :** [Amp_cab_fonctionnement_legacy.md](docs/Amp_cab_fonctionnement_legacy.md).
- **Helix LT** : topologie 4 paths / 2 DSP, 32 segments — non implémenté (TODO § grille device).
- **Matrice — copier/coller, déplacer (v1)** : même path, Pointer Events, cache session — handoff [`docs/matrix-edit-handoff.md`](docs/matrix-edit-handoff.md) ; bugs et suite dans TODO § Matrice.
- **Budget DSP** (`load` dans `.models`) — non calculé côté app.

Le hardware Line 6 se comporte de façon cohérente ; les écarts viennent surtout du **protocole** (plusieurs conventions selon contexte select vs scroll).

---

## Conventions à ne pas confondre

### Index slot effet (grille Kempline)

| Langage utilisateur | Index UI `data-kempline-slot-index` | `slot_bus` USB (`82:62:XX:1a`) |
|---------------------|-------------------------------------|--------------------------------|
| Slot 1 (1er effet path 1) | **0** | **`0x01`** |
| Slot 2 | **1** | **`0x02`** |
| … | … | … |
| Slot 8 (path 2) | **8** | **`0x0b`** |

Les captures nommées `Slot0_…` = **index 0**.

### Blocs structurels Path 1 (I/O et routage)

| Bloc | `slot_bus` | Live write / scroll (Stomp XL) |
|------|------------|--------------------------------|
| Input | `0x00` | ✅ scroll + write |
| Output | `0x09` | picker seulement |
| Split | `0x0a` | ✅ scroll + write |
| Merge | `0x13` | picker seulement |

Focus USB : `switch_active_hardware_special_slot` / trames `82:62:SS:1a`.

### Deux sens « modèle » sur USB

| Sens | Mécanisme | Fichier config |
|------|-----------|----------------|
| **Assign FX** (changer un bloc dans un slot) | Paquet `bulkHex` capturé HX Edit | `entries[]` dans `HX_ModelUsbAssign.json` |
| **Scroll FX** (molette sur slot actif) | Pull `1b`/`19` → `module_hex` / chainHex | `scroll_model_pull.rs` + hints catalogue |
| **I/O / Split type** | Trames dédiées (`1d`, `25`, …) | `ioSources[]`, `splitSources[]` |

Ne pas confondre trames **pull scroll** (`1b:00` 36 o) et **bulk assign**.

---

## Architecture

| Couche | Rôle |
|--------|------|
| **Rust / Tauri 2** | USB (`rusb`), `HelixState`, modes (`helix/modes/`), listener IN → events |
| **TypeScript** | `main.ts` (presets), `models.ts` (grille + params + picker) |
| **Ressources** | Catalogue, `.models`, `HX_ModelUsbAssign.json`, icônes |

```
hxlinux/
├── src/main.ts, models.ts, styles.css
├── src/hxModelCatalogMeta.ts     # Picker, catalogue, ioSources / splitSources
├── src-tauri/src/lib.rs         # Commandes Tauri, parse preset, grille Kempline
├── src-tauri/src/preset_chain_params.rs
├── src-tauri/src/stomp_layout.rs
└── src-tauri/src/helix/
    ├── usb_listener.rs          # IN 0x81 → modes + events UI
    ├── scroll_model_pull.rs     # Scroll modèle FX (ex slot_model_hw_pull)
    ├── path1_io_live_write.rs   # Input Path 1
    ├── path1_split_live_write.rs
    ├── live_write.rs            # Paramètres slot FX
    ├── slot_param_in.rs         # Echo param IN → live write
    └── modes/                   # Connect, RequestPreset, …
```

Build : `npm run tauri dev` · Tests Rust utiles : `cd src-tauri && cargo test path1_split path1_input scroll_model`

---

## Events Tauri (`models:*`)

| Event | Quand |
|-------|--------|
| `models:hardware-slot-changed` | Changement slot actif (`82:62:…:1a`) |
| `models:slot-model-changed` | Scroll / changement modèle FX (pull) |
| `models:slot-param-changed` | Twist knob paramètre |
| `models:slot-content-changed` | Capsule focus slot (watch) |
| `models:path1-input-source-changed` | Scroll / echo source Input |
| `models:path1-split-type-changed` | Scroll / echo type Split |

Détail sync front : [`docs/models-hardware-sync.md`](docs/models-hardware-sync.md).

---

## Commandes `invoke` (principales)

Déclarées dans `lib.rs` — liste non exhaustive.

| Commande | Rôle |
|----------|------|
| `get_preset_names`, `get_active_preset`, `activate_preset`, `rename_preset` | Presets |
| `request_preset_content` | Dump preset actif |
| `get_active_preset_slots`, `get_active_preset_stomp_layout` | Grille / routing |
| `get_active_preset_slot_chain_param_values` | Valeurs chaîne slot 0..15 — **hydratation load preset uniquement** (cache session côté TS) |
| `get_active_preset_path1_io_chain_param_values` | Params Input/Output Path 1 |
| `get_active_preset_kempline_flow_chain_param_values` | Params Split/Merge (flow) |
| `get_active_hardware_slot_state` | Slot actif côté backend |
| `switch_active_hardware_slot` | Focus slot FX 0..15 |
| `switch_active_hardware_special_slot` | Focus Input/Output/Split/Merge |
| `sync_hardware_slot_focus_usb` | Capsule focus + content watch |
| `write_live_param` | Écriture param FX |
| `write_path1_input_source`, `write_path1_split_type` | I/O Path 1 |
| `get_path1_input_source_wire_value`, `get_path1_split_type_wire_value` | Wire mémorisé IN USB |
| `probe_slot_model_usb` | Assign / remove FX (picker) |

Flux typique models : `request_preset_content` → `get_active_preset_slots` + layout → **`hydrateSlotChainSessionFromPresetData`** → clic slot → `resolveChainValuesForKemplineSlot` + `.models`.

---

## Matrice — édition preset

Handoff détaillé (problèmes, solutions, architecture, bugs) : **[`docs/matrix-edit-handoff.md`](docs/matrix-edit-handoff.md)**.

Backlog et cases à cocher : **[`TODO.md`](TODO.md)** § Matrice.

---

## Panneau paramètres (rappel court)

- **Valeurs au load** : hydratation unique depuis `preset_data` → `slotChainSessionByKey` (`preset_chain_params.rs`).
- **Valeurs en session** : `resolveChainValuesForKemplineSlot` (cache + overrides live) ; assign picker / probe utilise défauts `.models` puis live write.
- **Schéma UI** : `.models` + ordre/filtre via `HX_ModelCatalog.json` (`hxModelCatalogMeta.ts`).
- **Formatage** : `HelixControls.json` + `formatChainParamValue` dans `models.ts`.
- **Alignement** : `alignChainValuesToModelParamOrder` (mono / stéréo, `assign`, Amp+Cab).

Ne **pas** relire `get_active_preset_slot_chain_param_values` au clic slot ou après probe/move — mettre à jour le cache session à la place.

Split A/B vs Y : échelle fil **0…1** vs affichage Helix **−100…+100** — voir commentaires dans `models.ts` si régression d’échelle.

---

## Picker slot (`HX_ModelUsbAssign.json`)

- **`entries[]`** : FX assignables (`bulkHex`).
- **`ioSources[]`** / **`splitSources[]`** : I/O et types Split (live write, pas bulkHex).
- **`pickerExcludedCategories`** : ex. `"Split"` exclu des `entries[]` FX mais **`splitSources[]` toujours enregistrés** pour le picker.
- Script sync : `scripts/sync_usb_assign_from_catalog.py` · Injection bulk : `scripts/inject_bulk_from_captures.py`.

Picker verrouillé : Input, Output, Split, Merge — catégorie figée quand le bus structurel correspond (`models.ts`, `slotPickerIoLock`).

---

## Flags debug utiles

Dans la console de la fenêtre **Models** (`localStorage`) :

| Clé | Effet |
|-----|--------|
| `models_debug_sync_trace` | Logs `[ModelsSync]` |
| `models_debug_hw_slot_sync` | Slot actif / focus USB |
| `models_debug_hw_model_fast` | Timing scroll modèle (aperçu vs settle) |
| `models_hw_usb_preset_poll_ms` | Re-dump preset périodique (ex. `2500`) |
| `models_hw_force_preset_dump_on_slot_notify` | Dump à chaque changement slot (secours) |

Côté terminal Rust :

| Env | Effet |
|-----|--------|
| `HX_SCROLL_CHAINHEX=1` | Trace chainHex à chaque pull scroll |
| `HX_SCROLL_PULL_DEBUG=1` | Trace protocole pull |
| `RUST_LOG` / `set_preset_debug_verbose` | Verbose preset |

---

## Reprise rapide après redémarrage

1. Brancher le Stomp XL · `npm run tauri dev` depuis la racine.
2. Vérifier connexion + chargement preset dans la fenêtre Models.
3. Backlog du jour : **[`TODO.md`](TODO.md)** (ne pas dupliquer ici).
4. Protocole scroll / lanes : **[`docs/Scroll_model_pull_handoff.md`](docs/Scroll_model_pull_handoff.md)**.
5. Captures USB : `captures/usb-wireshark/` (+ README workflow).

**Note Kempline** : le dépôt s’inspire du reverse [helix_usb / Kempline](https://github.com/kempline/helix_usb) ; le code actuel n’est **pas** une traduction ligne à ligne — comparer avec les analyses Kempline sans contexte HXLinux prête à confusion (cf. TODO § refactor nommage).

---

## Historique détaillé

Les journaux de session (avril–mai 2026 : Phase C, keep-alive, Amp+Cab, lane scroll init, etc.) ne sont **plus maintenus dans ce fichier** pour éviter la dérive. Ils restent accessibles via **l’historique git** de `description.md` et dans **`docs/`** (handoffs, addendums scroll, bootstrap).
