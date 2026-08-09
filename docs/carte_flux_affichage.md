# Carte du flux d'affichage des données (models.ts) — état 2026-08-10

But : comprendre le flux actuel « dump device → valeurs affichées » pour concevoir un
**modèle d'état unique** et retirer la couche de caches qui pourrit (courses, `preset_data`
périmé, figement du model par défaut).

> Portée : on assainit la **couche d'état d'affichage**. On NE touche PAS aux décodeurs /
> protocole USB / cas legacy-cab-dual-split-snapshot (sains et validés device).

---

## 1. Source de vérité device : `preset_data` (RAM backend Rust)

Dernier dump du preset lu depuis le HX. Exposé au frontend par des commandes Tauri (toutes
= un **parse de `preset_data`**) :

| Commande | Donne | Cache frontend cible |
|----------|-------|----------------------|
| `get_active_preset_slots` / `_debug` | modèle par slot (grille 16) | `lastHwSyncNormalizedSlots` |
| `get_active_preset_slot_chain_param_values` | **valeurs des params** par slot (les knobs) | `slotChainSessionByKey` |
| `get_active_preset_slot_dual_parts` | parties amp/cab (dual) | `slotDualPartsSessionByKey` |
| `get_active_preset_routing_markers` | split/merge | (overrides split/merge) |
| snapshot values / names / controller assignments | params snapshot + noms + contrôles | `snapshotParamValuesCache`, `snapshotNamesCache`, `controllerAssignmentsCache` |

⚠️ **`preset_data` n'est PAS mis à jour par un édit live** (le knob change en RAM device via
`write_live_param`, mais le backend `preset_data` garde l'ancien dump jusqu'à un **nouveau dump**).
Donc tout parse de `preset_data` après un édit = **périmé**. Idem après un SAVE (pas de re-dump).

---

## 2. Caches frontend (état module-level) — LA zone à assainir

| Cache (models.ts) | Clé | Rôle | Écrit par | Lu par |
|---|---|---|---|---|
| `slotChainSessionByKey` (388) | `${preset}\|${slot}` | valeurs de chaîne « de travail » par slot | hydrate (581), setSlotChainSessionValues (528/559), move (13088) | resolveChainValues (962), dual (2711), 1108, 14650 |
| `liveChainParamOverridesByPresetSlot` (385) | `${preset}\|${slot}` → Map<symbolicId,val> | overrides d'édits live (globaux ; params snapshot seedés par snap actif) | édit live (437/440) | resolveChainValues (969), mergeLiveChainOverrides (999) |
| `slotDualPartsSessionByKey` (404) | `${preset}\|${slot}` | parties dual amp/cab | hydrate dual | rendu dual |
| `flowIoChainSessionByKey` (3009) | idem | flow I/O | hydrate flow | rendu I/O |
| `snapshotParamValuesCache` (6516) | liste | params source « Snapshot » + 4 valeurs | loadSnapshotParamValues (dump) | seeding overrides snapshot |
| `lastHwSyncNormalizedSlots` (1032) | — | derniers slots rendus (SlotDebug[]) | renderSlots | focusMatrixSlotParamsPane (slot du panneau) |
| `activeSnapshotIndex` (420) | — | snapshot actif 0..3 | bascule / loadActiveSnapshot | seeding, rendu |
| `currentPresetIndex` (143) / `loadedPresetIndex` (144) | — | preset courant / dernier chargé | load flow | partout |
| `selectedParamsKemplineSlotIndex` (5162) / `selectedParamsPresetIndex` (5161) | — | slot dont le panneau params est ouvert | focus / clear | refresh conditionnel |

**Résolution d'une valeur affichée** (`resolveChainValuesForKemplineSlot`, ~962) :
`base` = `slotChainSessionByKey[preset|slot]` (sinon défauts catalogue) **OVERLAY**
`liveChainParamOverridesByPresetSlot[preset|slot]`.

---

## 3. Flux de CHARGEMENT (requestLoadForPreset, ~14160)

1. Garde-fous (loading, cooldown, init settling).
2. `clearSelectedParamsContext()` → `selectedParamsKemplineSlotIndex = null`.
3. Lecture dump (via `request_preset_content` / poll) → `preset_data` backend rafraîchi.
4. `renderSlots(normalizedSlots)` → grille + `lastHwSyncNormalizedSlots`.
5. Rechargement caches : controllers → snapshot values → snapshot names → snapshot actif.
6. Hydratation valeurs : `hydrateSlotChainSessionFromPresetData(index)` (remplit
   `slotChainSessionByKey` par `get_active_preset_slot_chain_param_values`, **une fois**), + dual + flow.
7. Auto-sélection du **model par défaut** → `focusMatrixSlotParamsPane` (rend le panneau).

**⚠️ COURSES / RUSTINES (le cœur du bug refresh) :**
- **Rendu AVANT hydratation** : « le panneau s'est ouvert avant l'hydratation (cache vide →
  valeurs par défaut) » (14392). Un refresh post-hydratation existe MAIS conditionnel :
  `if (selectedParamsKemplineSlotIndex !== null …)` — or il a été mis à `null` à l'étape 2 →
  souvent PAS de refresh → **model par défaut figé**.
- **Branche `if (hardwareRefresh)` (14388) ne rafraîchit JAMAIS le panneau** (juste ré-hydrate).
- Le clic sur un AUTRE model rend APRÈS hydratation → frais (d'où « re-clic OK »).

---

## 4. Flux de RENDU d'un panneau (focusMatrixSlotParamsPane, ~5413)

`focusMatrixSlotParamsPane(slot)` → `slot = lastHwSyncNormalizedSlots[slot]` →
`loadAndShowModelsParamsForSlot` → `resolveChainValuesForKemplineSlot` → `appendModelsParamRows`
(les knobs `<input type=range>`, valeur = `snapRawToIncrement(sliderCv, …)`).

Auto (model défaut) ET clic passent par le **même** chemin → la différence est **purement le
timing** (hydratation faite ou non, refresh déclenché ou non).

---

## 5. Flux d'ÉCRITURE (édit live)

Knob bougé → `write_live_param` (device RAM) + `recordLiveChainParamOverrideForKemplineSlot`
(437/440 → `liveChainParamOverridesByPresetSlot`). **`preset_data` backend NON mis à jour.**
SAVE (`save_preset_to_hardware`) → flash device. `models:preset-saved` (15121) = **seulement
`resetPresetModified()`** → **aucune relecture** (HX Edit, lui, relit après save).

---

## 6. Symptômes expliqués par cette carte

- **Model par défaut figé après save** : rendu avant hydratation + refresh conditionnel raté
  (selectedParams=null) → panneau garde les valeurs du preset quitté ; re-clic (post-hydratation) OK.
- **Preset vide affiche le précédent** : même famille (panneau/grille pas re-rendus depuis un
  état frais quand le nouveau est vide).
- **Snapshot** : couche seeding (`snapshotParamValuesCache` → overrides du snap actif) empilée
  par-dessus, encore un cache à synchroniser → fragile.
- **Accès `preset_data` périmé** : parses `get_active_preset_slots` post-édit (voir commentaires
  « stale preset_data parse » 1933, 5317) — à auditer (dont accès introduits par Cursor).

---

## 7. Cible : MODÈLE D'ÉTAT UNIQUE (proposition)

Un seul objet autoritaire, reconstruit **systématiquement depuis le dump frais** à chaque
chargement ET après save (relecture façon HX Edit) :

```
CurrentPreset = {
  index,
  activeSnapshot,               // 0..3, décodé du dump (0x5c)
  slots: [{ kemplineIndex, model, dualParts, params: [{ symbolicId, value }] }],
  snapshots: [{ name, valuesBySymbolicId }],   // params source « Snapshot »
}
```

Règles :
1. **Une seule source** : tout le rendu (grille + panneau, auto ET clic) lit `CurrentPreset`.
   Suppression progressive de `slotChainSessionByKey` / `liveChainParamOverridesByPresetSlot` /
   `snapshotParamValuesCache` séparés (fusionnés dans le modèle).
2. **Reconstruit du dump** à chaque load + après save (relire, comme HX Edit). Jamais de rendu
   « avant hydratation ».
3. **Édit live** = met à jour `CurrentPreset` (et device via `write_live_param`) — pas de cache d'override séparé.
4. **Zéro parse de `preset_data` en runtime** hors reconstruction (bannir les accès périmés).
5. Décodeurs / protocole / cas legacy = **inchangés**.

Migration par paliers validés device : (a) introduire `CurrentPreset` peuplé en parallèle des
caches ; (b) migrer le rendu panneau à le lire ; (c) migrer grille/dual/flow ; (d) supprimer les
anciens caches ; (e) brancher la relecture post-save. À chaque palier : tester device.

---

## Fichiers / points d'ancrage
- `src/models.ts` : caches §2, load §3 (~14160/14388), rendu §4 (~5413/10370/10917), écriture §5.
- Backend commandes §1 : `src-tauri/src/lib.rs` (get_active_preset_*), `preset_chain_params.rs`.
- Save : `src-tauri/src/helix/preset_label.rs` (commité `cb2c106`, lane editor).
