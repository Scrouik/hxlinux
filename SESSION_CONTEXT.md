# Session Context (HXLinux)

## Date
2026-04-16 (session : affichage preset / models + commit local)

## Objectif du moment
Fiabiliser l’affichage des blocs d’un **preset actif** dans la fenêtre **models** (grille Kempline, routage Split/Merge), pour se rapprocher de HX Edit et pouvoir **déboguer preset par preset** demain (sniff + captures).

## État actuel du code (à relire demain sans réexpliquer)

### UI models (`src/models.ts`, `src/styles.css`)
- Grille **2 lignes × 19 colonnes** pour les presets au format Kempline 16 cases :
  - col. 1 : `INPUT` (ligne 1)
  - col. 2 : **split** si `splitAfterCol === 0` (split « au départ »)
  - cols 3, 5, …, 17 : slots **Path 1A** (ligne 1) et **Path 1B** (ligne 2 si branche B non vide)
  - cols 4, 6, …, 18 : **split** ou **merge** sur la ligne 1 selon les frontières 1…8 (`splitAfterCol` / `mergeAfterCol`)
  - col. 19 : `MAIN L/R` (ligne 1)
- Pas de traits / spine entre nœuds pour l’instant (volontaire).
- **Important** : les positions split/merge ne dépendent plus seulement de `get_active_preset_routing_markers` (souvent vide). On utilise **`get_active_preset_stomp_layout`** quand `kemplineGridOk`, sinon la même **heuristique d’occupation** que le Rust (`computeRoutingJunctionColumns` ↔ `split_merge_from_occupancy`). L’UI routing s’active si marqueurs **ou** branche B avec au moins un bloc (`showRoutingUi` / `hasBranchB`).

### Backend (`src-tauri/src/stomp_layout.rs`, `lib.rs`, `usb_monitor.rs`)
- Module **`stomp_layout`** : grille 16 segments, `split_after_col` / `merge_after_col`, chaîne stomp.
- Commande Tauri **`get_active_preset_stomp_layout`** branchée côté `models.ts`.
- Ajustements USB / preset (voir diff du commit ci-dessous).

### Commit local
- Branche : **`refactor/multithread`**
- Hash : **`008015e`** — message : *Preset models: matrice Kempline 2×19 et layout stomp côté Tauri*
- Fichiers typiques du lot : `src/models.ts`, `src/styles.css`, `src-tauri/src/stomp_layout.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/helix/usb_monitor.rs`, `src/main.ts`, `index.html`, `TODO.md`, suppression `CLAUDE.md`.

## Plan prochaine session
- Analyser **preset par preset** ceux dont l’affichage est incorrect.
- **Sniffer** le flux USB / binaire et recouper avec `split_after_col`, `merge_after_col`, `grid_x`/`grid_y` quand dispo.
- Affiner règles parseur ou mapping UI au fil des cas réels.

## Sauvegardes « anti-perte »
- **Contexte texte (ce fichier)** : reprise indépendante de l’historique Cursor.
- **Backup état Cursor** (DB / historique app) : depuis la racine du repo  
  `./backup_cursor_state.sh`  
  Option plus lourde : `./backup_cursor_state.sh --workspace`
- Les échanges longs peuvent aussi être archivés dans le repo (ex. `Export Echange.md`).

## Fichiers non versionnés (hors dernier commit)
`agent.md`, `CURSOR_RECOVERY.md`, `Export Echange.md`, `histo.md`, etc. — non inclus dans `008015e` ; les ajouter au git seulement si tu veux les versionner.
