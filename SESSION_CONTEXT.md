# Session Context (HXLinux)

## Date
2026-05-23 — session : **sync scroll modèle HW** (molette Stomp → pull USB → UI models)

## Objectif du moment
Quand l'utilisateur tourne la molette modèle sur le Helix, HXLinux doit afficher le **bon modèle + params** sans `request_preset_content` à chaque pas — comme HX Edit : **1 pull host par `1f`**, lane scroll `hw_model_scroll_ack_ctr` alignée, pas de freeze Stomp.

**Test stress** : scrolls rapides répétés. **Succès** : pulls `0x1f` fiables, pas de quarantaine/boucle, grille preset OK au connect.

**Debug** : `HX_SLOT_MODEL_HW_PULL_DEBUG=1 npm run tauri dev`  
**Doc détaillée** : `description.md` (§ 22 mai 2026), `docs/models-hardware-sync.md`

---

## Fichiers centraux

| Fichier | Rôle |
|---------|------|
| `src-tauri/src/helix/slot_model_hw_pull.rs` | Pull `1b`+`f0` → echo → `19`×2 → bulk 272 ; ACK `1d`/`1f`/`21` ; settling ; pending ; quarantaine |
| `src-tauri/src/helix/mod.rs` | `hw_model_scroll_ack_ctr`, `hw_model_scroll_ack_step`, `hw_model_pull_pending_*`, compteurs pull |
| `src-tauri/src/helix/usb_listener.rs` | Ordre : `ack_hw_model_scroll_in` **puis** `ingest_slot_model_hw_in` |
| `src-tauri/src/helix/modes/standard.rs` | Guards pull/settling sur ACK parasites `0x17`/`0x21` preset |
| `src-tauri/src/helix/preset_dump_stream_ack.rs` | ACK chunks 272 (`80:10:ed:03`, lane dump — pas scroll) |

**Captures Wireshark** (`captures/usb-wireshark/`) : `3_scroll_HXEdit.json`, `3_scroll_problem_Linux.json`, `multi_scroll_HXEdit.json`, `Flood_connection.json`, `scroll_problem_HXLinux.json`

---

## État des correctifs (non commités au 23/05)

### Appliqués dans le working tree

| Patch | Statut | Effet attendu |
|-------|--------|----------------|
| **ACK `21` post-assign** (`+0x0019`, pas `+0x0015`) | ✅ Appliqué | HX Edit envoie `f0 sub=08` après chaque IN `21` 44 o post-pull ; l'ignorer faisait dériver la lane d'un `0x19` par cycle |
| **Gate post-settle `1d`** (`arm_post_settle_firmware_1d_gate`) | ✅ Appliqué | Après `post-pull settling terminé`, ignorer rafale firmware `1d` sans ACK jusqu'au prochain `1f` molette |
| **Compteur `hw_model_pull_pending_count`** | ✅ Appliqué | Ne plus écraser un seul `pending` quand plusieurs `1f` arrivent pendant pull/settling |
| Guards `standard.rs` (pas d'ACK dump en enveloppe scroll pendant pull/settling) | ✅ (session antérieure) | Fini les doubles `f8:XX` parasites sur lane scroll |
| Quarantaine deux phases, pas d'ACK `1f` step 1, retry `+0x45`, etc. | ✅ (sessions antérieures) | Voir `description.md` |

### Non appliqué / à valider HW

| Sujet | Notes |
|-------|--------|
| **Off-by-one `pending_count`** | Décrément dans `finalize_pull_capture` : simulé **N scrolls → N−1 pulls** pour N≥3. Fix proposé : décrémenter dans `after_post_pull_settling_expired` au flush, pas dans finalize |
| **`1f` jeté vs file en settling** | Code actuel **met en file** + `pending_from_scroll` ; 2 tests unitaires obsolètes échouent encore |
| **Rafale `1d` au connect** | Gate post-settle aide ; risque de 2–3 ACK avant la gate si timing serré |
| **Init `hw_model_scroll_ack_ctr`** | `HelixState::new()` : vérifier alignement cold boot `0x1009` vs connect handshake (`description.md` § bug ouvert) |

---

## Diagnostic chiffré (capture `3_scroll_problem_Linux.json`)

- **16** scrolls utilisateur (`1f` IN)
- **16** changements modèle Stomp (`21` IN)
- **11** pulls émis par HXLinux → **5 scrolls perdus**, UI ~5 modèles en retard

**HX Edit** (`3_scroll_HXEdit.json`, `multi_scroll_HXEdit.json`) : **53 `1f` = 53 pulls**, sans exception.

**Cause racine scrolls perdus** : `hw_model_pull_pending_slot_bus: Option<u8>` ne gardait qu'un slot ; le 2ᵉ `1f` écrasait le 1er sans pull pour celui-ci.

---

## Politique ACK scroll (rappel)

| IN | Comportement host actuel |
|----|-------------------------|
| **`1f` → pull imminent** | **Pas** d'ACK scroll avant `1b` (lane figée pour f0 interstitiel) |
| **`1d` molette** | ACK différé ou flush avant pull ; ignoré post-settle / rafale firmware |
| **`21` post-assign** (44 o, `82:69:27:6a` + `82:62:xx:1a`) | **ACK** `f0 sub=08`, step **`+0x0019`** si `prev=1f` (ancien commentaire « unidirectionnel » = **faux** vs captures) |
| **`1d`/`1f` pendant settling 272** | Pas d'ACK (lane figée, HX Edit) |
| **`1d`/`1f` pendant pull actif step 1** | `1f` en file sans ACK |

**Pull** : seul **`1b` + f0 interstitiel** en rafale au départ ; les **`19`** partent **seuls** (pas de `f0` derrière) ; le `+0x2e` manquant après `19` #1 est simulé par `advance_scroll_ack_after_pull_interstitial_f0`.

---

## Points bloquants / symptômes observés

1. **Connect sans scroll** : rafale IN `1d` firmware (`0x10`→`0xf5+`) après preset + settling ; ACK partiels → lane `hw_model_scroll_ack_ctr` désync (`Flood_connection.json` : 242 IN `1d` / 147 OUT ACK).
2. **Scroll après ~4 pas ou au wrap `cd:04`** : rejets `XX:04`, timeout, quarantaine (lane trop avancée ou pas assez selon les cas).
3. **Scroll rapide** : UI en retard du Stomp (pending unique → compteur en cours de fix).
4. **Lane scroll vs preset** : trois compteurs distincts — ne jamais mélanger `preset_dump_ack_ctr` / `editor_ed03_double` / `hw_model_scroll_ack_ctr`.

---

## Solutions envisagées (priorité)

| Priorité | Action | Risque |
|----------|--------|--------|
| **P0** | Valider HW patch **`21` ACK +0x19** isolément | Faible ; logs `ACK OUT … IN 0x21` après chaque pull OK |
| **P0** | Corriger **off-by-one `pending_count`** (décrément au flush) | Faible ; test unitaire 5 scrolls → 5 pulls |
| **P1** | Retest connect : gate post-settle + **zéro** `ACK OUT sub=08` sur rafale `1d` avant 1er scroll | Moyen |
| **P1** | Aligner init lane scroll **`0x1009`** + handshake connect | Moyen |
| **P2** | Mettre à jour tests `rapid_1f_after_finalize…`, `post_pull_settling_ignores_1f…` | Faible |
| **P2** | `(Some(0x21), 0x21)` step `0x0015` vs `0x0019` — seulement si observé en capture | — |

**Volontairement pas touché cette itération** : machinerie quarantaine/settling complète, debounce temps sur `1f`, refonte file d'attente autre qu'`pending_count`.

---

## Logs clés à surveiller

```
post-pull settling terminé (plus de 272 dump)
post-settle — ignore 1d firmware jusqu'à 1f utilisateur
ACK OUT f0:03 sub=08 pour IN 0x21
pull en file slot_bus=… count=N
pull réussi — pending_count restant=N
pull slot_bus=… depuis notif 0x1f
écho interstitiel rejet Stomp (XX:04)
quarantaine pull — attendre 1d+1f frais
```

**Bon signe connect** : pas de `ACK OUT sub=08` entre fin settling et premier scroll utilisateur ; premier scroll → `pull … depuis notif 0x1f`.

---

## Tests unitaires

```bash
cd src-tauri && cargo test slot_model_hw_pull
```

- **55 tests** visés ; **2 échecs connus** (sémantique settling / file `1f`, pas liés au patch `21`) :
  - `rapid_1f_after_finalize_ignored_during_settling_not_immediate_pull`
  - `post_pull_settling_ignores_1f_then_drops_pending_without_pull`

---

## Contexte antérieur (hors sujet actuel)

Session **2026-04-16** : grille models Kempline 2×19, `stomp_layout.rs`, commit `008015e` sur `refactor/multithread` — **pas le focus de la session USB scroll**. Voir anciennes sections dans l'historique git si besoin.

---

## Sauvegardes « anti-perte »

- **Ce fichier** + `description.md` § 22 mai 2026
- Transcript Cursor : agent `4a303e11-e91a-49b6-b045-ad8d3a4184ff`
- `./backup_cursor_state.sh` pour l'état IDE

**Git** : changements scroll HW **non commités** au 23/05 — ne pas committer sans demande explicite.
