# HXLinux — description pour reprendre une session

Ce fichier sert de **mémo locale** quand l’historique de chat ou le contexte IDE est perdu après un redémarrage. Il complète le `README.md` (objectifs produit et commandes de base).

**Dernière mise à jour significative** : **avril 2026** — panneau paramètres **min | chaîne | max** ; **`HX_ModelCatalog.json`** enrichi (`presetMeta.chainHex` / `signal`, mono+stéréo) ; table **`MODULES_BY_ID`** générée côté Rust depuis le catalogue embarqué ; front **`hxModelCatalogMeta.ts`** (`pickSignal` / `pickChannel`) ; scripts Python sous **`scripts/`** ; il reste des **`chainHex` vides** à compléter à la main (voir section catalogue ci-dessous).

## À quoi sert l’application

**HXLinux** est un éditeur / explorateur de presets pour **Line 6 HX Stomp XL** (et IDs USB voisins listés dans le code), sur **Linux**, en application **desktop Tauri** (Rust + front web).

Fonctions déjà utiles en pratique :

- Connexion **USB** au boîtier, machine d’états côté Rust pour le protocole (inspiré du travail **Kempline / helix_usb**).
- Lecture des **125 noms de presets**, **activation** d’un preset (Program Change), **renommage** depuis l’UI.
- Chargement du **contenu binaire du preset actif**, parsing partiel en **« slots »** (catégorie + nom) pour l’affichage.
- Mise en page type **grille** (16 blocs + routage), données renforcées par **`stomp_layout`** (split/merge, grille USB quand dispo).
- **Panneau paramètres** (sous la grille dans la vue models) : clic sur un bloc → définitions **`.models`** (noms, min, max) + valeurs **chaîne** lues dans le segment binaire du slot (**pas** de requête USB supplémentaire ; tout vient du dump déjà chargé). Les pastilles de la matrice 16 portent **`data-kempline-slot-index`** (0–7 path 1, 8–15 path 2) pour cette corrélation.

**État réel des valeurs chaîne** : alignement corrigé avec `user_slot_reader` Kempline (pointeur après le délimiteur `09` : ne **pas** faire `rel09 + 2` puis `+= 4` comme avant ; même séquence que Python `bytes_read` au début du `09` puis premier `+= 4`). En usage, **~90 %** des paramètres affichés correspondent bien ; le reste peut venir d’ordre / champs internes vs liste `params` du JSON, blocs **Amp+Cab** (`c319`, non géré dans `preset_chain_params`), **IR**, etc.

Ce qui reste largement ouvert : **édition** des paramètres vers l’appareil, export/import de fichiers presets (voir `README.md`).

---

## Lecture des paramètres « dans la chaîne » (ce qui a été fait — avril 2026)

Les **valeurs** affichées dans la colonne **chaîne** ne viennent **pas** des fichiers `.models` : elles sont **décodées dans le binaire du preset** déjà reçu en USB (`RequestPreset` → accumulation dans `HelixState.preset_data`). Les `.models` fournissent seulement les **métadonnées** (nom du paramètre, min/max du slider HX Edit, défaut, `displayType`, etc.).

### Chaîne de traitement (Rust)

1. **`split_preset_by_8213`** (`lib.rs`) — découpe le flux en segments (marqueur `82 13` côté octets, équivalent au split hex `8213` chez Kempline).
2. **`kempline_grid_window_start_and_seg_count`** — retrouve la **fenêtre de 20 segments** validée comme grille Kempline (même critères que `try_parse_preset_kempline_grid` : segment `00`, `01`, `02`, `03` aux positions attendues, 16 blocs assignables en `06` ou `08`).
3. **`kempline_assignable_segment_bytes(data, slot_index)`** — pour un index **0…15** (ordre UI : path1 slots 0–7, path2 slots 8–15), renvoie le **segment brut** `&[u8]` correspondant à `KEMPLINE_ASSIG_INDICES[slot_index]`.
4. **`parse_standard_assignable_segment`** (`preset_chain_params.rs`) — logique calquée sur **`user_slot_reader`** + **`read_params`** du Python Kempline (`simple_filter.py`) :
   - n’accepte que les segments dont le **premier octet** est **`0x06`** (slot « standard » ; le `0x08` vide ou d’autres variantes ne suivent pas ce chemin) ;
   - passe le reste en **chaîne hex** (deux caractères par octet), comme le script Python sur sa chaîne ;
   - cherche le motif **`85188317`**, refuse **`c319`** (Amp+Cab : autre lecteur dans le Python, **non porté** chez nous pour l’instant) ;
   - attend **`c219`**, extrait le « type » jusqu’au premier **`09`**, recale **`br`** exactement comme Python : **`br` sur le premier caractère du `09`**, puis premier saut **`+= 4`** (quatre caractères hex : en pratique `09` + la paire suivante) — une erreur ici décale **tout** `read_params` ;
   - lit **`num_params`** sur **un octet** exprimé en **deux nibbles hex** consécutifs (`int(c0)*16 + int(c1)` en Python) ;
   - saute **8** caractères hex additionnels, puis **`read_params_hex`** : suite de tokens (`c2`/`c3` bool, `ca` + 8 hex = float IEEE arrondi, paires hex = entier u8, optionnellement bloc `1bda…`).

Les valeurs renvoyées au front sont une liste **`ChainParamValue`** (sérialisation JSON **untagged** : booléen, nombre, ou chaîne hex pour les blobs).

### Chaîne de traitement (TypeScript)

1. Chaque pastille de la grille 16 a **`data-kempline-slot-index="0"` … `"15"`** (`gridSlotNode` dans `models.ts`).
2. Au clic, **`loadAndShowModelsParamsForSlot`** appelle **`invoke("get_active_preset_slot_chain_param_values", { slotIndex })`** (si l’index est défini), puis charge le JSON **`.models`** (cache + `read_models_definition_file` / fetch selon l’environnement).
3. **`findModelDefinitionForSlot`** associe le **nom long** issu du preset (table `MODULES_BY_ID` / `modules.py`) au bon objet modèle dans le tableau JSON (matching préfixe / nom le plus long, etc.).
4. **`renderModelsParamsPane`** affiche une ligne par entrée **`params[]`** du modèle : **min** et **max** viennent du JSON ; la **chaîne** est **`chainValues[i]`** à la même position **`i`** — c’est un **zip par indice** entre la liste décodée du firmware et la liste des paramètres HX Edit. Ce n’est **pas** une jointure par `symbolicID` : si l’ordre diverge ou si le firmware expose des champs internes non listés dans le JSON, l’alignement peut être faux pour quelques lignes.

### Ce qui n’était pas (encore) dans la description avant cette complétion

- Le **fil exact** preset → segment slot → hex → `read_params` → `invoke` → zip avec `.models`.
- Le **rôle distinct** : binaire = valeurs, `.models` = schéma d’UI.
- La **référence explicite** au Python Kempline pour la spec du parseur.

---

## Prochaine étape : fichiers `.models` et affichage (virgule, 0 / 1 / 2 → libellés)

**Oui** : pour que l’affichage colle à l’interface HX (virgule au bon endroit, **0 / 1 / 2** affichés comme **220 Hz / 800 Hz / 3000 Hz**, etc.), il faudra **enrichir** les données — en pratique les **`.models`** (champs **optionnels** pour ne pas casser les outils qui s’attendent au format Line 6 d’origine) **ou** un fichier / table séparée dans le dépôt.

Pistes de champs (à valider ensemble avant implémentation dans `models.ts`) :

- **Échelle linéaire** : par ex. `chainScale`, `chainOffset`, `chainDecimals` (appliqués à la valeur numérique brute renvoyée pour la colonne « chaîne » ou une colonne dérivée « affichage »).
- **Liste discrète** : par ex. `chainEnum` : `[{ "raw": 0, "label": "220 Hz" }, …]` ou tableau de labels indexés par l’entier lu ; si la chaîne est un **u8** `2`, afficher le libellé d’index 2.
- **Registre dans le code** : pour les `displayType` répétitifs, une seule règle dans `models.ts` évite de dupliquer des milliers de lignes dans les JSON ; les cas **spécifiques à un modèle** restent dans le `.models` de ce modèle.

Tant que ces règles ne sont pas déclarées, la colonne **chaîne** reste une **vue brute** (ou légèrement formatée : bool on/off, float arrondi côté Rust) — d’où les écarts d’échelle par rapport à l’écran du Helix.

---

## Stack technique

| Couche | Rôle |
|--------|------|
| **Rust / Tauri 2** | USB (`rusb`), threads listener/écriture, état `HelixState`, commandes `invoke` exposées au front. |
| **TypeScript + Vite 6** | UI : `src/main.ts` (liste presets + intégration workspace), `src/models.ts` (vue « chaîne / grille » des blocs du preset + panneau paramètres). |
| **CSS** | `src/styles.css` — styles partagés ; la page `models.html` importe aussi ce fichier via `models.ts`. |

Build front : `npm run build` (`tsc` + `vite build`). App complète : `npm run tauri dev` / `npm run tauri build`.

## Structure des dossiers (utile au quotidien)

```
hxlinux/
├── index.html              # Fenêtre principale : liste + panneau « HX Models » (même document que main.ts)
├── models.html             # Entrée Vite secondaire (build MPA) ; utile si tu ouvres cette page seule en dev
├── description.md          # Ce fichier — mémo de reprise de session
├── src/
│   ├── main.ts             # Liste presets, statut, drag/rename, appels invoke vers Rust
│   ├── models.ts           # Rendu grille / chaîne preset, polling, stomp_layout, panneau params + invoke chaîne
│   └── styles.css          # Tout le look `.models-pane`, matrice `hx-matrix-*`, `.models-params-*`, etc.
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs          # Commandes Tauri, AppState, parse preset, fenêtre Kempline 20 segments, invoke
│   │   ├── preset_chain_params.rs  # parse segment slot 0x06 : 85188317 / c219 / read_params (serde → UI)
│   │   ├── stomp_layout.rs # Layout stomp + routing (split/merge cols) aligné USB / heuristiques
│   │   └── helix/          # Protocole : modes (connect, request preset(s), standard…), USB, paquets
│   ├── resources/          # Bundlé : HX_ModelCatalog.json, icons_*, models/*.models (gros fichiers)
│   └── tauri.conf.json     # devUrl 1420, ressources bundle
└── README.md               # Statut produit, prérequis, crédits Kempline
```

## Deux surfaces front pour les « models »

1. **Dans la fenêtre principale** (`index.html`) : section `.models-pane` avec `<main class="models-content" id="content">`. **`main.ts` et `models.ts` sont tous les deux chargés** sur cette page ; `models.ts` attache son UI à `#content` / `#status` / `#preset-label` **du panneau droit** (attention aux `id` dupliqués si tu dupliques des fragments HTML).
2. **`models.html`** : page dédiée au build Vite ; `models.ts` y importe `./styles.css`. Le `<main id="content" class="content models-pane">` sert à activer les sélecteurs `.models-pane` / `#content.models-pane` (layout matrice, largeur grille, etc.).

En dev Tauri, ce qui compte le plus est souvent **index + models.ts** dans le même WebView.

## Rust — commandes exposées (`invoke`)

Déclarées dans `lib.rs` (`tauri::generate_handler![...]`), typiquement utilisées par le front :

| Commande | Rôle court |
|----------|------------|
| `get_preset_names` | Liste des noms (125 entrées). |
| `get_active_preset` | Index preset actif (0-based côté app). |
| `get_connected_device_name` / `get_connection_hint_text` | Statut connexion / message utilisateur. |
| `activate_preset` | Program Change USB. |
| `rename_preset` | Renommage sur l’appareil (ASCII, longueur limitée). |
| `request_preset_content` | Lance la lecture du dump preset actif. |
| `get_active_preset_slots` | Slots `[catégorie, nom]` quand le dump est prêt **et** cohérent avec `active_preset`. |
| `get_active_preset_slots_debug` | Idem + coords grille debug. |
| `get_active_preset_routing_markers` | Entrées routing type Split/Merge si présentes dans le parse. |
| `get_active_preset_stomp_layout` | Objet `ActivePresetStompLayout` (grille OK, split/merge cols, etc.). |
| **`get_active_preset_slot_chain_param_values`** | **`{ slotIndex: 0..15 }`** → `Vec<ChainParamValue>` ou `null` : valeurs décodées `read_params` pour le segment assignable Kempline du slot (voir `preset_chain_params.rs`). |
| `read_models_definition_file` | Lecture d’un `resources/models/{base}.models` côté bundle (nom de base alphanumérique). |
| `get_preset_data_hex` | Dump brut hex (debug). |
| `request_active_preset_name` | Resync nom preset actif. |

Le flux côté `models.ts` : après changement de preset → `request_preset_content` → boucle d’attente → `get_active_preset_slots` + routing + `get_active_preset_stomp_layout` pour `renderGrid16`. Au clic sur un slot avec modèle → `get_active_preset_slot_chain_param_values` si `data-kempline-slot-index` est défini, fusion avec le JSON `.models` chargé (fetch ou `read_models_definition_file`).

## Fichiers Rust à connaître pour le preset / UI grille

- **`lib.rs`** — `parse_preset_slots`, `split_preset_by_8213`, `kempline_grid_window_start_and_seg_count`, `kempline_assignable_segment_bytes`, `try_parse_preset_kempline_grid`, `KEMPLINE_ASSIG_INDICES`, commentaires `[PresetDebug]`.
- **`preset_chain_params.rs`** — `parse_standard_assignable_segment`, `read_params_hex`, enum sérialisable `ChainParamValue` (bool, float IEEE via `ca`, u8, blob `1bda`).
- **`stomp_layout.rs`** — `split_merge_from_usb_preset_body`, `compute_stomp_layout_from_kempline_grid_with_usb`, tests ; colonnes split/merge consommées par `models.ts`. Le helper `merge_after_col_from_usb_preset_body` n’existe qu’en build test (`#[cfg(test)]`) pour éviter un warning `dead_code` en `cargo build` lib.

## Front — matrice stomp 4×20 (`renderGrid16` dans `models.ts`)

Grille **20 colonnes × 4 lignes**, cellules **56×56 px** (`NUM_COLS = 20`, `NUM_ROWS = 4`, `CELL_PX = 56`). Nomenclature des lignes dans le code :

| Ligne | Rôle |
|-------|------|
| **L1** | Path 1 — slots 0–7, I/O Input / Output, traits horizontaux `Icons_line.png` entre colonnes paires, pastille `Icons_split_merge.png` aux colonnes **jonction** (split/merge issus du routing). |
| **L2** | Description Path 1 — textes catégorie ; aux colonnes split/merge, petite barre verticale `Icons_vertical_line.png`. |
| **L3** | Path 2 — slots 8–15 si branche B ; aux mêmes colonnes, icônes coin **`Icons_link_split.png`** / **`Icons_link_merge.png`** (alignées sur `stomp_layout`). |
| **L4** | Description Path 2 — catégories path B. |

- **Colonne 20** : numéros de ligne grille (debug lisible).
- **Colonnes « jonction »** : dérivées des frontières split/merge (1…8) via `matrixEvenColForRoutingBoundary` (colonnes paires 2…18 côté UI).
- **`ENABLE_MATRIX_VSPAN_ON_PATH2`** (`models.ts`) : par défaut **`false`**. Un overlay `vspan` vertical sur Path 2 partageait la même `grid-area` que les icônes lien ; les deux se superposaient. Laisser à `true` uniquement pour un revert visuel expérimental (commentaires **REVERT** à côté).
- **Ancienne mise en page (5 lignes + rangée 3 « séparateur » 0 px)** : le retour est documenté en blocs commentés **REVERT** dans `models.ts` et `styles.css` (constantes de lignes, hauteurs de rangées, boucle séparateur, classes `row-line-debug-sep`, etc.).

Panneau paramètres : liste **`.models-params-list`** avec lignes **`.models-params-row`** en grille **nom | min | chaîne | max** (classes `.models-params-row-min`, `-chain`, `-max`).

Le CSS associé est sous **`.models-pane .hx-matrix-*`** et **`.models-params-*`** dans `styles.css`. Des régressions visuelles passent souvent par : parent sans `.models-pane`, ou styles inline dupliqués dans `models.html` vs `styles.css`.

## Ressources et métadonnées Line 6

- **`src-tauri/resources/HX_ModelCatalog.json`** — catalogue modèles.
- **`src-tauri/resources/icons_models/`** — icônes par modèle.
- **`src-tauri/resources/icons_category/`** — icônes catégories + assets maison pour la matrice : `Icons_line.png`, `Icons_split_merge.png`, `Icons_vertical_line.png`, `Icons_link_split.png`, `Icons_link_merge.png`, ainsi que les icônes I/O (`icon-input-category.png`, etc.).
- **`src-tauri/resources/models/*.models`** — définitions JSON Line 6 (params, min/max, `displayType`, `valueType`, etc.) ; utilisées pour le panneau paramètres et le matching nom long Kempline ↔ nom court catalogue.
- **`src-tauri/resources/modules_by_id.json`** — carte **hex → [catégorie, nom long]** (référence pour croiser avec la machine / outils ; **pas** la source runtime de `MODULES_BY_ID`, qui vient uniquement du catalogue ci-dessous).
- **`src-tauri/resources/HelixControls.json`** — données controls (fichier ajouté au bundle ; brancher dans l’app si besoin).

Chemins côté front pour les PNG sous Tauri : souvent `/src-tauri/resources/...` (comme dans `models.ts` pour les I/O).

### Catalogue HX — `presetMeta`, `chainHex`, mono / stéréo (mémo session)

- Chaque modèle du JSON peut porter un objet **`presetMeta`** : notamment **`chainHex`** (une chaîne hex **ou** un tableau **`[mono, stéréo]`**) et **`signal`** en parallèle (`["mono", "stereo"]`) quand le même bloc existe en deux variantes.
- **`src-tauri/src/lib.rs`** : au build, **`MODULES_BY_ID`** est rempli **uniquement** depuis **`include_str!("../resources/HX_ModelCatalog.json")`** en parcourant tous les `presetMeta.chainHex` (chaîne ou tableau) + nom court du modèle. C’est cette table qui sert à résoudre l’UID hex du segment preset vers **catégorie + nom** affichés.
- **`src/hxModelCatalogMeta.ts`** : en dev, `fetch` du catalogue sous `/src-tauri/resources/HX_ModelCatalog.json`, map **catégorie + nom** → `presetMeta`, helpers **`pickSignal` / `pickChannel`** pour choisir le bon libellé quand `chainHex` / `signal` sont des tableaux parallèles au `module_hex` du slot.
- **`scripts/apply_mono_stereo_pairs_to_catalog.py`** : lit **`modules_by_id.json`**, regroupe les paires dont le nom long ne diffère que par **`(mono)`** / **`(stereo)`** / **`(stéréo)`** (même catégorie, même indice Guitar/Bass si présent), met à jour la fiche catalogue dont le **`name`** matche : `chainHex` + `signal` en tableaux. Dernière exécution indicative : **115** paires détectées, **103** fiches mises à jour, **12** sans correspondance catalogue (surtout **Dynamic** et **Vol/Pan** — noms de catégorie / modèle qui ne collent pas aux critères du script).
- **`scripts/enrich_catalog_preset_meta.py`** — autre script d’enrichissement `presetMeta` (heuristiques nom long, etc.), à lancer au besoin.
- **Travail restant (manuel)** : il reste de l’ordre de **~100** entrées **`"chainHex": ""`** dans **`HX_ModelCatalog.json`** (compter avec `rg '"chainHex":\\s*""' src-tauri/resources/HX_ModelCatalog.json`). Le goulot d’étranglement est d’**assigner le bloc sur la machine** pour récupérer l’hex, puis de recopier dans le JSON. Piste plus tard : script de **suggestion** hex depuis `modules_by_id.json` pour les cas évidents, et assouplir le mapping catégorie pour les **12** paires skippées si on veut les couvrir sans toucher à la machine deux fois.

**Note** : une copie **`External files/HX_ModelCatalog.json`** peut exister hors bundle ; elle n’a **pas** été incluse dans le commit local ci-dessous (diff très volumineux) — resynchroniser à la main si tu t’en sers comme miroir.

### Git — commits sans indexer les gros sous-dossiers de `resources/`

Pour préparer un commit **sans** inclure les changements sous `icons_category/`, `icons_models/`, `models/` (trop lourds ou générés ailleurs), depuis la racine du dépôt :

```bash
git add -A \
  ":(exclude)src-tauri/resources/icons_category" \
  ":(exclude)src-tauri/resources/icons_models" \
  ":(exclude)src-tauri/resources/models"

git status
git commit -m "Ton message"
git push origin refactor/multithread
```

Les fichiers **à la racine** de `src-tauri/resources/` (ex. `HX_ModelCatalog.json`) restent éligibles au staging s’ils sont modifiés. Ajoute d’autres `:(exclude)…` si tu dois aussi ignorer `External files/` ou autre.

**Commits / contexte** : sur la branche **`refactor/multithread`**, le commit **`f79be40`** reste la référence pour `preset_chain_params` + UI min | chaîne | max. Un commit local regroupe le catalogue (`HX_ModelCatalog.json`), `modules_by_id.json`, `HelixControls.json`, les scripts `scripts/*.py`, `hxModelCatalogMeta.ts`, les évolutions `lib.rs` / `models.ts` / `styles.css` et cette **`description.md`** (message **`feat(catalog): presetMeta chainHex/signal, MODULES_BY_ID depuis le JSON`** — voir **`git log -1`**).

## Reprise rapide après redémarrage

1. Lire **`README.md`** + ce **`description.md`**.
2. Lancer **`npm run tauri dev`** (ou `npm run dev` pour le front seul sur `http://localhost:1420`).
3. Pour toute modification UI models : **`src/models.ts`** + **`src/styles.css`** ; vérifier que **`models.ts` importe bien `./styles.css`** si tu travailles sur `models.html`.
4. Pour protocole / parsing preset / valeurs chaîne : **`src-tauri/src/lib.rs`** + **`preset_chain_params.rs`** + **`stomp_layout.rs`** + modules **`helix/`**.

Bon courage pour la suite.
