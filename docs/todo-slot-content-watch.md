# TODO — Écouter le slot hardware actif (contenu)

**Objectif** : une fois le **slot actif** connu (`82 62 SS 1a` → `models:hardware-slot-changed`), détecter sur **ce slot** :

| Événement | Besoin UI |
|-----------|-----------|
| Changement de **modèle** | Mettre à jour nom / `moduleHex` / icône grille |
| **Slot vide** (suppression modèle) | Cellule vide, panneau params vidé |
| Modification des **paramètres** | Rafraîchir sliders / valeurs chaîne sans re-dumper tout le preset |

**État actuel (mai 2026)**

- ✅ **Sélection slot** HW → UI : `ingest_hw_slot_notify_in`, événement `models:hardware-slot-changed`, soft-sync sélection + panneau depuis `lastHwSyncNormalizedSlots`.
- ✅ **Focus USB** (OUT `cd:04`) : `sync_hardware_slot_focus_usb` → parse 36+44 o → `SlotFocusInCapsule` (`anchor12`, `ed_suffix7`) par index.
- ⚠️ **Contenu slot** : grille / params = surtout `preset_data` (dump complet) ou MAJ **optimistes** après `probe_slot_model_usb` (UI → device). **Pas** d’écoute device → UI pour modèle / params / vide sur le slot courant.

---

## Principe d’architecture cible

```text
IN 0x81 (listener)
  → slot actif connu (hw_active_slot_index)
  → filtrer trafic « concernant ce slot » (bus SS, cd:04, anchor, bulks assign…)
  → comparer empreinte vs dernière connue pour CE slot
  → emit models:slot-content-changed { slotIndex, kind, … }
Front
  → params : softRefreshParamsPaneFromSlots / fetch chaîne
  → grille : MAJ cellule ou renderSlots si layout (model) change
```

Ne pas revenir au dump preset à chaque tick (voir `description.md` §12 mai).

---

## Phases proposées

### Principe produit (accord mai 2026)

- **Changement de preset** → lecture preset → **écraser** toute la grille (18 col × 2 lignes, paths In/Out/Split/Merge/Slots).
- **Ensuite** : toute modif (UI ou HW) → MAJ **ciblée** via lecture USB temps réel du bloc concerné — **sans** repasser par `preset_data`.
- `preset_data` = buffer temporaire au **chargement** preset uniquement (parse initial), pas source de vérité pour la surveillance.

### Phase A — Empreinte IN (backend) — **implémenté**

- [x] `slot_watch_prev[16]` : empreinte **capsule** (`anchor12` + `ed_suffix7`) uniquement.
- [x] **Pas** de `layout_sig` / `params_sig` depuis `preset_data`.

### Phase B — Événement + front — **implémenté (partiel)**

- [x] `models:slot-content-changed` (`kind: "content"`, `capsuleSig`).
- [x] Poll `models_hw_slot_content_watch_ms` (défaut 1200 ms).
- [ ] Grille : decode IN → MAJ cellule (modèle / vide) — **phase C**.
- [ ] Params : decode IN chaîne — **phase C** (le panneau peut encore passer par `get_active_preset_slot_chain_param_values` / dump tant que decode IN absent).

### Phase C/D — Decode IN + grille 18 colonnes

- [ ] Captures HW (param, model, clear, In/Out/Split/Merge).
- [ ] Parser focus / assign → `GridState` sans relire `preset_data`.

### Phase C — Écoute passive IN (sans OUT focus à chaque fois)

- [ ] Analyser captures :
  - `src/Paquets Json/Model_change_slot1_Linux.json`
  - scénarios à capturer : idle sur slot, twist knob, add/remove model sur HW
- [ ] Étendre `usb_listener` ou `ingest_*` : sur IN long / `ed:03` / paires post-`82:62`, tenter parse ou déclencher focus différé sur slot actif seulement.
- [ ] Étendre `slot_focus_in.rs` : variante **`cd:03`** (Linux `switch_active_hardware_slot`) si utile.

### Phase D — Décodage contenu depuis capsule (sans dump)

- [ ] Corréler `anchor12` ↔ offset dans `preset_data` (déjà log verbose).
- [ ] Décoder modèle vide vs assigné depuis segment ou capsule (à définir après captures).
- [ ] `get_active_preset_slot_chain_param_values` : valeurs depuis IN si possible, pas seulement `Some([])`.

---

## Captures à produire (priorité)

| Fichier suggéré | Action sur le Helix |
|-----------------|---------------------|
| `…_slot_idle.json` | Slot 3 sélectionné, 60 s sans toucher |
| `…_hw_param_twist.json` | Même slot, tourner une knob |
| `…_hw_model_change.json` | Changer modèle sur le HW (pas via HXLinux) |
| `…_hw_slot_clear.json` | Vider le slot sur le HW |
| `…_hxedit_same_scenarios.json` | Mêmes actions sous HX Edit (référence) |

---

## Fichiers code (prévision)

| Fichier | Rôle |
|---------|------|
| `helix/slot_focus_in.rs` | Parse IN, empreintes, variantes `cd:03/04` |
| `helix/mod.rs` | État empreintes, ingest contenu |
| `helix/usb_listener.rs` | Emit `slot-content-changed` |
| `lib.rs` | Sync focus, commandes debug |
| `models.ts` | Listener, refresh ciblé |
| `preset_chain_params.rs` | Decode params si segment connu |

---

## Flags / tests existants

- `models_hw_slot_focus_usb` — focus OUT après notif slot (défaut actif).
- `models_hw_slot_focus_await_chain=1` — await sync avant chaîne.
- `models_hw_force_preset_dump_on_slot_notify=1` — **à éviter** comme solution permanente.
- Test Rust : `hxedit_slot_focus_preset_test_reference`, `slot_focus_in` unit test.

---

*Créé mai 2026 — suite logique après sync slot hardware → UI.*
