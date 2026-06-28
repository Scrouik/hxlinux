# Matrice — copier/coller, déplacer, cache params session

> **Statut :** fonctionnel en **v1** (move même path ; coller inter-preset / inter-path). Bugs terrain ouverts — voir §6.
>
> **Dernière révision :** juin 2026.
>
> **Backlog priorisé :** [`TODO.md`](../TODO.md) § Matrice.
>
> **Résumé reprise session :** [`description.md`](../description.md) (état actuel + liens).

Ce document décrit les **problèmes rencontrés**, les **solutions retenues** et l’**architecture** pour l’édition de preset dans la grille stomp (fenêtre Models, HX Stomp XL).

---

## 0. TL;DR pour qui reprend

- **Copier / coller** : menu contextuel sur un slot rempli → `probe_slot_model_usb` add + replay `write_live_param` ; pas de re-dump `preset_data`.
- **Déplacer** : **Pointer Events** sur la source (pas HTML5 DnD) ; flux = copier → coller → vider source (`remove`).
- **Params UI** : `preset_data` lu **une seule fois** au load preset → `slotChainSessionByKey` ; en session, `resolveChainValuesForKemplineSlot` (cache + overrides live + défauts `.models`).
- **Contraintes v1** : move uniquement **dans le même path** (0–7 ↔ 0–7, 8–15 ↔ 8–15), destination **vide**, pas I/O / Split / Merge. **Coller** : toute case vide FX (même path ou autre, preset courant quel qu’il soit).
- **Suite** : DnD inter-path (auto split/merge), DnD Split/Merge — règles métier dans TODO ; bugs DnD purge source / lecture preset à investiguer.

---

## 1. Fonctionnalité visée

Éditer un preset depuis la matrice 2×19 comme dans HX Edit, sans recharger tout le dump USB à chaque geste :

| Action | UX attendue |
|--------|-------------|
| Copier | Snapshot modèle + params du slot source |
| Coller | Assign USB sur case vide + params rejoués |
| Déplacer | Glisser un bloc vers une autre case vide du **même path** |
| Après move | Focus hardware déjà sur la destination ; UI alignée (sélection + panneau params) |

Hors scope v1 : save preset disque, insert dans chaîne pleine, move Path 1 ↔ Path 2, drag Split/Merge.

---

## 2. Problèmes rencontrés et solutions

### 2.1 Drag & drop — HTML5 DnD ne fonctionne pas sous Tauri / WebKitGTK

**Symptôme**

- `draggable="true"`, `dragstart` / `dragover` (avec `preventDefault()`) : le feedback visuel partiel fonctionnait.
- **`drop` ne se déclenchait jamais** sur Linux (WebKitGTK, Tauri 2).
- Logs temporaires `[dragover]` visibles, `[drop]` absent.

**Cause**

- Comportement connu de WebKitGTK : la phase drop de l’API HTML5 Drag and Drop est peu fiable dans les webviews embarquées, même avec `dropEffect` et capture corrects.

**Solution retenue : Pointer Events**

| Avant (abandonné) | Après |
|-------------------|--------|
| `draggable`, `dragstart`, `dragover`, `drop` sur cible | `pointerdown` / `pointermove` / `pointerup` / `pointercancel` sur la **source** |
| Cible = listener `drop` | Cible = `document.elementFromPoint(clientX, clientY)` au `pointerup` |
| — | `setPointerCapture` / `releasePointerCapture` pour garder le geste |

Implémentation : `bindMatrixSlotDragSource` dans `src/models.ts`.  
`bindMatrixSlotDropTarget` et `initMatrixDragDrop` sont des coquilles vides (logique entièrement côté source).

Feedback CSS : `node--matrix-draggable`, `node--matrix-drag-source`, `node--matrix-drag-over` (`src/styles.css`).

**Pourquoi ça marche**

- Pas de dépendance au pipeline DnD du navigateur : on traite un geste pointeur classique et on résout la cellule sous le curseur au relâchement.

---

### 2.2 Params incorrects ou vides après un move

**Symptôme**

- Juste après le move : panneau params parfois OK (clipboard).
- En quittant le slot puis en re-cliquant : sliders vides ou valeurs par défaut.

**Cause**

- Au clic slot, l’ancien code appelait `fetchSlotChainParamValuesReliable` : boucle jusqu’à ~14 s sur `get_active_preset_slot_chain_param_values` (parse `preset_data` côté Rust).
- Après probe/move, la **RAM preset du device** n’est pas encore à jour (ou la destination était vide au load) → lecture périmée ou vide.
- Un re-dump `request_preset_content` après chaque geste était trop lourd et instable.

**Solution : cache session + règle « preset_data une fois »**

| Moment | Source de vérité params FX (slots 0–15) |
|--------|----------------------------------------|
| Load / changement preset | `request_preset_content` puis `hydrateSlotChainSessionFromPresetData` |
| Clic slot, affichage panneau | `resolveChainValuesForKemplineSlot` (session + overrides live) |
| Assign picker / probe add | Défauts `.models` puis `write_live_param` → MAJ session |
| Move / coller | `replayMatrixClipboardParams` → `setSlotChainSessionValues` |
| Scroll modèle HW | Défauts catalogue + session (plus de `schedulePresetRamRefreshAfterHwModelScroll`) |

**Supprimé**

- `fetchSlotChainParamValuesReliable` (lecture répétée `preset_data` au clic).
- `pendingForceUsbPresetContent` après move.
- Re-dump preset systématique après scroll modèle HW.

**Note** : Input/Output/Split/Merge ne sont pas encore entièrement sur ce cache (FX slots 0–15 oui).

---

### 2.3 Focus UI après move

**Symptôme**

- Le hardware se positionne sur le slot destination après probe.
- L’UI restait sur l’ancienne sélection ou un panneau params incohérent.

**Solution**

- `focusMatrixSlotParamsPane(destKi)` en fin de `moveMatrixSlotFromTo` : sélection matrice + `switch_active_hardware_slot` + affichage params depuis le cache session.

---

### 2.4 Re-sélection du slot source vide après remove

**Symptôme**

- Pendant le move, `removeMatrixSlotFromCell` re-sélectionnait le slot source vide.

**Solution**

- `removeMatrixSlotFromCell(sourceKi, { reselect: false })` dans le flux move.

---

### 2.5 Purge HW source ignorée après DnD (slot 2 → 3)

**Symptôme**

- UI : seul le slot destination rempli.
- Hardware : **source et destination** encore pleins (doublon).

**Cause**

1. **Ordre paste → remove** : `pasteMatrixSlotToCell` bascule le focus USB sur la destination ; le bulk `remove` sur la source est alors souvent **ignoré** par le Stomp (l’UI optimiste vide quand même la source).
2. Bulk remove template : `slot_bus` patché mais pas l’octet **lane** après `83 66 cd 03` (contrairement à add/replace).

**Solution (juin 2026)**

| Changement | Fichier |
|------------|---------|
| Move : **copier → remove source → délai 150 ms → coller dest** (+ restauration auto si coller échoue) | `models.ts` `moveMatrixSlotFromTo` |
| Verrou UI `models-matrix-usb-busy` pendant toute l’opération (overlay + `pointer-events: none`) | `models.ts` + `styles.css` |
| Remove : `switch_active_hardware_slot` + délai 100 ms avant `probe_slot_model_usb` remove | `models.ts` `removeMatrixSlotFromCell` |
| `patch_kempline_lane_after_cd03_or_cd04` aussi pour `RemoveFromOccupied` | `edit_slot_model.rs` |

---

### 2.8 Soft-sync qui écrase la grille optimiste

**Symptôme**

- Après probe, un poll USB ou soft-sync pouvait re-parser `preset_data` périmé et annuler la ligne optimiste.

**Solution**

- `mergeProbeSlotModelUntil` (fenêtre ~20 s, multi-slot après move via `armProbeSlotMergeGrace`).
- `suppressUsbPresetPollUntilMs` (~10 s) après probe/move.
- Pas de `request_preset_content` immédiat après assign (log `no pendingForceUsbPresetContent`).

Voir aussi [`models-hardware-sync.md`](./models-hardware-sync.md) pour le contexte soft-sync / poll optionnel.

---

### 2.7 Presse-papiers perdu au changement de preset

**Symptôme**

- Copier un slot, changer de preset → « Coller » indisponible.

**Cause**

- `clearMatrixSlotClipboard()` au `models:load-preset` et `canPasteMatrixSlotToEmpty` exigeait même `presetIndex` et même `path`.

**Solution**

- Buffer conservé en mémoire jusqu’à déconnexion (`purgeModelsUi`).
- Coller sur toute case FX vide du preset actif (path 0 ou 1).
- Statut barre : `Copié : … (preset source)` / `Collé : … depuis …` si preset différent.

**Effet secondaire validé terrain :** le bug « copier sur plusieurs slots — UI OK, hardware pas à jour » était en pratique lié à la copie perdue au changement de preset ou au coller refusé hors même path (menu « Coller » inactif ou état incohérent) ; corrigé par ce changement.

---

### 2.6 Parse params Ampeg Scrambler (`cd0209`)

**Symptôme**

- Params du modèle `cd0209` mal décodés depuis `preset_data`.

**Cause**

- Le suffixe `09` de l’id modèle était confondu avec un token argument `1aff09` dans le parse Rust.

**Solution**

- Ancrage sur délimiteur `1aff` dans `extract_c219_argument_type_hexes` (`src-tauri/src/preset_chain_params.rs`).

---

### 2.9 Lecture preset bloquée après rafales D&D

**Symptôme**

- Après plusieurs moves sur un preset, changer de preset → lecture échoue (~90 %).

**Cause probable (pas un ACK dump en premier lieu)**

- Rafales `probe_slot_model_usb` laissent parfois `preset_content_only=true` **sans** `RequestPreset` actif (« session fantôme »).
- `request_preset_content` renvoyait alors `Ok(())` **sans relancer** le dump → sablier UI puis timeout.
- Changement de preset trop tôt après probe (collision USB) aggravait le symptôme.

**Solution (juin 2026)**

| Changement | Où |
|------------|-----|
| Reset `content_only` fantôme (garde si attente MIDI PC active) | `lib.rs` `request_preset_content` |
| Throttle 260 ms → `Err` + retry côté TS (plus de `Ok` silencieux) | `lib.rs` + `models.ts` |
| Attente fin verrou matrice + pause 400 ms post-probe avant dump | `models.ts` `requestLoadForPreset` |

Si le problème persiste : logs `[PresetDebug][request_preset_content]` et `force_recover_preset_reader` (déjà armé après timeout).

---

## 3. Architecture cache session

```
request_preset_content
        │
        ▼
hydrateSlotChainSessionFromPresetData(presetIndex)
        │  get_active_preset_slot_chain_param_values × 16
        ▼
slotChainSessionByKey  Map<"preset:slot", ChainParamValueJson[]>
        │
        ├─► clic slot ──► resolveChainValuesForKemplineSlot
        │                      ├─ session
        │                      ├─ liveChainParamOverridesByPresetSlot
        │                      └─ défauts .models (si session vide)
        │
        ├─► probe / coller / move ──► setSlotChainSessionValues
        │
        └─► write_live_param ──► overrides live (fusionnés à l’affichage)
```

Clé de stockage : `liveChainOverrideStorageKey(presetIndex, kemplineSlotIndex)`.

**Règle à ne pas violer** : ne pas rappeler `get_active_preset_slot_chain_param_values` au clic slot ou après probe/move pour « attendre » que le device rattrape — mettre à jour le cache session à la place.

---

## 4. Flux implémentés

### 4.1 Copier / coller

```
copyMatrixSlotFromCell
  └─ resolveChainValuesForKemplineSlot → matrixSlotClipboard

pasteMatrixSlotToCell
  └─ MAJ optimiste grille
  └─ loadAndShowModelsParamsFromCatalogDefaults
  └─ probe_slot_model_usb (add)
  └─ replayMatrixClipboardParams (write_live_param)
  └─ setSlotChainSessionValues
```

Contraintes coller : destination **vide** ; pas Input / Output / Split / Merge ; colonne non bloquée. **Inter-preset** et **inter-path** autorisés — le buffer survit au changement de preset (vidé à la déconnexion uniquement).

### 4.2 Déplacer

```
moveMatrixSlotFromTo(sourceKi, destKi)
  └─ copyMatrixSlotFromCell
  └─ removeMatrixSlotFromCell(source, { reselect: false })  ← avant coller (focus HW source)
  └─ pasteMatrixSlotToCell(dest)  ← restauration source si échec
  └─ armProbeSlotMergeGrace + suppressUsbPresetPoll
  └─ focusMatrixSlotParamsPane(dest)
```

Garde : `matrixMoveInFlight` (un move à la fois).  
Validation : `canMoveMatrixSlotToEmpty` (même path, dest vide, slot source copiable).

### 4.3 Résolution cible DnD

- `matrixDropTargetFromElement` : cellule `.node-empty.node--hx-slot` avec `data-kempline-slot-index`.
- Exclut colonnes bloquées (`.node-empty-column-blocked`).

---

## 5. Fichiers clés

| Fichier | Rôle |
|---------|------|
| `src/models.ts` | Clipboard, move, Pointer Events, cache session, replay params |
| `src/styles.css` | Curseur grab, états drag source / drag over |
| `src/hxModelCatalogMeta.ts` | Types alignement chaîne / picker |
| `src-tauri/src/preset_chain_params.rs` | Parse valeurs depuis dump preset |
| `src-tauri/src/lib.rs` | `probe_slot_model_usb`, `write_live_param`, invoke chain values |

Symboles utiles (recherche rapide) :

- `slotChainSessionByKey`, `hydrateSlotChainSessionFromPresetData`, `resolveChainValuesForKemplineSlot`
- `matrixSlotClipboard`, `copyMatrixSlotFromCell`, `pasteMatrixSlotToCell`, `moveMatrixSlotFromTo`
- `bindMatrixSlotDragSource`, `canMoveMatrixSlotToEmpty`, `focusMatrixSlotParamsPane`
- `mergeProbeSlotModelUntil`, `armProbeSlotMergeGrace`

---

## 6. Bugs terrain

### Corrigé (juin 2026)

| Bug | Cause probable | Fix |
|-----|----------------|-----|
| **Copier-coller multi-slot / hardware** | Copie perdue au changement de preset ; coller hors même path | Buffer persistant inter-preset / inter-path |
| **Source non vidée après D&D** | paste → remove, HX focalisé sur destination | remove avant paste + focus USB + verrou UI |

### Encore ouverts

Remontés dans [`TODO.md`](../TODO.md) § Matrice :

| Bug | Description | Correctif juin 2026 |
|-----|-------------|---------------------|
| **Lecture preset après D&D** | Après plusieurs moves, changement de preset échoue ~90 % | Reset `content_only` fantôme ; throttle → retry ; pause 400 ms post-probe — **à revalider** |

Repro conseillé : noter séquence exacte, activer `models_debug_sync_trace`, conserver logs `[MatrixMove]` / `[ProbeSlot]`.

---

## 7. Suite prévue (règles métier)

Détail et cases à cocher : [`TODO.md`](../TODO.md) § « À faire — drag & drop inter-path et structurel ».

### Move Path 1 ↔ Path 2

1. Si path 2 est vide et drop depuis path 1 : **création auto** d’un Split juste après Input et d’un Merge juste avant Output.
2. Interdit de drop inter-path sur un slot **avant** le Split ou **après** le Merge.

### Drag Split / Merge

1. Split déplaçable, mais **jamais après** le premier slot rempli du path 2.
2. Merge déplaçable, mais **jamais avant** le dernier slot rempli du path 2.
3. Comme pour un modèle FX : le bloc d’origine est **supprimé** après move réussi.

Fichiers à étendre en priorité : `canMoveMatrixSlotToEmpty`, `moveMatrixSlotFromTo`, trames focus / live write structurelles (`switch_active_hardware_special_slot`, `write_path1_split_type`, etc.).

---

## 8. Test manuel rapide

1. Charger un preset avec au moins 2 blocs sur le même path (ex. slots 1 et 3).
2. **Copier / coller** slot 1 → case vide : modèle + params visibles ; twist knob → live write OK ; re-clic slot → params toujours là (cache session).
3. **Move** slot 1 → autre case vide même path : source vide, dest remplie, focus UI sur dest.
4. Changer de preset puis revenir : valeurs re-hydratées depuis `preset_data`.
5. Sous Linux Tauri : vérifier que le drag fonctionne sans `drop` HTML5 (logs `[MatrixMove] pointerup`).

---

## 9. Liens

| Document | Contenu |
|----------|---------|
| [`TODO.md`](../TODO.md) § Matrice | Backlog, bugs, règles DnD inter-path |
| [`description.md`](../description.md) | État produit, conventions slot_bus |
| [`models-hardware-sync.md`](./models-hardware-sync.md) | Events USB, soft-sync, flags `localStorage` |
| [`captures/usb-wireshark/README.md`](../captures/usb-wireshark/README.md) | Captures pour bulk assign / comparaison HX Edit |
