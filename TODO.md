# TODO HXLinux

## Réalisé (mai–juin 2026)

- [x] **Scroll modèle HW (~92 %)** — pull `1b`/`19`, lanes scroll vs preset, settling UI ; reste ~8 % cas limites (voir docs scroll handoff). Analyse commune sur plusieurs nuits — plafond raisonnable, pas de patch Line 6 à attendre.
- [x] **Bootstrap preset / lecture one-shot** — ACK dump sur `editor_ed03_lane` ; trailer phase 4 reconnu structurellement (`sub=0x04`, `17≤len<272`). Doc : [`docs/preset_bootstrap_analysis_traps.md`](docs/preset_bootstrap_analysis_traps.md).
- [x] **Snapshot preset au branchement** — `Waiting68o` : préambule structurel (plus de `head∈{39,3c}` en dur) + filet 1er chunk 272 ; commit `975b29c`. Ex. validé : **TX WOODY BLUE** (index 27), 125/125 noms.
- [x] **Fermeture app propre** — `sub=0x02` keepalive, lanes `ed→f0→ef`, sans figer le Stomp en mode éditeur. Doc : [`docs/quitter_sans_figer_hardware.md`](docs/quitter_sans_figer_hardware.md) (commit `cd8dcb0`).
- [x] **Lane preset couplée par défaut** — graine figée `0x1c7e` remplacée par mode couplé (`HX_PULL_COUPLE_LANE=0` pour témoin historique).

_Infrastructure USB/session stabilisée → travail métier possible (`HX_ModelUsbAssign.json`, models, UI) sans redémarrages en boucle._

- [x] **Path 1 Input / Split — live write + scroll hardware (HX Stomp XL)** — `ioSources[]` / `splitSources[]` dans `HX_ModelUsbAssign.json` ; Rust `path1_io_live_write` / `path1_split_live_write` ; picker verrouillé + sync molette sans recharger le preset. Piège Line 6 : scroll Split Stomp envoie **Y=1 / A/B=0** sur le fil, select HX Edit et JSON **`splitSources[]`** restent **Y=0 / A/B=1** (`TT=05`).

---

## Path 1 structurel — Input / Output / Split / Merge (live write + scroll HW)

Hors campagne **`bulkHex`** effets : trames dédiées (`1d` Input, `25` Split, …) documentées dans `ioSources[]` / `splitSources[]` + captures `captures/usb-wireshark/`.

| Bloc | `slot_bus` | Stomp XL (juin 2026) | Helix LT |
|------|------------|----------------------|----------|
| **Input** Path 1 | `0x00` | [x] Clic picker → live write ; scroll HW → picker (`82:62:00:33:XX`, IN `21`) | [ ] **À identifier** — sources, wire `@input`, scroll `21` |
| **Output** Path 1 | `0x09` | [~] Picker verrouillé + focus USB ; pas encore live write / scroll HW | [ ] **À identifier** — `ioSources[]` Output, trames OUT, scroll |
| **Split** Path 1 | `0x0a` | [x] Clic picker → live write `25` ; scroll HW → picker (swap Y/A/B scroll vs select) | [ ] **À identifier** — même piège encodage Y/A/B ? |
| **Merge** Path 1 | `0x13` | [~] Picker verrouillé (Join) + focus USB ; pas encore live write / scroll HW | [ ] **À identifier** — `mergeSources[]` ou équivalent, trames USB |

### Todo LT (quand hardware disponible)

- [ ] **Brancher Helix LT** : confirmer `flowIoCatalogIdsForConnectedDevice()` (`HelixFx_*` vs `HelixStomp_*` / `HD2_*`) pour Input/Output/Split/Merge Path 1 **et** Path 2 si applicable.
- [ ] **Captures Wireshark LT** par bloc : select UI HX Edit + scroll molette (fichiers `input scroll LT.json`, `split scroll LT.json`, …) — ne pas supposer les mêmes octets que Stomp XL.
- [ ] **Input LT** : valider ou compléter `ioSources[]` (nombre de sources, `wireValue`, `liveWriteHex`).
- [ ] **Output LT** : implémenter live write + scroll sur le modèle Input (à capturer).
- [ ] **Split LT** : vérifier convention Y/A/B (select vs scroll) — le piège Stomp peut différer.
- [ ] **Merge LT** : capturer assign + scroll mixer ; ajouter `mergeSources[]` si plusieurs variantes.

_Rust : `write_path1_input_source`, `write_path1_split_type`, `get_path1_*_wire_value` ; events `models:path1-input-source-changed`, `models:path1-split-type-changed`. Tests : `cargo test path1_split` / `path1_input`._

---

## Matrice — copier/coller, déplacer (drag & drop), cache params session

> Handoff technique (problèmes, solutions Pointer Events, architecture cache) :
> **[`docs/matrix-edit-handoff.md`](docs/matrix-edit-handoff.md)**

### Réalisé (juin 2026)

- [x] **Copier / coller inter-preset et multi-slot** — buffer persistant (`matrixSlotClipboard` non vidé au changement de preset) ; coller sur toute case FX vide (inter-path). Corrige le bug « plusieurs coller : UI OK, hardware pas à jour » (cause : copie perdue ou coller refusé hors même path/preset).
- [x] **Déplacer slot (drag & drop)** — Pointer Events (`pointerdown` / `pointermove` / `pointerup` + `setPointerCapture`) ; HTML5 DnD abandonné (WebKitGTK ne déclenchait pas `drop`). Flux : copier → coller destination → vider source (`remove`).
- [x] **Contraintes move v1** — même path (slots 0–7 ↔ 0–7, 8–15 ↔ 8–15) ; destination vide ; pas I/O / Split / Merge ; `mergeProbeSlotModelUntil` multi-slot après move.
- [x] **Focus UI post-move** — sélection + panneau params sur le slot destination (hardware déjà positionné dessus).
- [x] **Cache session params (`slotChainSessionByKey`)** — `preset_data` lu **une seule fois** au chargement / changement de preset (`hydrateSlotChainSessionFromPresetData`) ; ensuite `resolveChainValuesForKemplineSlot` (session + overrides live + défauts catalogue). **Plus de** `fetchSlotChainParamValuesReliable` au clic slot.
- [x] **Scroll modèle HW** — plus de re-dump preset après molette (`schedulePresetRamRefreshAfterHwModelScroll` supprimé) ; défauts catalogue + cache session.
- [x] **Fix params `cd0209` (Ampeg Scrambler)** — ancrage délimiteur `1aff` dans parse Rust (`preset_chain_params.rs`, `extract_c219_argument_type_hexes`).
- [x] **Warnings compilation** — TS et Rust nettoyés (code mort supprimé).

### Bug a Corriger

_Détail et pistes : [`docs/matrix-edit-handoff.md`](docs/matrix-edit-handoff.md) §6._

- [x] **Drag & drop — purge source HW** — copier → remove (focus USB) → délai → coller ; verrou UI `models-matrix-usb-busy`.
- [ ] **Lecture preset après D&D** — après plusieurs moves, changement de preset souvent en échec. Correctif juin 2026 : reset `content_only` fantôme côté Rust + attente post-probe avant dump ; **à revalider terrain**.

### À faire — drag & drop inter-path et structurel

- [ ] **Move Path 1 → Path 2** (et réciproque ?) — étendre le DnD au-delà du même path (0–7 ↔ 8–15) ; contraintes DSP / budget load ; comportement hardware USB.
1 - Lorsque l'on fait un drag & drop du path 1 vers le path 2 et que le path 2 est vide, un split se créer automatiquement juste apres le Input et un merge juste avant le Output
2 - Il n'est pas possible de faire un drag & drop depuis un path vers un autre path sur un slot positionné avant le split ou apres le merge.
- [ ] **Drag & drop Split et merge** — déplacement ou réassignation du bloc Split Path 1; trames live write / focus USB.
1 - L'utilisateur peut drag & drop un split mais jamais apres le premier slot rempli du path 2
2 - L'utilisateur peut drag & drop un merge mais jamais avant le dernier slot rempli du path 2
3 - Comme pour le drag & drop de model, le split ou le merge d'origine est supprimé


_Fichiers touchés en priorité : `src/models.ts` (`moveMatrixSlotFromTo`, `canMoveMatrixSlotToEmpty`, `bindMatrixSlotDragSource`), `src/styles.css` (feedback visuel `node--matrix-drag-*`)._

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

### Campagne bulkHex — captures Wireshark (en cours)

Workflow : filtre + export JSON → `scripts/inject_bulk_from_captures.py` → test assign picker terrain. Doc : [`captures/usb-wireshark/README.md`](captures/usb-wireshark/README.md).

**État juin 2026** : **335** entrées avec `bulkHex` sur **1087** (campagnes effets en cours).

| Famille | Statut capture / injection | Pièges déjà rencontrés |
|---------|------------------------------|------------------------|
| EQ mono + stéréo | **16/16** | — |
| Modulation mono + stéréo + legacy | **79/79** | — |
| Delay mono + stéréo + legacy | **68/68** | `L6SPB_InfSustain` = doublon caché de `VIC_DelayPolySustain` ; catalogue `cd0243` erroné → fil scroll **`cd0265`** (capture `scroll to poly sustain.json`) |
| Reverb mono + stéréo + legacy | **38/38** | moderne **et** legacy en **`cd:05`** (pas `cd:03`) ; legacy **44 o** + hints **4 nibbles** (`ccf6`, `ccf7`, …) ; extracteur actuel OK sans correctif |
| Pitch/Synth mono + stéréo + legacy | **37/38** | tout en **`cd:05`** ; classiques **44 o** (`ccb6`…) ; doublon catalogue **12-String** (`L6SPB_12String` / `VIC_PitchTwelveString` → même `cd026c`) ; **`Poly Bass Wham`** absent capture + hint vide catalogue |
| Filter mono + stéréo + legacy | **19/19** | **`cd:05`** ; classiques 44 o (`cc89`…) ; modernes 48 o ; legacy 48 o — extracteur OK sans correctif |
| Wah mono + stéréo | **22/22** | mono **`cd:05`** / stéréo **`cd:06`** ; hint stéréo UK Wah = `cd011a` (catalogue avait `cd01` erroné) ; correctif extracteur `1aff` pour ids `cd…1a…` |
| Amp Guitar + Bass (`variant: amp` seul) | **111/111** | captures `guitar:` / `bass:` → `subCategory` ; Guitar **`cd:06`**, Bass **`cd:06`/`cd:07`** ; hints courts (`2c`, `1a`) + longs ; **Line 6 Doom** : catalogue `1a47` → fil **`1a`** |
| Amp+Cab Guitar + Bass (IR, `amp+cab`) | **111/111** | **un bulk** / entrée (`8317c319`, amp `1a` cab IR `cd:03:xx`) ; préfixes **`25:00`** + **`27:00`** ; hint = entrée **`amp`** jumelle |
| Amp+Cab Legacy Guitar + Bass (`amp+cab-legacy`) | **111/111** | même **`c319`** ; cab hybrid **courte** (`2c:1a:47:00`) vs IR (`2c:1a:cd:03:29`) ; longueurs **44/48** |
| Preamp Guitar + Bass + Mic | **113/113** | `c219` comme Amp ; corps **`cd:08`** ; hints `ccc1`… / `cce7` (mic) ; **Grammatico Nrm** : catalogue `cd02` → fil **`cd021a`** |
| Cab `single` (IR Mic) | **46/46** `subCategory: Single` | `c219` ; **`cd:09`** / **`cd:0a`** |
| Cab `dual` (IR Mic) | **46/46** `subCategory: Dual` | **`c319`** + hint `1a` + suffixe stéréo **`cd02d6`** — ≠ bulk single ; hint `cd031c` vs single `cd031b` ; **2x12 Interstate** catalogue `cd03` → **`cd031a`** |
| Cab Legacy | **82/82** | `c219` single / `c319` dual ; hints courts `33`… + 11× `cd02…` ; dual voie droite défaut **1x12 Lead 80** (`30`) |
| IR | **3/3** | dual = **`c219`** `cd02c4` (pas cab-style `c319`) |
| Volume/Pan | **7/7** | `cd:04`, 48 o |
| Looper | **7/7** | Shuffling mono/stéréo hints distincts ; `VIC_LooperShuffling` copié sur `cd0268` |
| Send/Return | **9/18** | Send/Return/FX Loop **3 et 4** non capturés (`chainHex` vide catalogue) |
| Distortion, Dynamics | partiel (**105** + **30** avec bulk) | à confirmer / compléter |
| Input, Output, Split, Merge (live write / scroll) | **Input + Split OK Stomp XL** | pas `bulkHex` — voir section Path 1 structurel ; **LT à identifier** |
| Connected Devices | **0** | hors scope assign classique |

- [ ] **Poursuivre la campagne** famille par famille (Distortion/Dynamics → Amp → Cab → Preamp → …) : une capture JSON par **catégorie × variante** (`mono`, `stereo`, `legacy`, `amp`, `single`, `dual`, …), puis injection + test picker sur Stomp.
- [ ] **Test terrain Reverb** : assign picker mono / stéréo / legacy sur Stomp après injection `reverb *.json`.
- [ ] **Durcir l’extracteur au fil de l’eau** : chaque nouvelle famille peut introduire un préfixe longueur, un octet `cd:XX` ou un encodage `chainHexHint` non vu (cf. correctifs Delay dans `inject_bulk_from_captures.py`). Documenter chaque cas dans le README captures.
- [ ] **Ne pas confondre assign et scroll** dans les captures : paquets `1b:00` 36 o = lane pull scroll, **pas** bulk assign — erreur fréquente sur Delay, à garder en tête pour les familles suivantes.
- [ ] **Vérifier les hints courts après chaque injection** : `rg '"chainHexHint": "[0-9a-f]{2}"'` + entrées avec `bulkHex` non vide ; le scroll résout les noms via `chainHexHint` (`model_catalog`) — trous ou hints mal extraits = modèles « inconnus » à la molette même si la capture est bonne.

#### Amp+Cab — risque élevé (à traiter en dernier ou avec soin)

L’assign Amp+Cab n’est **pas** un simple bulk effet : variante picker **`amp+cab`** / **`amp+cab-legacy`**, `chainHexHint` souvent **vide** sur le clone (partage le fil **`amp`**), encodage dual-slot (`c319` + `1a`, paires amp/cab, token court) déjà problématique en **scroll** et **preset** (WhoWatt slot 0, bootstrap, `split_preset_by_8213`). S’attendre à :

- [ ] **Captures dédiées** : `amp+cab` moderne (Guitar/Bass IR) et `amp+cab-legacy` (hybrid) — **séparées** de la capture `amp` seule ; ne pas réutiliser le bulk `amp` pour le picker Amp+Cab.
- [ ] **Critères d’extraction à définir / étendre** : format bulk probablement différent des effets (`83:66:cd:03` ? autre marqueur ? longueur 48 o vs autre ?) ; le routeur scroll [`extract_module_hex_for_hw_scroll_dump`](docs/todo-scroll-hw.md) a déjà un chemin Amp+Cab — **aligner** extracteur injection et parseur scroll sur les mêmes motifs.
- [ ] **`chainHexHint` / index scroll** : clones `amp+cab` non indexés seuls (`chain_hex_hint_shared_with_amp`) — vérifier que la campagne bulkHex + `categoryHint` UI couvrent bien l’assign **et** l’affichage scroll sans régression sur le fil `amp`.
- [ ] **Tests terrain obligatoires** : assign picker Amp+Cab, scroll molette sur slot Amp+Cab, preset avec Amp+Cab en slot 0 (régression bootstrap / grille).
- [ ] **Script sync** : `sync_usb_assign_from_catalog.py` génère déjà les stubs `amp+cab` / `amp+cab-legacy` — préserver les `bulkHex` capturés au fil des injections.

- [ ] **Campagne hardware (rappel global)** : compléter **`src-tauri/resources/HX_ModelUsbAssign.json`** jusqu’à une entrée `id` + `variant` + `bulkHex` valide par cas testé ; `--allow-partial` tant que des modèles manquent dans HX Edit.
- [ ] **Audit final post-campagne** : passer en revue toutes les entrées **sans `bulkHex`** et classer la cause (pas encore capturé, absent du picker HX Edit / `hidden`, doublon id catalogue, `chainHexHint` vide, I/O & routing hors assign classique, vrai trou). Inventaire de départ juin 2026 : **715** vides / 1087 — surtout **Amp / Preamp / Cab / Amp+Cab** (≈610), puis Wah/Filter/Send-Return ; **245** sans `bulkHex` **et** sans `chainHexHint` (dont 222 clones Amp+Cab).
- [ ] **Audit de structure** : aujourd’hui le Rust (`helix/edit_slot_model.rs`, `load_usb_assign_entries`) ne lit que **`id`**, **`variant`**, **`bulkHex`**. Le picker (`hxModelCatalogMeta.ts`, `loadUsbAssignPickerDataFromJson`) lit en plus **`name`**, **`category`**, **`subCategory`**. Les champs **`edOpcode`**, **`bulkKind`**, **`chainHexHint`** (et **`notes`**) ne sont **pas** consommés par le code — redondants ou purement doc par rapport au bulk. Décider : les retirer, les garder comme doc seulement (mettre à jour la description du fichier + schéma `schemaVersion`), ou les **dériver / valider** par script à partir de `bulkHex` pour éviter la dérive.
- [ ] **Alignement `HX_ModelCatalog.json`** : pour chaque entrée (ou via script), **importer `presetMeta.basedOn`** (et sa valeur) depuis le catalogue **pour la même `id`**, afin d’afficher / filtrer côté UI de façon cohérente avec HX Edit sans dupliquer à la main. Vérifier les cas mono/stéréo / `chainHex` tableau.
- [ ] **Même alignement — champ `image`** : récupérer depuis le catalogue la valeur **`image`** (nom de fichier PNG sous `icons_models/`, etc.) pour la même **`id`**, et la **porter dans `HX_ModelUsbAssign.json`** (ou documenter la jointure) ; étendre le picker / l’UI si besoin pour **lire l’icône depuis l’assign** quand on veut se passer du catalogue pour l’affichage liste modèles.
- [ ] **`chainHexHint` vs catalogue** : intention produit = s’affranchir des **`chainHex` / params erronés** du catalogue pour l’USB. Or **`patch_catalog_chain_into_bulk`** utilise encore **`resolve_catalog_model_chain_bytes`** (`HX_ModelCatalog.json`) quand la chaîne catalogue est assez longue. **`chainHexHint`** dans le JSON d’assign n’est **pas** lu — à exploiter (ou un champ **`chainHexUsb`** dédié) comme **source prioritaire** pour le patch quand présent, avec repli catalogue seulement si absent.
- [ ] **Ordre d’affichage picker vs ordre hardware** : l’ordre des modèles dans le picker suit aujourd’hui **l’ordre des lignes** dans **`HX_ModelUsbAssign.json`**. Une insertion au milieu **décale** l’ordre d’énumération côté fichier sans que ce soit l’ordre « mémoire hardware ». Réfléchir à un champ explicite (**`hardwareOrder`**, **`programIndex`**, etc.) stable, ou une convention « ne trier que par ce champ », documentée dans le schéma du fichier.

---

## Picker Cab — instrument Guitar / Bass (au-delà de Line 6)

HX Edit n’empêche pas d’associer n’importe quel cab à n’importe quel ampli, mais le catalogue porte déjà un indice utile : **`"bass": true`** sur **24** entrées **Cab** (Epicenter, Ampeg, Brute, SVT, etc.) — **métadonnée descriptive**, pas une règle firmware. HXLinux ne l’exploite pas encore ; pour un bassiste (ou un guitariste), savoir qu’un cab est **initialement modélisé pour la basse** est une info produit forte.

### Todo

- [ ] **Schéma assign** : ajouter un champ explicite sur les entrées **Cab** (ex. **`cabInstrument`**: `"guitar"` \| `"bass"`) dans **`HX_ModelUsbAssign.json`** + `fieldGuide` ; import initial depuis **`bass: true`** du catalogue (`scripts/sync_usb_assign_from_catalog.py` ou script dédié) ; défaut **`guitar`** si absent.
- [ ] **Revue manuelle** : compléter / corriger la liste (tous les cabs « évidents » bass ne portent pas forcément le flag catalogue ; l’inverse aussi) — notes dans l’assign ou petit fichier d’audit.
- [ ] **Picker** : suffixe lisible **`(B)`** / **`(G)`** à côté du nom dans la liste Cab (et éventuellement sous-rubriques Single / Dual / Legacy) ; **filtre optionnel** Guitar \| Bass \| Tous (ne **bloque pas** l’assign USB — information seulement, plus clair que HX Edit).
- [ ] **UI slot Amp+Cab** : afficher **`(B)`** / **`(G)`** sur la ligne cab du panneau params et/ou tooltip matrice quand le cab lié est identifié.
- [ ] **Sans dépendre du gros catalogue** : une fois importé dans l’assign, la vérité runtime = assign (comme `chainHexHint` / `image`).

---

## Noms affichés — marque Line 6 vs référence hardware (`basedOn`)

Aujourd’hui la **liste modèles** (picker) et la **matrice** utilisent le champ **`name`** de **`HX_ModelUsbAssign.json`** — codenames Line 6 (*Cali Rectifire*, *Minotaur*, etc.), comme HX Edit.

Le champ **`basedOn`** (ex. *MESA/Boogie® Dual Rectifier*) dans l’assign est une **donnée curatée du dépôt** : renseignée à partir des **fiches publiques Line 6** sur le web (le site explique que *Cali Rectifire* est basé sur le Dual Rectifier®, parfois avec photo du hardware). **Ce texte ne sort pas du firmware ni du binaire HX Edit** — c’est une couche documentaire HXLinux, pas une revendication de marque.

**Choix produit / prudence** : ne **pas** promouvoir ce mode en UI visible (risque perçu marques + confusion avec l’éditeur officiel). **Défaut = toujours les noms Line 6** ; bascule **paramètre caché** (flag env, préférences avancées, combinaison clavier — à définir) pour afficher `basedOn` à la place ou en complément dans le picker / tooltips.

### Todo

- [ ] **Préférence cachée** : ex. `HX_DISPLAY_BASED_ON_NAMES=1` ou entrée « avancé » non exposée au premier lancement — persistance locale ; **désactivé par défaut**.
- [ ] **Règles d’affichage** : si `basedOn` vide → repli sur `name` ; conserver les `®` tels que sur le site Line 6 ; formulation UI du type **« Référence (site Line 6) »** plutôt qu’un « vrai nom officiel ».
- [ ] **Périmètre** : picker liste en priorité ; puis tooltips matrice / en-tête panneau params (le bandeau *Based on : …* peut rester même quand le mode caché est off).
- [ ] **Données** : poursuivre le remplissage **`basedOn`** dans l’assign depuis la **doc publique Line 6** (et repli catalogue / CSV quand aligné) ; noter la **source** dans `notes` ou audit si besoin.
- [ ] **README / disclaimer** : HXLinux non affilié Line 6 / Yamaha / fabricants cités ; marques = propriétaires ; `basedOn` = aide utilisateur issue de docs publiques, pas du device.
- [ ] **`fieldGuide`** : documenter `basedOn` comme *métadonnée curatée (site L6)* ; passer en *runtime TS (affichage optionnel)* seulement quand le flag caché existe.

_Voir aussi le **guide de reprise** dans **`description.md`** (architecture, conventions, commandes)._
