# HXLinux — description pour reprendre une session

Ce fichier sert de **mémo locale** quand l’historique de chat ou le contexte IDE est perdu après un redémarrage. Il complète le `README.md` (objectifs produit et commandes de base). Dernière passe de contenu significative : **avril 2026** (matrice stomp 4×20, icônes jonction / lien, notes REVERT et Rust `stomp_layout`).

## À quoi sert l’application

**HXLinux** est un éditeur / explorateur de presets pour **Line 6 HX Stomp XL** (et IDs USB voisins listés dans le code), sur **Linux**, en application **desktop Tauri** (Rust + front web).

Fonctions déjà utiles en pratique :

- Connexion **USB** au boîtier, machine d’états côté Rust pour le protocole (inspiré du travail **Kempline / helix_usb**).
- Lecture des **125 noms de presets**, **activation** d’un preset (Program Change), **renommage** depuis l’UI.
- Chargement du **contenu binaire du preset actif**, parsing partiel en **« slots »** (catégorie + nom) pour l’affichage.
- Mise en page type **grille** (16 blocs + routage), données renforcées par **`stomp_layout`** (split/merge, grille USB quand dispo).

Ce qui reste largement ouvert : décodage profond des paramètres, édition temps réel, export/import de fichiers presets (voir `README.md`).

## Stack technique

| Couche | Rôle |
|--------|------|
| **Rust / Tauri 2** | USB (`rusb`), threads listener/écriture, état `HelixState`, commandes `invoke` exposées au front. |
| **TypeScript + Vite 6** | UI : `src/main.ts` (liste presets + intégration workspace), `src/models.ts` (vue « chaîne / grille » des blocs du preset). |
| **CSS** | `src/styles.css` — styles partagés ; la page `models.html` importe aussi ce fichier via `models.ts`. |

Build front : `npm run build` (`tsc` + `vite build`). App complète : `npm run tauri dev` / `npm run tauri build`.

## Structure des dossiers (utile au quotidien)

```
hxlinux/
├── index.html              # Fenêtre principale : liste + panneau « HX Models » (même document que main.ts)
├── models.html             # Entrée Vite secondaire (build MPA) ; utile si tu ouvres cette page seule en dev
├── src/
│   ├── main.ts             # Liste presets, statut, drag/rename, appels invoke vers Rust
│   ├── models.ts           # Rendu grille / chaîne preset, polling get_active_preset_slots, stomp_layout
│   └── styles.css          # Tout le look `.models-pane`, matrice `hx-matrix-*`, nœuds `node--hx-slot`, etc.
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs          # Commandes Tauri, AppState, parse_preset_slots*, lancement USB monitor
│   │   ├── stomp_layout.rs # Layout stomp + routing (split/merge cols) aligné USB / heuristiques
│   │   └── helix/          # Protocole : modes (connect, request preset(s), standard…), USB, paquets
│   ├── resources/          # Bundlé : HX_ModelCatalog.json, icons_models/, icons_category/
│   └── tauri.conf.json     # devUrl 1420, ressources bundle
└── README.md               # Statut produit, prérequis, crédits Kempline
```

## Deux surfaces front pour les « models »

1. **Dans la fenêtre principale** (`index.html`) : section `.models-pane` avec `<main class="models-content" id="content">`. **`main.ts` et `models.ts` sont tous les deux chargés** sur cette page ; `models.ts` attache son UI à `#content` / `#status` / `#preset-label` **du panneau droit** (attention aux `id` dupliqués si tu dupliques des fragments HTML).
2. **`models.html`** : page dédiée au build Vite ; `models.ts` y importe `./styles.css`. Le `<main id="content" class="content models-pane">` sert à activer les sélecteurs `.models-pane` / `#content.models-pane` (layout matrice, largeur grille, etc.).

En dev Tauri, ce qui compte le plus est souvent **index + models.ts** dans le même WebView.

## Rust — commandes exposées (`invoke`)

Déclarées dans `lib.rs` (`tauri::generate_handler![...]`), typiquement utilisées par le front :

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
| `get_preset_data_hex` | Dump brut hex (debug). |
| `request_active_preset_name` | Resync nom preset actif. |

Le flux côté `models.ts` : après changement de preset → `request_preset_content` → boucle d’attente → `get_active_preset_slots` + routing + `get_active_preset_stomp_layout` pour `renderGrid16`.

## Fichiers Rust à connaître pour le preset / UI grille

- **`lib.rs`** — `parse_preset_slots`, `parse_preset_slots_internal`, orchestration preset ; commentaires `[PresetDebug]` dans les logs.
- **`stomp_layout.rs`** — `split_merge_from_usb_preset_body`, `compute_stomp_layout_from_kempline_grid_with_usb`, tests ; colonnes split/merge consommées par `models.ts`. Le helper `merge_after_col_from_usb_preset_body` n’existe qu’en build test (`#[cfg(test)]`) pour éviter un warning `dead_code` en `cargo build` lib.

## Front — matrice stomp 4×20 (`renderGrid16` dans `models.ts`)

Grille **20 colonnes × 4 lignes**, cellules **56×56 px** (`NUM_COLS = 20`, `NUM_ROWS = 4`, `CELL_PX = 56`). Nomenclature des lignes dans le code :

| Ligne | Rôle |
|-------|------|
| **L1** | Path 1 — slots 0–7, I/O Input / Output, traits horizontaux `Icons_line.png` entre colonnes paires, pastille `Icons_split_merge.png` aux colonnes **jonction** (split/merge issus du routing). |
| **L2** | Description Path 1 — textes catégorie ; aux colonnes split/merge, petite barre verticale `Icons_vertical_line.png`. |
| **L3** | Path 2 — slots 8–15 si branche B ; aux mêmes colonnes, icônes coin **`Icons_link_split.png`** / **`Icons_link_merge.png`** (alignées sur `stomp_layout`). |
| **L4** | Description Path 2 — catégories path B. |

- **Colonne 20** : numéros de ligne grille (debug lisible).
- **Colonnes « jonction »** : dérivées des frontières split/merge (1…8) via `matrixEvenColForRoutingBoundary` (colonnes paires 2…18 côté UI).
- **`ENABLE_MATRIX_VSPAN_ON_PATH2`** (`models.ts`) : par défaut **`false`**. Un overlay `vspan` vertical sur Path 2 partageait la même `grid-area` que les icônes lien ; les deux se superposaient. Laisser à `true` uniquement pour un revert visuel expérimental (commentaires **REVERT** à côté).
- **Ancienne mise en page (5 lignes + rangée 3 « séparateur » 0 px)** : le retour est documenté en blocs commentés **REVERT** dans `models.ts` et `styles.css` (constantes de lignes, hauteurs de rangées, boucle séparateur, classes `row-line-debug-sep`, etc.).

Le CSS associé est sous **`.models-pane .hx-matrix-*`** dans `styles.css`. Des régressions visuelles passent souvent par : parent sans `.models-pane`, ou styles inline dupliqués dans `models.html` vs `styles.css`.

## Ressources et métadonnées Line 6

- **`src-tauri/resources/HX_ModelCatalog.json`** — catalogue modèles.
- **`src-tauri/resources/icons_models/`** — icônes par modèle.
- **`src-tauri/resources/icons_category/`** — icônes catégories + assets maison pour la matrice : `Icons_line.png`, `Icons_split_merge.png`, `Icons_vertical_line.png`, `Icons_link_split.png`, `Icons_link_merge.png`, ainsi que les icônes I/O (`icon-input-category.png`, etc.).

Chemins côté front pour les PNG sous Tauri : souvent `/src-tauri/resources/...` (comme dans `models.ts` pour les I/O).

## Reprise rapide après redémarrage

1. Lire **`README.md`** + ce **`description.md`**.
2. Lancer **`npm run tauri dev`** (ou `npm run dev` pour le front seul sur `http://localhost:1420`).
3. Pour toute modification UI models : **`src/models.ts`** + **`src/styles.css`** ; vérifier que **`models.ts` importe bien `./styles.css`** si tu travailles sur `models.html`.
4. Pour protocole / parsing / split-merge USB : **`src-tauri/src/lib.rs`** + **`stomp_layout.rs`** + modules **`helix/`**.

Bon courage pour la suite.
