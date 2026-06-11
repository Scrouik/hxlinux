# TODO HXLinux

## Réalisé (mai–juin 2026)

- [x] **Scroll modèle HW (~92 %)** — pull `1b`/`19`, lanes scroll vs preset, settling UI ; reste ~8 % cas limites (voir docs scroll handoff). Analyse commune sur plusieurs nuits — plafond raisonnable, pas de patch Line 6 à attendre.
- [x] **Bootstrap preset / lecture one-shot** — ACK dump sur `editor_ed03_lane` ; trailer phase 4 reconnu structurellement (`sub=0x04`, `17≤len<272`). Doc : [`docs/preset_bootstrap_analysis_traps.md`](docs/preset_bootstrap_analysis_traps.md).
- [x] **Snapshot preset au branchement** — `Waiting68o` : préambule structurel (plus de `head∈{39,3c}` en dur) + filet 1er chunk 272 ; commit `975b29c`. Ex. validé : **TX WOODY BLUE** (index 27), 125/125 noms.
- [x] **Fermeture app propre** — `sub=0x02` keepalive, lanes `ed→f0→ef`, sans figer le Stomp en mode éditeur. Doc : [`docs/quitter_sans_figer_hardware.md`](docs/quitter_sans_figer_hardware.md) (commit `cd8dcb0`).
- [x] **Lane preset couplée par défaut** — graine figée `0x1c7e` remplacée par mode couplé (`HX_PULL_COUPLE_LANE=0` pour témoin historique).

_Infrastructure USB/session stabilisée → travail métier possible (`HX_ModelUsbAssign.json`, models, UI) sans redémarrages en boucle._

---

## Budget DSP — limiter les models selon la capacité (Helix LT, Stomp XL)

HX Edit refuse d’ajouter un model quand le **DSP est saturé** ; hxlinux fabrique des presets mais **ne calcule pas encore** cette limite (seul un warning « >8 blocs » sur Stomp, sans somme de charge).

### Hypothèse (validée par inspection `.models` + manuel Helix)

| Élément | Détail |
|---|---|
| **Source** | Champs **`load`** / **`load_stereo`** dans `resources/models/*.models` (pas le catalogue, pas `HelixModelDefs.bin`) |
| **Unité** | Points ≈ **% d’un DSP** (ex. German Mahadeva `load: 28.27` → ~28 % d’une puce) |
| **Plafond** | **~100 par puce DSP** (à calibrer empiriquement sur hardware) |
| **Stéréo** | Utiliser **`load_stereo`** (≈ 2× mono sur beaucoup de blocs) |
| **Topologie** | **Stomp XL** : 1 DSP, paths A+B partagent le budget + **~8 blocs max** ; **Helix LT** : **2 DSP** (path 1 → DSP1, path 2 → DSP2), budgets indépendants |
| **Disponibilité model** | **`devices`** / **`exclude_devices`** + `id` interne Line 6 (ex. `2162944`, `2162694`) + parfois `version` firmware |
| **Autres règles** | Limites par **catégorie** (manuel : ex. max 4 Cab/IR) — en plus de la somme `load` |

Algorithme cible :

```
pour chaque puce DSP :
  charge = Σ load(bloc)  (+ Input/Split/Merge si présents sur ce path)
  refuser ajout si charge + load(nouveau) > budget[device]
```

### Todo implémentation

- [ ] **Calibrer le plafond** avec **Helix LT** (semaine prochaine) : presets test HX Edit, noter refus vs `Σ load` ; confirmer **100 ± ε** par DSP ; tester stéréo (`load_stereo`) et split (1A+1B sur même DSP).
- [ ] **Table `device id` → produit** : mapper les `id` des `.models` (`2162944`, `2162694`, …) vers Stomp XL / Helix LT / Floor (capture ou comparaison device branché).
- [ ] **Résolution load** : `symbolicID` + variante mono/stereo → entrée `.models` (`load` ou `load_stereo`).
- [ ] **Répartition par path** : réutiliser routing / `stomp_layout` (split-merge, path 1A/1B/2A/2B) pour savoir **quel DSP** porte chaque slot.
- [ ] **Picker / assign** : griser ou bloquer les models qui dépasseraient le budget ; message UX clair (comme HX Edit).
- [ ] **Profils device** : étendre au-delà de `isSingleDspDevice()` (`src/models.ts`, aujourd’hui : compteur 8 blocs Stomp seulement).
- [ ] **Script audit** (optionnel) : `sum_load(preset)` + comparaison `HX_ModelUsbAssign.json` / catalogue ; pas besoin de parser `HelixModelDefs.bin` (cache binaire des mêmes `.models`).

_Réf. : champs `load` visibles aussi dans `HelixModelDefs.bin` (sérialisation TLV des `.models`). `HX_ModelCatalog.json` = noms/`chainHex`, pas la charge DSP._

---

## Grille preset selon la machine connectée

Détecter le device (`get_connected_device_name` / PID) et adapter **topologie grille + limites DSP** — commencer par le **Stomp**, préparer le **Helix LT**.

### Topologie (Helix / Line 6)

| Machine | DSP | Branches signal | Slots Kempline (cible) | Limite blocs |
|---|---|---|---|---|
| **HX Stomp XL** | 1 | **2** : A + B (même DSP) | 16 segments dump (8+8), **~8 blocs réels** | `load` Σ ≤ ~100 |
| **Helix LT** (Floor, Rack…) | 2 | **4** : 1A, 1B, 2A, 2B | **32** segments (8×4) | ~100 par DSP (1A+1B → DSP1 ; 2A+2B → DSP2) |

Attention nomenclature **hxlinux actuel** : la matrice `renderGrid16` affiche « Path 1 » / « Path 2 » pour les **deux rangées Stomp** — en langage Helix complet ce sont plutôt **1A et 1B**, pas « Path 1 » vs « Path 2 » du LT. Le LT ajoutera **Path 2A / 2B** (2ᵉ DSP).

### Déjà en place

- USB : `SUPPORTED_DEVICES` + `connected_device_name` (`HX Stomp XL`, `Helix LT`, …).
- I/O catalogue : `flowIoCatalogIdsForConnectedDevice()` — Stomp vs HD2 (`HelixStomp_AppDSPFlow*` vs `HD2_AppDSPFlow*`).
- Grille **16 cases** : `isKemplineGrid16`, `stomp_layout.rs`, matrice **4 lignes** (path1 + **desc** + path2 + **desc**) — la moitié sert aux libellés catégorie (à supprimer, cf. ci‑dessous).

### Refonte UI grille (responsive) — décisions produit

- [ ] **Supprimer les rangées « Description Path »** (`LINE_DESC_PATH_1` / `_2`, `.hx-matrix-category`) : l’**icône catégorie** suffit à identifier le type de bloc ; garder le **nom du model** en tooltip / infobulle au survol ou dans le panneau params (sélection). Gain : **−50 % hauteur** sur Stomp, base saine pour 4 paths LT.
- [ ] **Séparateurs entre slots** (icône ligne / rail vertical — `Icons_line.png`, `Icons_vertical_line.png`, etc.) : **largeur minimale** tant que le slot adjacent est vide ; **élargissement dynamique** (`flex-grow`, `minmax()`, ou colonnes `fr` dans la grille) selon la **largeur disponible** du panneau — le séparateur « respire » avec la fenêtre au lieu d’occuper une colonne fixe 56 px partout.
- [ ] **Cellules slots** : `--hx-matrix-cell: clamp(32px, …, 48px)` (remplacer le 56 px fixe TS + CSS) ; conteneur scroll horizontal ou **scale fit-width** si la matrice dépasse.
- [ ] **LT — layout 2 colonnes DSP** (Path 1 \| Path 2) plutôt qu’empiler 4 rangées pleine largeur.
- [ ] **`GridRenderer` abstrait** : `DeviceProfile` → spec lignes/colonnes ; Stomp et LT partagent logique slots, pas le même DOM.

### Todo implémentation (device + parseur)

- [ ] **`DeviceProfile`** (Rust + TS) : `stompSingleDsp` \| `helixDualDsp` \| `hxFx`… dérivé du nom/PID connecté.
- [ ] **Stomp** : formaliser le profil (2 branches, 8 blocs, 1 budget ~100) ; brancher **somme `load`** (section Budget DSP) ; garder warning 8 blocs.
- [ ] **Grille Stomp** : renommer libellés UI si besoin (A/B ou « branche haute/basse ») pour ne pas confondre avec Path 2 du LT.
- [ ] **Helix LT** : parser / afficher **32 segments** ; UI dual-DSP (cf. refonte ci‑dessus).
- [ ] **Routing** : split/merge par paire A/B (réutiliser `stomp_layout` / `computeRoutingJunctionColumns` par DSP path).
- [ ] **Preset dump** : vérifier taille chaîne Kempline LT (32 vs 16) dans le parseur Rust (`try_parse_preset_kempline_grid`, etc.).

_Fichiers touchés en priorité UI : `src/models.ts` (`renderGrid16`), `src/styles.css`, `models.html`._

---

## Refactor nommage « Kempline »

- [ ] Renommer progressivement les identifiants **`kempline_*`**, types **`KemplineCell`**, attributs **`data-kempline-slot-index`**, commande Tauri **`get_active_preset_kempline_flow_chain_param_values`**, etc., vers un vocabulaire **produit** (ex. grille 16 slots, `preset_slot_index`, `grid16_*`, `flow_segment_*`).
- [ ] Ajouter une courte section dans **`README.md`** : l’app s’inspire du reverse **helix_usb / Kempline** ; le code actuel **n’est plus** une traduction ligne à ligne — les comparaisons avec les analyses Kempline ne suffisent pas à juger « bon / faux » sans contexte HXLinux.
- [ ] Conserver une **table de correspondance** (ancien nom → nouveau) dans le premier commit du refactor, pour les recherches git et les discussions issues.

_Raison : éviter que des développeurs optimisant ou modifiant le dépôt comparent avec Kempline et concluent à tort à une erreur d’implémentation._

## Scroll modèle HW — UX et robustesse (plus tard)

- [~] **Architecture scroll : un chemin par type de modèle** (comme loopers) — routeur `extract_module_hex_for_hw_scroll_dump` (standard → **Amp+Cab** → looper) ; chemin Amp+Cab dédié (`c319` + `1a`, dual-slot `19…1a…09`, paires `c219`, token court `2b`+cab) + `categoryHint` UI scroll. Reste : Send/Return, I/O routing, mute amp ~12ᵉ scroll à valider terrain. **Spec** : [`docs/todo-scroll-hw.md`](docs/todo-scroll-hw.md) § *Piste ouverte — extraction par type de modèle*.
- [ ] **Popup consigne utilisateur** : au premier scroll / commande matérielle détectée pendant une session éditeur active, afficher une popup du type « évitez d’utiliser les commandes du Stomp pendant l’utilisation du programme ; préférez l’éditeur » (aligné handoff §0). **Prévoir un flag dev** (`HX_SKIP_HW_SCROLL_WARNING=1` ou équivalent) pour ne **pas** déclencher la popup pendant les tests terrain — sinon galère à valider le multi-cran.
- [x] **Chargement preset sans flags debug** — lane couplée ON par défaut ; bootstrap preset + snapshot corrigés (voir **Réalisé**). Reste : validation session « normale » sans env de trace ; gate `editor_ready` sur le pull si un cas limite réapparaît.

_Raison : l’utilisateur final ne lancera pas l’app avec une ligne d’env ; le scroll HW reste expérimental (~92 %) — la popup et le bootstrap preset doivent tenir sans flags._

## `HX_ModelUsbAssign.json` — complétude, schéma, alignement catalogue

- [ ] **Campagne hardware** : vérifier les **autres familles de modèles** (au-delà des distorsions / ce qui est déjà capturé), captures USB si besoin, et **ajouter / valider** les entrées dans **`src-tauri/resources/HX_ModelUsbAssign.json`** (une ligne `id` + `variant` + `bulkHex` valide par cas testé).
- [ ] **Audit de structure** : aujourd’hui le Rust (`helix/edit_slot_model.rs`, `load_usb_assign_entries`) ne lit que **`id`**, **`variant`**, **`bulkHex`**. Le picker (`hxModelCatalogMeta.ts`, `loadUsbAssignPickerDataFromJson`) lit en plus **`name`**, **`category`**, **`subCategory`**. Les champs **`edOpcode`**, **`bulkKind`**, **`chainHexHint`** (et **`notes`**) ne sont **pas** consommés par le code — redondants ou purement doc par rapport au bulk. Décider : les retirer, les garder comme doc seulement (mettre à jour la description du fichier + schéma `schemaVersion`), ou les **dériver / valider** par script à partir de `bulkHex` pour éviter la dérive.
- [ ] **Alignement `HX_ModelCatalog.json`** : pour chaque entrée (ou via script), **importer `presetMeta.basedOn`** (et sa valeur) depuis le catalogue **pour la même `id`**, afin d’afficher / filtrer côté UI de façon cohérente avec HX Edit sans dupliquer à la main. Vérifier les cas mono/stéréo / `chainHex` tableau.
- [ ] **Même alignement — champ `image`** : récupérer depuis le catalogue la valeur **`image`** (nom de fichier PNG sous `icons_models/`, etc.) pour la même **`id`**, et la **porter dans `HX_ModelUsbAssign.json`** (ou documenter la jointure) ; étendre le picker / l’UI si besoin pour **lire l’icône depuis l’assign** quand on veut se passer du catalogue pour l’affichage liste modèles.
- [ ] **`chainHexHint` vs catalogue** : intention produit = s’affranchir des **`chainHex` / params erronés** du catalogue pour l’USB. Or **`patch_catalog_chain_into_bulk`** utilise encore **`resolve_catalog_model_chain_bytes`** (`HX_ModelCatalog.json`) quand la chaîne catalogue est assez longue. **`chainHexHint`** dans le JSON d’assign n’est **pas** lu — à exploiter (ou un champ **`chainHexUsb`** dédié) comme **source prioritaire** pour le patch quand présent, avec repli catalogue seulement si absent.
- [ ] **Ordre d’affichage picker vs ordre hardware** : l’ordre des modèles dans le picker suit aujourd’hui **l’ordre des lignes** dans **`HX_ModelUsbAssign.json`**. Une insertion au milieu **décale** l’ordre d’énumération côté fichier sans que ce soit l’ordre « mémoire hardware ». Réfléchir à un champ explicite (**`hardwareOrder`**, **`programIndex`**, etc.) stable, ou une convention « ne trier que par ce champ », documentée dans le schéma du fichier.

_Voir aussi le bloc **Todo** dans **`description.md`**._
