# HXLinux — description pour reprendre une session

Ce fichier sert de **mémo locale** quand l’historique de chat ou le contexte IDE est perdu après un redémarrage. Il complète le `README.md` (objectifs produit et commandes de base).

---

## 22 mai 2026 — Pull modèle HW (scroll molette Stomp) — **session en cours**

**Objectif** : quand l’utilisateur change de modèle sur le Helix (molette / scroll), HXLinux affiche le bon modèle + params **sans** `request_preset_content` à chaque pas. Le host doit **tirer** le bloc assignable USB (`chainHex` / `module_hex`) via le protocole observé sur HX Edit.

**Branche / fichiers centraux** :

| Fichier | Rôle |
|---------|------|
| `src-tauri/src/helix/slot_model_hw_pull.rs` | Pull `1b`+`f0` → IN ~92 o → `19`×2 → IN ~272 o ; parse `module_hex` ; settling post-pull |
| `src-tauri/src/helix/mod.rs` | `hw_model_scroll_ack_ctr`, `editor_ed03_double`, `hw_model_pull_ctr` |
| `src-tauri/src/helix/usb_listener.rs` | Ordre : `ack_hw_model_scroll_in` **puis** `ingest_slot_model_hw_in` |
| `src-tauri/src/helix/usb_writer.rs` | Gap 14 ms entre `ed:03` ; exemption ACK dump 272 |
| `docs/models-hardware-sync.md` | Soft-sync UI, settling, pas de dump pendant scroll |

**Debug** : `HX_SLOT_MODEL_HW_PULL_DEBUG=1` (+ optionnel `HX_PRESET_DUMP_STREAM_ACK_DEBUG=1`).

### Flux pull (aligné HX Edit)

1. IN **`1d`** puis **`1f`** 40 o (`f0:03:02:10`, `09:02` aux octets 12–13) — souvent **même lot USB**.
2. **Un seul** pull sur **`1f`** (pas sur `1d`). Le `1f` « None » (`05:79:0e:6a`) = pas de pull.
3. Host : **`1b`** 36 o + court **`f0:03` sub=08** 16 o (rafale, pas de délai).
4. IN echo 16 o puis bulk **~92 o** (`53`…, `19…1a` = `module_hex`).
5. Host : **`19`** #1 → IN ~68 o → **`19`** #2 → IN **~272 o** final (obligatoire avant finalize).
6. UI : catalogue + défauts `.models` ; event `models:slot-model-changed`.

**Ne pas** envoyer `1b`+`19`+`19` d’un bloc avant la réponse IN au `1b` (le Stomp ne renvoie pas les bulks).

### Trois compteurs distincts (ne pas mélanger)

| Lane | Champ Rust | Octets 12–13 (LE) | Usage |
|------|------------|-------------------|--------|
| **Preset / éditeur** | `editor_ed03_double` | ex. `f1:64`, `05:64` | OUT `1b` / `19` pull ; +3 sur octet bas par `1b` ; hi souvent **`0x64`** |
| **Scroll ACK** | `hw_model_scroll_ack_ctr` | ex. `35:10`, `8f:10` | OUT `f0:03` sub=08 (ACK `1d`/`1f` + **f0 interstitiel** pull) ; hi souvent **`0x10`–`0x14`** |
| **Pull CTR** | `hw_model_pull_ctr` | ex. `3f:41` | Lane séparée sur les `1b`/`19` 36 o (+`0x4b` / +`0x31`) |

- Les IN **`1f` 40 o ne contiennent pas** le `chainHex` — seulement slot (`81:62`), type événement (`05:79:0a` assign / `0e` None / `2f` scroll intermédiaire).
- Echo IN après f0 interstitiel : **`preset_double_bas:67`** (octet bas du double **preset** du `1b`), pas le double scroll.
- Rejet Stomp si mauvais double scroll : echo **`XX:04`** → pas de bulk 92 o.

### Lane scroll — ce qui est corrigé (mai 2026)

| Problème | Correction |
|----------|------------|
| `f0` interstitiel hardcodé `52:11` | Octets 12–13 = `hw_model_scroll_ack_double()` |
| Pas de 2ᵉ `f0` HX après `19` #1 | `+0x2e` sur le ctr avant `send_pull_19_first` |
| ACK `1f` **avant** le pull décale la lane | **Pas d’ACK** sur le `1f` déclencheur + `1d` différé abandonné si pull (`would_start_hw_model_pull_on_1f`) |
| `cd_lane` effacée au re-arm pull | Conserver `cd=04` après wrap `fd:64` |
| Debounce 50 ms sur `1f` | Retiré ; settling post-pull **50 ms** à la place |
| Gap writer sur ACK 272 | Exclus du délai 14 ms (`usb_writer.rs`) |

### Lane scroll — **bug ouvert : valeur initiale**

| Source | Premier double `f0 sub=08` | u16 LE |
|--------|---------------------------|--------|
| **`01_connect_HXEdit.json`** / **`connect_Linux_for_ synchro.json`** (avant tout `1d`/`1f` scroll) | **`09:10`** | **`0x1009`** |
| **`3_scroll_HXEdit.json`** (1er pull interstitiel) | **`35:10`** | **`0x1035`** (+`0x2c` après bootstrap) |
| **Code actuel** `HelixState::new()` | `61:65` | **`0x6561`** ← **faux** (domaine preset `0x64xx`, pas scroll `0x10xx`) |

**Constats captures** :

- Pendant **connect** : **0** trame scroll `1d`/`1f` — les ACK init ne resynchronisent **pas** la lane.
- Le `09:10` du connect est envoyé en **dur** dans `helix/modes/connect.rs` (handshake post-`x11`) — **pas** branché sur `hw_model_scroll_ack_ctr`.
- Logs terrain récents : `scroll_ack_double=9e:67` (hi **`0x67`** = famille preset) → echo `17:04` → `finalize: aucun bulk assignable`.

**À faire (prochain agent)** :

1. Init `hw_model_scroll_ack_ctr` à **`0x1009`** (pas `0x1035` ni `0x6561`).
2. Lors de l’envoi bootstrap `f0` `09:10` dans `connect.rs`, **aligner** le compteur scroll (même valeur).
3. Tester scroll : logs doivent montrer hi **`0x10`**, echo **`XX:67`**, bulk 92 o avec `module_hex`.
4. Optionnel : ne pas effacer `pending` sur pull échoué (rattrapage).

**Captures utiles** (`src-tauri/paquets JSON/`) :

- `3_scroll_HXEdit.json` — référence HX (pull OK, doubles `35:10`…)
- `3_scroll_HXLinux.json`, `scroll_problem_HXLinux.json` — échecs Linux
- `01_connect_HXEdit.json`, `connect_Linux_for_ synchro.json` — bootstrap `09:10`
- `crash_HW_Linux.json` — plantages / rafales

**Tests** : `cd src-tauri && cargo test slot_model_hw_pull`

---

## 19 mai 2026 — Prochaine session : Phase C (écoute passive IN → grille + panneau params)

**Objectif produit (accord session)** : après chargement preset (`preset_data` = parse initial uniquement), toute modif **hardware ou UI** sur un slot doit mettre à jour **la cellule concernée** (modèle / vide / params) **sans** `request_preset_content` à chaque tick. Vision grille : **18 colonnes × 2 lignes** (Path In / Split / Merge / Slots / Out).

**Déjà en place (ne pas refaire)** :

- Phase A/B : `helix/slot_watch.rs`, empreinte capsule (`anchor12` + `ed_suffix7`), événement `models:slot-content-changed` (`kind: "content"`), poll `models_hw_slot_content_watch_ms` (défaut 1200 ms).
- Soft-sync grille : entre deux dumps USB, clone `lastHwSyncNormalizedSlots` — pas de re-parse `get_active_preset_slots` (§12 mai).
- Write live OUT : `helix/live_write.rs` (float `27` + `77:ca`, bool/discret `23` + `77:c2/c3` ou index).
- Doc détaillée surveillance : [`docs/todo-slot-content-watch.md`](docs/todo-slot-content-watch.md).

### Convention slot (à ne pas confondre)

| Langage utilisateur | Index Kempline / grille | `slot_bus` USB (`82:62:XX:1a` ou `85:62:XX`) |
|---------------------|-------------------------|-----------------------------------------------|
| « Slot 1 » (1er bloc effet du path) | **0** | **`0x01`** |
| « Slot 2 » | **1** | **`0x02`** |
| … | … | … |

Les captures de mai 2026 nommées `Slot0_…` = **index 0** (premier slot effet).

### Protocole wire — paramètre live (validé captures HX Edit)

**Captures analysées et bonnes** (`src/Paquets Json/`, USBPcap Windows, preset « Preset Test », slot index **0**) :

| Fichier | Scénario | Résultat |
|---------|----------|----------|
| `Slot0_Change_param_#0.json` | 1er param (UI) | `param_selector = 0x00`, float après `77:ca` |
| `Slot0_Change_param_#1.json` | 2e param | `param_selector = 0x01` |
| `Slot0_Change_param_#2json` | 3e param | `param_selector = 0x02` |
| `Slot0_Change_Model_2_Time.json` | 2× changement modèle sur slot 0 | assign `83:66`, bulks IN, snapshots — dialecte **modèle**, pas param |

**Ancre commune** (même offset que `live_write.rs`, trames IN **52 o** `f0:03` côté HX Edit) :

```text
85:62 :SS :1d :c3 :1a :00 :1c :PP :77 :XX …
         slot_bus              param#   type
```

| Octet `XX` après `77` | Type | Valeur |
|------------------------|------|--------|
| `ca` + 4 octets BE | Float | IEEE (souvent 0…1) |
| `c2` / `c3` | Bool | off / on |
| `00`…`0n` | Discret | index position |

- **`PP` = index paramètre** dans l’ordre wire du modèle (0, 1, 2, …) — **identique** à `param_index` / `param_selector` HXLinux (`param_selector_byte_from_index` dans `live_write.rs`).
- **Un tour de knob hardware** = **plusieurs** trames avec le **même `PP`** et float qui change (pas un bug de capture).
- HX Edit envoie surtout des **IN 52 o** avec la valeur ; les **OUT `ed:03` 16 o** (`87:59`) sont du keep-alive/sync, pas la payload valeur.
- Pour lier au **nom** du param : slot + modèle actif (bloc `83:66:cd:…`) + fichier `.models` ; en **mono**, appliquer la même règle que l’envoi : `paramsVisibleForSignal` / `liveWriteParamIndexForRow` (`models.ts`) pour ne pas décaler les index `stereo-only`.

### Données d’analyse figées (19 mai — ne pas refaire le reverse)

**Export Wireshark / USBPcap** : champ `usb.capdata` au même niveau que `usb` dans `_source.layers` (pas imbriqué dans `usb`). Sur captures Windows, IN device = `usb.src` **≠** `"host"` (souvent `"1.1.1"`) — ne pas filtrer uniquement `src == host` pour les IN.

**Parseur Python / Rust — offsets dans `capdata` splitté par `:`**

Après repérage de `85:62:01` à l’index `i` :

| Champ | Index hex parts |
|-------|-----------------|
| `slot_bus` | `i+2` (ici `01`) |
| `param_selector` (`PP`) | `i+8` |
| marqueur type | `i+9` = `77`, `i+10` = `ca` \| `c2` \| `c3` \| index discret |
| float BE (si `ca`) | `i+11` … `i+14` |

**Trame IN 52 o type param (ex. `#0`, 1er pas, t≈1,20 s)** — à utiliser en test unitaire :

```text
2b:00:00:18:f0:03:02:10:00:f5:00:04:09:02:00:00:00:00:04:00:1b:00:00:00:82:69:1e:6a:84:52:00:44:06:79:14:6a:85:62:01:1d:c3:1a:00:1c:00:77:ca:3e:dc:28:f5:40
```

- Préfixe `2b:00:00:18:f0:03:02:10…` (diffère du OUT HXLinux `27:00:00:18:80:10:ed:03…`).
- Bloc stable avant slot : `82:69:1e:6a:84:52:00:44:06:79:14:6a` (même famille que focus 44 o).
- Queue utile : `85:62:01:1d:c3:1a:00:1c:00:77:ca:3e:dc:28:f5` → PP=`00`, float≈**0,43**.

**Statistiques par capture param (preset « Preset Test », slot index 0)**

| Fichier | Paquets fichier | `capdata` | Durée ~ | Pas IN `77:ca` | `PP` | Float (1er→dernier) | Fenêtre rafale | OUT `ed:03` 16 o |
|---------|-----------------|-----------|---------|----------------|------|---------------------|----------------|------------------|
| `Slot0_Change_param_#0.json` | 1262 | 40 | 4,2 s | **9** | `0x00` | 0,43 → 0,51 | 1,20–1,98 s | 4× `87:59`, seq `23→26` |
| `Slot0_Change_param_#1.json` | 1216 | 47 | 4,0 s | **15** | `0x01` | 0,54 → 0,69 | 0,74–0,95 s | 3× `87:59`, seq `47→49` |
| `Slot0_Change_param_#2json` | 916 | 38 | 3,0 s | **12** | `0x02` | 0,61 → 0,75 | 0,84–0,98 s | 3× `87:59`, seq `5e→5f…` |

- **OUT keep-alive param** (identique sur les 3 captures, **pas** la valeur) :  
  `08:00:00:18:80:10:ed:03:00:<seq>:00:10:87:59:00:00` — seul l’octet seq (position 9) bouge ~toutes les **1 s**.
- **IN echo court** (optionnel, calage seq) : `08:00:00:18:ed:03:80:10:00:<seq>:00:10:df:03:00:00` — **sans** bloc `83:66` → `ingest_ed03_param_echo` actuel ne s’en sert pas pour le modèle.
- **Knob HW** : N pas = N trames IN 52 o, **même `PP`**, float monotone (comportement normal, pas erreur de capture).

**Changement modèle ×2 — `Slot0_Change_Model_2_Time.json`**

| | Détail |
|--|--------|
| Fichier | 1622 paquets, **98** `capdata`, ~**5,08 s** |
| Slot | `81:62:01` / `82:62:01` (index **0**) |
| Rafales | ~**t=1,02 s** et ~**t=3,02 s** (2 remplacements modèle) |
| OUT préambule assign (36 o) | `1b:00:00:18:80:10:ed:03:00:dc:00:04:3f:41:00:00:01:00:06:00:0b:00:00:00:83:66:cd:03:fc:64:2d:65:81:62:01:00` |
| IN bulk assign (ex. 92 o, +2 ms) | commence par `53:00:00:18:ed:03:80:10…83:66:cd:03:fc:67:…` |
| Suite | IN ~68 o méta preset, IN **272 o** avec `SNAPSHOT 1/2/3`, texte **Preset Test** |
| 2ᵉ changement | variante **`83:66:cd:04`** sur un IN ~68 o (pas seulement `cd:03`) |
| ≠ param | pas de `85:62…1c:PP:77:ca` dans la fenêtre utile |

**Snippet Rust de recherche (à copier en Phase C)**

```rust
// Chercher dans buf IN : … 85 62 SS 1d c3 1a 00 1c PP 77 XX …
for i in 0..buf.len().saturating_sub(15) {
    if buf[i..i+3] != [0x85, 0x62, _] { continue; } // SS = buf[i+2]
    if buf.get(i+3..i+8) != Some(&[0x1d, 0xc3, 0x1a, 0x00, 0x1c]) { continue; }
    let pp = buf[i + 8];
    if buf.get(i+9) != Some(&0x77) { continue; }
    let tag = buf[i + 10];
    // tag 0xca => float BE buf[i+11..i+15]
}
```

### À faire en priorité (implémentation)

1. **Backend — parse IN passif** (nouveau module ou extension `usb_listener.rs` / `mod.rs`) :
   - Scanner chaque bulk IN `0x81` : chercher `85:62` + `00:1c` + `PP` + `77`.
   - Extraire `(slot_bus, param_selector, valeur décodée, kind float|bool|discrete)`.
   - Ignorer / fusionner les rafales (debounce ~100–200 ms par clé `slot_bus:PP` — garder dernière valeur).
   - Ne déclencher que si `hw_active_slot_bus` correspond **ou** si le bus est connu dans la grille courante.

2. **Backend — événement** : émettre vers le front (ex. `models:slot-param-changed` ou enrichir `models:slot-content-changed` avec `kind: "param"`, `paramIndex`, `rawValue`, `valueType`).

3. **Front — `models.ts`** :
   - Listener : mettre à jour la ligne du panneau params **in-place** (sliders / bool / combo) pour le slot + `paramIndex` sans `fetchSlotChainParamValuesReliable` / sans dump.
   - Optionnel : mettre à jour `lastHwSyncNormalizedSlots` / cache valeurs chaîne si la grille affiche des résumés.

4. **Ensuite (même phase C)** : decode IN **modèle** / **slot vide** depuis captures assign (`Slot0_Change_Model_2_Time.json`, `Model_change_slot1_Linux.json`) → MAJ cellule matrice (nom, icône, vide) sans `preset_data`.

5. **Tests** : unitaires Rust sur hex extraits des 3 JSON param `#0/#1/#2` ; test que `PP` 0/1/2 et floats monotones sont parsés.

### Inventaire `src/Paquets Json/` (19 mai soir — à analyser demain)

**Déjà reverse / données figées ci-dessus** (ne pas refaire sauf doute) :

- `Slot0_Change_param_#0.json`, `#1.json`, `#2json`
- `Slot0_Change_Model_2_Time.json`

**Nouvelles captures à analyser en session** :

| Fichier | Scénario supposé (d’après le nom) |
|---------|-----------------------------------|
| `Slot0_Delete_Model.json` | Suppression / vidage modèle slot 0 |
| `Slot0_Move_Path1.json` | Déplacement bloc vers path 1 |
| `Slot0_Move_Path2.json` | Déplacement bloc vers path 2 |
| `In_Change_Param_#0.json` | Twist param #0 sur bloc **Input** |
| `Out_Change_Param_#0.json` | Twist param #0 sur bloc **Output** |
| `Split_Change_Param_#0.json` | Twist param #0 sur **Split** |
| `Merge_Change_Param_#0.json` | Twist param #0 sur **Merge** |
| `Save Preset HXEdit.json` | Sauvegarde preset (HX Edit) |

**Autres** (référence / secondaire) : `verif.json` (déjà cité ailleurs pour live write).

Ordre d’analyse suggéré demain : **Delete** + **Save** (modèle / preset) → **Move Path1/2** → blocs **In/Out/Split/Merge** param (vérifier `slot_bus` `0x00` / `0x09` / `0x0a` / `0x13` et si le motif `85:62…1c:PP:77` est identique).

### Captures encore utiles (si absentes des fichiers ci-dessus)

| Manquant | Action HW |
|----------|-----------|
| Insert model dans slot **vide** | Si `Delete` ne couvre pas l’assign complet |
| Bool / discret HW | Toggle ou liste sur slot 0 |
| Même scénario **HXLinux** vs HX Edit | 1 capture Linux pour diff OUT |

### Fichiers code prévus

| Fichier | Action |
|---------|--------|
| `helix/usb_listener.rs` | Appeler ingest param après lecture IN |
| `helix/mod.rs` ou `helix/slot_param_in.rs` (nouveau) | Parse `85:62…1c:PP:77` |
| `lib.rs` | `emit` Tauri event |
| `models.ts` | Handler + MAJ panneau ; debounce UI |
| `preset_chain_params.rs` | Réutiliser decode types si possible |

### Ne pas faire / pièges

- Ne pas réactiver dump preset à chaque notif slot (`models_hw_force_preset_dump_on_slot_notify=1` = secours seulement).
- Ne pas utiliser `preset_data` pour la surveillance temps réel (accord architecture mai 2026).
- Teardown USB agressif à la quit : **abandonné** (cassait reconnexion) — voir `docs/todo-analyse-trames-communes.md`.

### Références rapides

- Analyse trames communes / idle : `docs/todo-analyse-trames-communes.md`
- Protocole write : `Line6_HX_Stomp_USB_Protocol.md`, § live write dans ce fichier (26–27 avril)
- Branche de travail : `refactor/multithread`

---

**18 mai 2026 — Keep-alive `f0:03` : canal activé, sync slot hardware → UI**

**Symptôme** : après unification du keep-alive `ed→ef→f0` et fix ACK `ed` parasite, le negotiate/subscribe `f0` passait (IN subscribe + handshake OK), mais **silence** sur les polls réguliers `08…02:10:f0:03` (`sub=10`) ; l’UI ne suivait pas le slot actif quand on changeait de bloc sur le Helix.

**Causes** (comparaison `src/Paquets Json/connect_device_30s_HXEdit.json` vs captures Linux) :

1. **Poll d’activation manquant** — HX Edit envoie, juste après le handshake `f0`, un poll court **`sub=08`** (`08:00:00:18:02:10:f0:03:…:00:08:09:10…`, frame #3255). HXLinux avait supprimé cette étape (`connect.rs` : « pas de poll ici »).
2. **Compteur `x2` en double** — le handshake utilisait `state.x2_cnt` sans incrément ; `next_x2_cnt()` sur le poll d’activation renvoyait encore **0x02** au lieu de **0x03** (HX : handshake **02**, activation **03**).
3. **Keep-alive / noms trop tôt** — le 1er poll `sub=10` et `RequestPresetNames` partaient pendant la rafale preset sur `0x81` ; HX Edit attend ~**688 ms** après bootstrap phase 4 (frames #3447 → #3761), puis la liste des noms.

**Correctifs** :

- **`src-tauri/src/helix/modes/connect.rs`** : poll d’activation `sub=08` après handshake (`OutPacket::with_delay` 15 ms) ; handshake via **`next_x2_cnt()`**.
- **`src-tauri/src/helix/keep_alive.rs`** : **`POST_PHASE4_SETTLE_MS = 700`** — `sleep` au démarrage du thread keep-alive avant `ed→ef→f0`.
- **`src-tauri/src/helix/modes/await_post_bootstrap_settle.rs`** : même délai avant **`RequestPresetNames`** (plus de `RequestPresetName` juste après bootstrap).

**Validation** : IN `f0:03:02:10` sur activation et polls réguliers ; compteurs requête/réponse alignés ; **changement de slot hardware visible dans l’UI** (test terrain, sans capture obligatoire). Les payloads subscribe/handshake sont **identiques byte à byte** à HX Edit.

**Doc détaillée** : [`docs/recap-keep-alive-ed-ef-f0-mai-2026.md`](docs/recap-keep-alive-ed-ef-f0-mai-2026.md).

**Suite** : un petit bug mineur reste à traiter (session suivante) ; les trames **44 o** `21…f0:03` (focus / détail slot) peuvent être re-capturées si besoin — voir `slot_focus_in.rs` et §12 mai ci-dessous.

**12 mai 2026 (suite) — Grille : mise à jour alignée sur la RAM `preset_data` (relecture preset uniquement)**

Objectif produit : **ne plus reconstruire la matrice** à partir d’un parse de **`preset_data`** (Rust) **entre** deux relectures — éviter les « fantômes » (UI qui réaffiche un slot supprimé sur le Helix parce que le buffer PC était encore l’ancien dump).

**`src/models.ts` — `runHardwareSyncSoftRefresh`**

- Si le cycle **ne** lance **pas** `request_preset_content` (`wantUsbPresetDump` faux) : **plus d’invoke** `get_active_preset_slots` / `get_active_preset_slots_debug` pour alimenter la grille. On clone **`lastHwSyncNormalizedSlots`** (snapshot pris au dernier chargement / dernière relecture USB + éventuelles **MAJ optimistes** picker / bouton × remove) pour **`softRefreshParamsPaneFromSlots`**, **`consumePendingHardwareSlotSelection`**, etc.
- Si **`didUsbPresetDumpThisCycle`** est vrai (poll `models_hw_usb_preset_poll_ms`, forçage `models_hw_force_preset_dump_on_slot_notify`, etc.) : comportement inchangé — attente parse après dump, **`applyProbeSlotMergeToNormalized`**, puis **`renderSlots`** + **`rememberHwSyncChainLayout`** quand la signature de layout l’exige.
- **Garde** explicite : sans dump USB **dans ce cycle**, sortie après rafraîchissement panneau / sélection HW — **pas** de **`renderSlots`** ni de nouveau **`rememberHwSyncChainLayout`** depuis ce chemin (le snapshot a déjà été posé au chargement ou après dump).
- **Traces** : `emitModelsSyncTrace` avec libellés du type `softSync sans request_preset_content : pas de get_active_preset_slots` et `softSync sans dump USB ce cycle : pas de renderSlots`.
- **Anti-spam (même session)** : messages soft-sync / cooldown / abort / événement `models:hardware-slot-changed` passent par **`emitModelsSyncTraceThrottled`** (fenêtres 400 ms … 30 s selon la clé) ; succès **`sync_hardware_slot_focus_usb`** → **`hwSlotDebugLog`** uniquement (`models_debug_hw_slot_sync=1`) ; **`console.info` événement slot** seulement si `models_debug_hw_slot_sync` **ou** `models_debug_sync_trace`.

**`rememberHwSyncChainLayout`**

- **`lastHwSyncNormalizedSlots`** : désormais pour **toute** liste de slots non vide (`slots.length > 0`), pas seulement la grille 16 cases — permet au soft-sync d’avoir un snapshot en mode **flow** aussi.

**Complément même session — remove slot (×) sans fantôme immédiat**

- **`attachSelectedSlotRemoveButton`** : après `probe_slot_model_usb` **remove** réussi, MAJ optimiste du snapshot + **`mergeProbeSlotModelUntil`** + **`slotModelUsbProbeInFlight`** pendant l’invoke (aligné sur le chemin picker add/replace), car **`probe_slot_model_usb`** n’écrit **pas** dans `preset_data` côté Rust.

**Doc / flags à relire avec ce changement**

- **Soft-sync (mai 2026)** : plus de poll **200 ms** — déclenché par **`models:hardware-slot-changed`** uniquement. Détail, flags, retrait futur : **`docs/models-hardware-sync.md`**.
- **`models_hw_sync_interval_ms`** : throttle **optionnel** entre deux soft-sync event (défaut **off**). Ne re-parse **pas** la grille entre deux `request_preset_content`.
- **`models_hw_usb_preset_poll_ms`** : timer **dédié** (ex. 2500 ms) pour relecture preset USB → grille alignée sur RAM device.
- Section **« Flags front utiles »** plus bas (notes *soft refresh* / signature layout) : la phrase « re-parse à chaque tick » pour la grille est **obsolète** ; garder l’idée *in-place params* + *renderSlots seulement après relecture*.

**12 mai 2026 — IN « focus slot » parsées, capsule par slot, comparaison captures HX Edit vs Linux**

Implémentation (commit **`02e0836`**, branche **`refactor/multithread`**) :

- **`src-tauri/src/helix/slot_focus_in.rs`** : après OUT type focus, reconnaît sur les IN bulk **`0x81`** la paire **36 o** (`19…` + `83:66:cd:04` + suffixe) + **44 o** (`21…` + `f0:03:02:10` + bloc stable **12 octets** `82:69…6a` puis `82:62:SS:1a`). Expose **`SlotFocusInCapsule`** (`slotBus`, hex des trames, `anchor12`, etc.). Test unitaire sur hex issus de **`Slot1_to_slot2_PresetTest_HXEdit.json`**.
- **`src-tauri/src/helix/mod.rs`** : **`last_slot_focus_capsule: [Option<SlotFocusInCapsule>; 16]`** — dernière capsule parsée **par index Kempline** après **`sync_hardware_slot_focus_usb`**.
- **`sync_hardware_slot_focus_usb`** (`lib.rs`) : en fin de fenêtre de capture, parse les trames → remplit **`last_slot_focus_capsule[slotIndex]`** ; JSON de retour enrichi avec **`slotFocusParsed`** (`null` si paires 36+44 introuvables).
- **`get_active_preset_slot_chain_param_values`** : si **`preset_data`** prêt → inchangé (segment assignable + `read_params`) ; si buffer **vide / pas prêt** mais capsule pour ce slot avec **`slot_bus`** cohérent → **`Some([])`** pour que le front ne **timeoute** pas en boucle (panneau sans valeurs chaîne tant qu’il n’y a pas de segment). Si **`preset_debug_verbose`** : log **`[SlotFocus][corr]`** si l’**ancre 12 o** de la capsule apparaît dans **`preset_data`** (corrélation preset ↔ IN).
- **Invalidation** : **`preset_data.clear()`** dans **`request_preset_content`** et **`force_recover_preset_reader`** remet aussi **`last_slot_focus_capsule`** à **`None`** partout (évite une capsule « ancien slot » réutilisée après vidage buffer).
- **Tests Rust** : module **`hxedit_slot_focus_preset_test_reference`** dans **`lib.rs`** — hex figés **Slot1_to_slot2** vs **Slot2_to_slot3** PresetTest (diff OUT / IN documentée).

**Front (`src/models.ts`)**

- **`localStorage.setItem("models_hw_slot_focus_await_chain", "1")`** : avant **`fetchSlotChainParamValuesReliable`**, **`loadAndShowModelsParamsForSlot`** fait un **`await sync_hardware_slot_focus_usb`** (si focus USB pas désactivé) — utile quand le soft-sync lance le focus en **fire-and-forget** et qu’il n’y a **pas** de dump preset (évite course capsule vs `invoke` chaîne).
- **Trace `softSync merge probe slot=…`** : une **seule** émission **`emitModelsSyncTrace`** par fenêtre de grace (**`mergeTraceEmitted`**) pour ne pas spammer la console / terminal à chaque tick (~200 ms).

**Comparaison captures `Slot1_to_2_HXEdit.json` vs `Slot1_to_2_Linux.json` (reprise session)**

- **HX Edit** : OUT **`83:66:cd:04`** (tag **`10`** sur cette capture — pas forcément `slot_bus` ; sur PresetTest c’était **`02`**) + IN **36 o** puis **44 o** `f0:03` avec ancre **`82:69:27:6a:84:52:01:44:03:79:13:6a`**.
- **Linux** (capture alignée **`switch_active_hardware_slot`** HXLinux) : OUT **`83:66:cd:03`** + **`f9`**, une IN **36 o** avec **`cd:03`**, **aucun** paquet **`21…f0:03`** dans tout le JSON exporté → le parseur « focus » **`cd:04`** ne matche **pas** ce trafic ; comparaison stricte HX Edit ↔ Linux : reprendre une capture Linux avec **`sync_hardware_slot_focus_usb`** / OUT **`cd:04`**, ou étendre le parseur aux variantes **`cd:03`** si on veut unifier.
- **`Slot1_to_2_HXEdit.json`** : capture **slot-only** — **pas de nom de preset** en ASCII dans le fichier (pas de gros IN type `Change_to_PresetTest`). Pour prouver « même preset » que Linux : noter le preset **hors** JSON ou inclure une fenêtre de capture avec dump / nom.

**Rappel correction doc (éviter confusion)**

- **`preset_data_packet_double()`** dans **`helix/mod.rs`** : les **2 octets** viennent de **`preset_pkt_counter`** (convention nom Kempline), **pas** des « deux derniers octets du buffer **`preset_data`** ».

**11 mai 2026 — lecture preset vs changement de slot (captures HX Edit) + garde-fous JS / suite**

- **Constat captures Windows (HX Edit)** dans `src/Paquets Json/` :
  - **`Change_to_PresetTest_HXEdit.json`** : se positionner sur le preset « Preset Test » → trafic **plus lourd** (ex. plus d’**OUT `0x01` avec `80:10:ed:03`**, plus d’octets **IN bulk `0x81`** que les captures slot-only).
  - **`Slot1_to_slot2_PresetTest_HXEdit.json`** / **`Slot2_to_slot3_PresetTest_HXEdit.json`** : changement de **slot** dans le **même** preset → trafic **nettement plus léger** → **forte présomption** : HX Edit **ne** refait **pas** une relecture preset **équivalente** à un changement / focus preset ; lecture **plus ciblée** (à corréler au parseur).
- **Trames repérées (ex. `Slot1_to_slot2_PresetTest_HXEdit.json`, ~+874 ms après début fichier)** :
  - **OUT `0x01` #1335** (40 octets `usb.capdata`) : commande type focus slot, contient **`83:66:cd:04`** (variante par rapport au **`cd:03`** observé sur la commande **`switch_active_hardware_slot`** côté HXLinux) puis **`82:62:02:1a`** (`slot_bus` **0x02** = slot path 1 index 1).
  - **IN `0x81` #1359** (36 octets) puis **#1361** (44 octets) : courtes réponses après l’OUT ; candidats pour la **réponse « bloc slot »** (pas un dump preset entier).
- **HXLinux (11 mai)** : sur notif **slot actif** (séquence backend), le soft-sync **ne force plus** par défaut **`request_preset_content`** (pas de dump USB à chaque changement de slot). **Mise à jour 12 mai (suite)** : entre deux **`request_preset_content`**, le soft-sync **ne** re-parse **plus** `get_active_preset_slots` pour **reconstruire la grille** ; panneau params / sélection HW = snapshot **`lastHwSyncNormalizedSlots`** (+ MAJ optimistes probe/remove). Option **`models_hw_force_preset_dump_on_slot_notify=1`** (secours dump immédiat). **`get_active_preset_slot_chain_param_values`** : segments depuis **`preset_data`** quand le dump est prêt ; **voir 12 mai 2026** (bloc focus) pour repli **sans** buffer (capsule IN + `Some([])`). **Poll** : **`models_hw_usb_preset_poll_ms`** (ex. `2500`) pour re-dump périodique → alors grille alignée sur RAM fraîche.
- **Garde-fous ajoutés / à conserver** pendant l’exploration « lecture slot » (ne **pas** tout retirer d’un bloc) :
  - **`src/models.ts`** : file **`enqueueHardwareSlotSwitch`** (pas deux `switch_active_hardware_slot` concurrents) ; **`waitUntilHardwareSyncIdle`** avant switch (évite un switch **pendant** les `await` du soft-sync / dump) ; **`fetchSlotChainParamValuesReliable`** en attente **temps** (défaut **14 s**, soft-refresh idem **14 s**) + trace **`chainFetch TIMEOUT`** si sync trace activée.
  - **`src-tauri/src/helix/usb_writer.rs`** : **`MIN_ED03_OUT_GAP_MS = 14`** (test **20 ms** rétrogradé ; l’écart slot/preset côté UI tenait surtout aux **courses** buffer vide / sync, pas à ce seul réglage).
- **Suite** : **`probe_hardware_slot_focus_usb`** + **`sync_hardware_slot_focus_usb`** voir **12 mai 2026** (parse IN, `slotFocusParsed`, capsule par slot). Prochain pas : exploiter l’**ancre** / IN au-delà du repli chaîne vide, variantes **`cd:03`**, corrélation preset. **Ne pas** supprimer les stabilisations tant qu’elles ne sont pas redondantes mesurées.

**2 mai 2026 — protocole ED03 stabilisé, blocs spéciaux (Input/Output/Split/Merge) détectés comme slot actif. Voir section « 30 avril – 2 mai 2026 » ci-dessous.**

**8 mai 2026 — anti-flash matrice, picker optimiste, parse preset transitoire, traces ModelsSync**

Symptômes traités : flash plein écran / grille vidée après **poll USB** ou après **changement de modèle** (`hw_notify_force` → `request_preset_content`) quand le backend renvoie un instant **`get_active_preset_slots` → `[]`** ou liste vide ; en JS **`[]` est truthy**, donc l’ancien code croyait à tort avoir des slots valides et appelait **`renderEmpty`** (« Aucun bloc détecté »).

**`src/models.ts` — soft-sync (`runHardwareSyncSoftRefresh`)**

- Après **`request_preset_content`** (dump **dans ce cycle**) : attente **~120 ms** avant la première lecture des slots ; boucles d’attente : ne **valider** le résultat que si **`normalizeSlotsPayloadFromInvoke(…).length > 0`** (sinon on continue à poller). **12 mai (suite)** : sans dump dans le cycle, le soft-sync **ne** refait plus cette boucle de parse pour la grille — clone du snapshot `lastHwSyncNormalizedSlots` (voir section en tête).
- Si après attente il n’y a **toujours pas** de slots : **`normalized = null`**, plus de log factice « usbDump ok » avec 0 slot.
- **Abort** : si `normalized` absent ou **longueur 0**, ne pas détruire la grille quand **`lastHwSyncNormalizedSlots`** contient encore un snapshot utile → **`softRefreshParamsPaneFromSlots(lastHwSyncNormalizedSlots)`** + log `emptyParse keepExistingGrid`.
- **Debounce layout** : dump USB déclenché **uniquement** par **`poll_interval`** (pas **`hw_notify_force`**) → même **passage 1 / 2** anti-flash que sans USB (`usbDumpIsPollOnly`), pour éviter un **`renderGrid16` complet** sur un simple pic de signature après re-parse.
- **Cache grille** : **`lastHwSyncNormalizedSlots`** (copie des `SlotDebug` au dernier `rememberHwSyncChainLayout`) ; **12 mai (suite)** : snapshot dès que `slots.length > 0` (grille 16 ou flow). Sert au rollback / cohérence signature et à l’UI optimiste.

**`src/models.ts` — picker changement de modèle (optimiste)**

- Ordre : **MAJ pastille + ligne catégorie** (`patchMatrixSlotVisualFromSlot`, `patchMatrixCategoryDescFromSlot`) + **`loadAndShowModelsParamsFromCatalogDefaults`** (défauts **`.models`** via **`buildDefaultChainValuesForSourceOrder`**, pas de lecture chaîne USB) **puis** **`probe_slot_model_usb`**.
- **`data-kempline-slot-desc-index`** sur les cellules **L2/L4** (`makeMatrixCategoryCell`) pour cibler la ligne catégorie sous le slot.
- **`slotModelUsbProbeInFlight`** : pendant probe + chargement catalogue, **soft-sync ignorée** (évite course avec d’anciens slots).
- Après **succès** USB : **`pendingForceUsbPresetContent = true`** pour réaligner sur le matériel au prochain cycle.
- **Erreur USB** : restauration depuis snapshot + **`loadAndShowModelsParamsForSlot`**, ou **`scheduleLoadForPreset(..., true)`** si pas de snapshot.

**`src/models.ts` — autres garde-fous**

- **`renderSlots([])`** : si **`lastHwSyncNormalizedSlots`** est non vide, **ne pas** appeler **`renderEmpty`** (évite flash si un chemin passe encore un tableau vide).
- **`requestLoadForPreset`** : si `slots !== null` mais normalisé **vide**, **continuer à poller** au lieu de rendre.

**Traces diagnostic**

- **`localStorage.setItem("models_debug_sync_trace", "1")`** (fenêtre **Models**) : lignes **`[ModelsSync][timestamp] …`** en **`console.info`** + relais **`invoke("log_frontend_message")`** → terminal **`cargo tauri dev`** (`eprintln!` + flush stderr dans **`src-tauri/src/lib.rs`**).

**6 mai 2026 — changement de référentiel pour les paramètres de modèle (IMPORTANT)**

- **Source des paramètres d’un modèle** : désormais, le référentiel est le fichier **`src-tauri/resources/models/<category>.models`** (ex. modèle de catégorie **modulation** → **`src-tauri/resources/models/modulation.models`**).
- **`HX_ModelCatalog.json` n’est plus la référence des paramètres** (`params`) pour le panneau / alignement des valeurs.
- **Rôle actuel de `HX_ModelCatalog.json`** : rester une source de méta via **`chainHex`** (au minimum **category** et **subCategory**, et métadonnées associées).
- **Conséquence doc/code** : quand une ancienne section de ce mémo mentionne l’ordre/filtrage des paramètres via `HX_ModelCatalog.json`, la considérer comme **historique** et privilégier la logique basée sur les fichiers **`.models`**.

**6 mai 2026 (soir) — live write USB paramètre (état pour reprendre demain)**

Travail validé sur hardware : bools, **Ratio** (discret), **Level** (float 2 jambes), **Clipping** / **Gain mod** (segmentés comme la compression).

**Fichiers**
- **`src-tauri/resources/HelixLiveWrite.json`** — `ppDefault`, marques bool `boolMarkOff`/`On`, `boolDisplayTypes`, **`discrete23DisplayTypes`** (map `displayType` → **N** positions = taille de `format` dans **`HelixControls.json`**), `allowedFloatValueTypes`. Clés **camelCase** ; le struct Rust a **`#[serde(rename_all = "camelCase")]`** (sinon Serde **ignore** les clés → map discrets vide, tout part en **`27`** par erreur).
- **`src-tauri/src/helix/live_write_config.rs`** — chargement config, `infer_bool_wire_payload`, `discrete_23_step_count`, **`validate_usb_live_write_metadata`** (refus si pas bool / discret / float autorisé).
- **`src-tauri/src/helix/live_write.rs`** — paires **`23`** (bool `c2`/`c3` ; discret **`77` + index 0…N−1**) vs **`27`** + float IEEE BE ; jambe **`04`** = normalisé **0…1** ; jambe **`0c`** = **`chain_min + norm×(chain_max−chain_min)`** quand min/max fournis (captures HX **Level**), sinon doublon du normalisé.
- **`src-tauri/src/helix/mod.rs`** — `pub mod live_write_config`.
- **`src-tauri/src/lib.rs`** — `write_live_param(..., chain_min, chain_max)` ; logs **`legBChain`**, **`chainMin`/`chainMax`**.
- **`src/models.ts`** — `PendingLiveWrite` avec **`rawMin`/`rawMax`** ; **`liveWriteUsbNormalized01`** avant `write_live_param` / MIDI CC (ex. Ratio **0…5** → norme pour l’USB) ; **`chainMin`/`chainMax`** passés au backend ; erreur **`console.warn`** ; bool : write seulement depuis le geste utilisateur (évite double envoi après sync hardware).

**Règle pratique** : contrôle **`isDiscrete` + `segmented`** dans HelixControls → ajouter **`"displayType": N`** dans **`discrete23DisplayTypes`** (une capture **`23`** + `77` suffit pour confirmer). Ne pas confondre avec un slider **0…1** sur 2 positions : le bus peut être **discret**, pas float.

**Suite possible** : autres `displayType` segmentés non encore listés ; affiner jambe B si une capture contredit `min+norm×span` pour un bloc précis.

**7 mai 2026 (soir) — correction `pSel` mono/stéréo + incident UI**

Contexte : sur un modèle modulation, des writes UI ne ciblaient pas le bon paramètre en **mono** alors que le même modèle en **stéréo** fonctionnait.

Découverte clé validée par captures HX Edit :
- En stéréo, l’ordre `pSel` suit bien la séquence visible (`Rate..Headroom`).
- En mono, le paramètre **`Spread`** (`"stereo-only": true`) est masqué et ne doit **pas** compter dans l’index write.
- Exemples confirmés dans les captures :
  - `src/Paquets Json/Mix.json` : `... 1c:05:77 ...` => **Mix** en `pSel=05`
  - `src/Paquets Json/headroom.json` : `... 1c:07:77 ...` => **Headroom** en `pSel=07`

Pourquoi :
- Le write USB utilisait l’index de ligne UI brute ; après une entrée `stereo-only`, les paramètres suivants étaient décalés en mono.

Correctif appliqué :
- `src/models.ts` : ajout de `liveWriteParamIndexForRow(...)`.
- Le calcul de `paramIndex` envoyé au backend passe par l’ordre visible **selon signal** (`paramsVisibleForSignal`), donc en mono les `stereo-only` sont retirés du comptage.
- Branché sur les trois chemins d’envoi live : slider continu, bool, combo discret.

Incident pendant la session (corrigé) :
- Un refactor intermédiaire a cassé l’UI (`ReferenceError: paramsForDisplay`).
- Fix : retour au scope correct (`params`) dans `appendModelsParamRows`.
- Vérifications après fix : `cargo check` OK, logs `[LiveWrite][sent]` à nouveau visibles.

**7 mai 2026 (nuit) — règle générique sélecteurs discrets + override `PP`**

Constat pendant tests terrain : plusieurs sélecteurs (`valueType=0`) ne passaient pas sans mapping explicite `displayType -> N`, alors que la règle est souvent déductible depuis le `.models`.

Règles appliquées :
- **Fallback générique discret** (`src-tauri/src/helix/live_write.rs`) :
  - si `valueType == 0`
  - et `chainMin/chainMax` sont des entiers valides
  - alors `N = max - min + 1`, routage en **`0x23`** discret avec index `0..N-1`.
- La table **`discrete23DisplayTypes`** reste prioritaire pour les cas explicitement capturés.
- Ajout d’un override **`PP` par `displayType`** (`ppByDisplayType`) dans `HelixLiveWrite.json` + support Rust (`live_write_config.rs`, `live_write.rs`).

Découvertes captures HX Edit (exemples validés) :
- `wave_shape` : discret `0x23`, **7** positions (`77 00..06`), **PP=04**.
- `delay_heads` : discret `0x23`, **6** positions (`77 00..05`), **PP=04**.

Pourquoi certains sélecteurs “semblables” ne marchaient pas :
- ce n’est pas seulement la valeur (`N`) ; il faut le triplet **opcode + PP + index**.
- une règle unique “tous les sélecteurs = PP par défaut” est insuffisante ; d’où `ppByDisplayType`.

**27 avril 2026 (après-midi) — write USB en pause, priorité stabilité UI (anti-flash)**

État session du jour :

- Le write live USB a été fortement avancé (pair `27` `04/0c`, opcode `80:10:ed:03`, `PP`, `YY`, séquences `08` autour du write, sélection paramètre supplémentaire observée en offset 40).
- Le ciblage paramètre a progressé (cas `generic_knob` validés sur plusieurs modèles, Distortion et Compressor), mais la stabilité globale n’est **pas** encore suffisante.
- Après tentative d’unification write/read dans le même rythme de polling, l’**écriture hardware a régressé** (comportement non fiable).  
  => Décision : **mettre le write hardware en stand-by temporaire**.

Priorité immédiate (prochaine session) :

1. **Nettoyer la cinématique de mise à jour UI** pour supprimer les flashs (grille + panneau paramètres) lors des refresh.
2. Introduire un **paramétrage propre du refresh** (fréquence, stratégie soft/in-place, garde-fous de concurrence) pour rendre le comportement prévisible.
3. Revalider la boucle **hardware -> UI** seule (sans write live), puis seulement ensuite réintégrer le write pas à pas.

Rappels utiles :

- L’infobulle slider doit afficher la **valeur brute** (avant conversion) pour debug.
- Le polling hardware reste piloté par `localStorage.setItem("models_hw_sync_interval_ms", "200")` (100..5000).
- Logs verbeux `PresetDebug` de polling (`request_preset_content`, `try_parse_preset_kempline_grid`) ont été réduits pour limiter le bruit console.

**Fin de soirée 27 avril 2026 — panneau paramètres : problème encore présent (à traiter demain)**

- **Symptôme** : dès qu’un paramètre est modifié **depuis l’UI** (slider / multi-état), au bout de **quelques secondes** l’écran se **vide** puis se **reconstruit** (flash / reload du panneau ou plus large), **sans** autre action de l’utilisateur.
- Des correctifs ont été posés dans **`src/models.ts`** (sync in-place, clé de slot, signature `selectedParamsValuesSig` après load, pas avant ; rendu `renderModelsParamsPane` sans `inner.replaceChildren()` vide ; invalidation de l’updater au changement de slot) — **le comportement ci-dessus persiste** quand on touche un paramètre en UI.
- **Piste pour la prochaine session** : interaction **live write** / `markLiveWriteUiInteraction` / `LIVE_WRITE_SYNC_PAUSE_MS` vs boucle **`runHardwareSyncSoftRefresh`** / `refresh()` ; ou rechargement preset déclenché par une écriture qui change la signature chaîne / layout perçu côté parseur. À isoler avec logs ciblés ou en coupant temporairement le write / le poll pour reproduire.

### Note importante (fin de session) — captures JSON à refaire

- Deux captures faites en fin de session (**`Slot2 Threshold -60 to -45.7 to -28.6.json`** et **`Slot8 Level 0 to 3.7 to 6.9.json`**) n’étaient pas exploitables pour le reverse write/focus.
- Elles contiennent du trafic **`usb:usbhid`** (endpoint **`0x84`**) uniquement ; pas de trames ED03 bulk attendues (`27 ... 80 10 ed 03`, `08 ... 80 10 ed 03`).
- Le hardware/câble était bien le bon (plus de trafic quand le HX est débranché), mais l’export JSON ne contient pas la bonne famille de paquets pour l’analyse write.
- À la reprise : refaire une capture **courte (2–5 s)** avec un seul geste paramètre et filtrer avant export sur :
  - `usb.capdata contains 27:00:00:18:80:10:ed:03`
  - `or usb.capdata contains 08:00:00:18:80:10:ed:03`

**Capture encore manquante (slot / preset)** : quand **HX Edit** (Line 6) **relit** un preset (ou se resynchronise), l’UI se **positionne sur le slot actif** du hardware — la **séquence USB** (host ↔ device) pour ce comportement **n’a pas encore** été exportée. À prévoir : une capture **courte** au moment où HX Edit affiche le preset et met en surbrillance le bon bloc (sans grands mouvements ailleurs), endpoint bulk **`0x01` / `0x81`**, même style d’export JSON que les autres paquets. Voir aussi `Line6_HX_Stomp_USB_Protocol.md` (section slot + « ce qui reste »).

**26 avril 2026 (fin de soirée) — write live USB : ça marche sur le hardware**

Source de vérité protocole : exports Wireshark JSON dans `src/Paquets Json/` + doc vivante `Line6_HX_Stomp_USB_Protocol.md` (corrigée après analyse des JSON : opcode **`80:10:ed:03`**, pas `03:10:ed:03` pour ce write 48 octets).

**Découverte clé** : HX Edit n’envoie pas une seule trame `27` par changement de paramètre, mais une **paire** sur endpoint OUT **`0x01`** :

1. **`frame27_a`** : octets 8–11 de la forme `00 SEQ 00 **04**` (octet 11 = `0x04`).
2. **`frame27_b`** (~8 ms après) : `00 SEQ' 00 **0c**` (octet 11 = `0x0c`).

Entre les deux (et d’une paire à la suivante), les captures montrent :

- **`CTR`** (16-bit LE, offsets 12–13) : **+`0x1F`** (31) à **chaque** trame `27` (pas `+0x19` sur cet échantillon).
- **`YY`** (offset 28 dans le bloc `… cd PP YY 64 1e 65`) : **+1** entre jambe `04` et `0c`, puis encore pour la suite.
- **`SEQ`** (offset 9) : suit le flux des compteurs keep-alive via `next_x80_cnt()` (comme le reste de la stack).

**Float** : IEEE754 **big-endian** en offsets **43–46**, terminateur **`00`** en 47. Pour les potards Minotaur 0–10, la valeur machine est bien **affiché / 10** (ex. `10.0` → `1.0` → `3f 80 00 00`).

**Travail code (session)** :

- `src-tauri/src/helix/live_write.rs` : builder `80:10:ed:03`, paire `frame27_a` / `frame27_b`, `PP=0x04` pour le test **Gain** aligné captures Minotaur/Heir ; pré-trames `pre_x80` / `pre_x2` conservées avant le `27`.
- `src-tauri/src/helix/mod.rs` : état **`live_write_ctr`** / **`live_write_yy`** pour continuité des compteurs entre writes (initialisation type capture ; à affiner / resynchroniser depuis le device si besoin).
- `src-tauri/src/lib.rs` : `write_live_param` envoie les deux jambes ; **test actuel** : valeur **forcée** `forcedRaw=1.0` pour valider le hardware sans ambiguïté slider ; garde encore centrée sur **`symbolicId == "Gain"`** (les autres params loggent `[LiveWrite][pending-packet][guard]`).

**Validation utilisateur** : premier succès hardware avec log du type  
`frame27_a=… 00 04 …` puis `frame27_b=… 00 0c …`, même float sur les deux.

**Plan B MIDI** : toujours dispo (`write_live_param_midi_cc`, transport `midi_cc` dans `src/models.ts`) mais **secondaire** ; l’édition temps réel prioritaire repasse par **USB propriétaire**.

**Fichiers clés** : `helix/live_write.rs`, `helix/mod.rs`, `lib.rs` (`write_live_param`), `models.ts` (flags `models_live_write_transport`, etc.).

### Prochaine étape (prioritaire)

1. **Retirer le forçage `forcedRaw=1.0`** : envoyer le `rawValue` réel du slider (clamp 0..1).
2. **Mapper `PP` (octet 27) par paramètre / modèle** à partir des captures (`src/Paquets Json/` + table dans `Line6_HX_Stomp_USB_Protocol.md`) ; élargir le garde-fou au-delà de `Gain` (Tone, Level, etc.).
3. **Valider sur d’autres slots** (pas seulement le slot 0) et d’autres modèles ; vérifier si `live_write_ctr` / `YY` doivent être **initialisés** depuis le dernier trafic IN plutôt que des constantes de départ.
4. Optionnel : réactiver / compléter `in_echo_strict` si on trouve enfin des échos IN exploitables côté Linux pour coller au bloc `83 66 …` sans template statique.

### Capture Windows (référence continue) — limiter le bruit

- Fenêtre **2–5 s**, un geste ciblé, arrêt rapide.
- Filtre utile sur la bonne famille : `usb.capdata contains 27:00:00:18:80:10:ed:03` (write param observé dans nos JSON).
- Pour étendre le mapping : **un fichier JSON par paramètre** (déjà la convention dans `src/Paquets Json/`).

**25 avril 2026 (soir) — état write live + reprise Windows/Wireshark**

- **Lecture/sync UI** : fonctionne bien à **200 ms** avec refresh “soft” (pas de rerender complet si layout slots inchangé) + patch **in-place** du panneau paramètres du slot sélectionné (anti-flash).  
- **Focus panneau** : stabilisé (sélection conservée dans le même preset, purge sur vrai changement de preset).  
- **Trace USB** : endpoint `0x81/0x01` instrumentés ; bruit keep-alive réduit ; `PresetDebug` verbeux peut être coupé à chaud via `set_preset_debug_verbose(false)`.
- **Write live pipeline** :
  - front : `models_live_write_probe` + `models_live_write_enabled` (localStorage), transport optionnel `models_live_write_transport` (`usb_raw` \| `midi_cc`), MIDI `models_live_midi_channel`, `models_live_midi_cc`,
  - backend : `probe_live_param_write`, `write_live_param` (USB propriétaire), `write_live_param_midi_cc` (CC).
- **IMPORTANT — write USB réel** : voir bloc **26 avril 2026 (fin de soirée)** : **effet hardware confirmé** sous Linux avec paire `27` opcode `80:10:ed:03`, `CTR` +`0x1F`, jambes `04`/`0c`. Le code actuel garde encore un **test forcé** `Gain` → `forcedRaw=1.0` et un garde sur **`symbolicId == "Gain"`** jusqu’à généralisation.

### Ce qui reste côté protocole (après preuve hardware)

Les captures **Windows + HX Edit** dans `src/Paquets Json/` ont déjà permis d’identifier la famille **set parameter** (`27 … **80 10 ed 03**`, paire `04`/`0c`, `CTR` +`0x1F`, float BE). Pour généraliser HXLinux :

- enrichir le **mapping `PP`** (et si besoin autres octets) par **modèle + paramètre** ;
- capturer d’**autres slots / autres blocs** si le binaire diffère ;
- décider si **`live_write_ctr` / `YY`** doivent être **amorcés** depuis le dernier paquet IN observé plutôt que des constantes de session.

Les garde-fous opt-in (`models_live_write_*`) restent pertinents tant que l’écriture n’est pas stabilisée sur tous les cas.

### Commandes utiles (rappel runtime)

- Couper bruit `RequestPreset` :  
  `await window.__TAURI__.core.invoke("set_preset_debug_verbose", { enabled: false })`
- Activer trace USB :  
  `await window.__TAURI__.core.invoke("set_usb_trace_enabled", { enabled: true })`
- Delta-only USB :  
  `await window.__TAURI__.core.invoke("set_usb_trace_delta_only", { enabled: true })`
- Flags front live write :  
  `localStorage.setItem("models_live_write_probe", "1")`  
  `localStorage.setItem("models_live_write_enabled", "1")`  
  USB propriétaire (recommandé pour le write param HX Edit) :  
  `localStorage.setItem("models_live_write_transport", "usb_raw")`  
  MIDI (optionnel / fallback) :  
  `localStorage.setItem("models_live_write_transport", "midi_cc")`  
  `localStorage.setItem("models_live_midi_channel", "0")`  
  `localStorage.setItem("models_live_midi_cc", "20")`

**24 avril 2026** — **Split A/B** (`split_ab_route_to`) et **Split Y** (`split_balance`, alias **`pan`** dans `HelixControls.json`) : sur le **fil preset**, **`RouteTo`** / **`BalanceA`** / **`BalanceB`** (`io.models`) sont souvent des **floats normalisés [0, 1]** avec **101** pas (**0, 0.01, …, 1**) ; les libellés Helix (A100, A50, Even split, B50, B100, etc.) correspondent à **−100 … +100**. Le front applique **`v_helix = v × 200 − 100`** avant **`formatHelixFromControl`**, force **`step = 0.01`** sur le slider 0…1, et aligne l’**infobulle** sur le libellé Helix pour ces paramètres (**`helixNumericInputForSplitNormalized0To1`**, **`paramSliderHoverTitle`** dans **`src/models.ts`**). **Split Crossover** et **Split Dynamic** ne passent pas par cette conversion (échelle déjà cohérente).

**24 avril 2026 (fin de journée)** — **Panneau `.models` et catalogue** (`src/models.ts`, **`src/hxModelCatalogMeta.ts`**) : une fois l’**`id`** catalogue connu (**`chainHex` → `byHex`** ou **`catalogModelId`**), le **fichier** à charger (**`amp.models`**, **`preamp.models`**, etc.) est choisi via **`getPresetMetaForId(id).presetMeta.categoryName`** (même champ que dans **`HX_ModelCatalog.json`**, ex. **Amp** / **Preamp**), puis **`modelsDefinitionFileBasesForCategory`** ; **repli** sur **`slot.category`** seulement si **`categoryName`** est absent. La jointure **entrée JSON** reste **`symbolicID` = `id`** (jamais par nom d’affichage). **`loadModelsDefinitionArray`** appelle **`stripModelsDefinitionFilePreamble`** avant **`JSON.parse`** : enlève le BOM et tout texte **avant le premier `[`** ; les fichiers qui commencent déjà par un tableau JSON sont inchangés. Si le contenu n’est **pas** du JSON valide (ex. édition accidentelle du fichier), le parse échoue et **`findModelDefinitionForSlot`** peut afficher *aucune entrée .models* alors que l’**`id`** existe dans le dépôt. **À traiter en session suivante** : la **récupération des données pour les blocs Merge** (valeurs chaîne / panneau) n’est pas encore fiable côté produit ; côté Rust, un test **`merge_flow_segment_03_from_usb_capture_parses`** dans **`preset_chain_params.rs`** fixe une capture USB sur un segment merge **`0x03`** pour **`parse_flow_io_segment_params`**.

**Dernière mise à jour significative** : **avril 2026** — panneau paramètres : grille **nom | min | cellule (valeur + contrôle) | max** ; **en-tête** : ligne 1 = **titre catégorie** (`#models-params-pane-title`, ambre, toute la largeur) ; ligne 2 = **sous-tête** (nom modèle, **`basedOn`**, **`subCategory`** depuis le catalogue / CSV Line 6) à gauche + **icône catalogue** à droite + **aperçu au survol** ; **toggles Off/On** pour paramètres **bool** (`valueType === 2` ou `displayType` **`off_on`**) sans slider ; **`pickBasedOn`** / **`formatSubCategoryForHeader`** dans **`hxModelCatalogMeta.ts`** ; format **`.models`** étendu côté TS (**`min` / `max`** `number | boolean`, variantes **`displayType_stereo`**, **`min_stereo`**, **`max_stereo`**, **`default_stereo`** appliquées en signal **stéréo** pour l’affichage des bornes / défauts / formatage). **Jointure stricte** **`chainHex` → `id` catalogue → `.models.symbolicID`** (pas de fallback par nom pour la définition du modèle) ; **`hxModelCatalogMeta.ts`** : index séparés **`byHex`**, **`byId`**, **`byCategoryAndName`** pour éviter les collisions (ex. deux **Ping Pong** Delay avec **`chainHex`** différents). **Règles d’affichage des lignes** : liste des paramètres **filtrée par** les clés présentes dans **`HX_ModelCatalog.json`** (`params` du modèle catalogue), **ordre des lignes** = ordre du **`params[]`** du **`.models`** ; alignement des **valeurs** de chaîne sur l’**ordre catalogue** puis projection par **`symbolicID`** ; en **mono**, choix automatique avec/sans entrées **`stereo-only`** dans la séquence source si la longueur de chaîne ne colle pas (évite le décalage type **Bucket Brigade**) ; lignes **`stereo-only`** masquées en mono. Formatage « chaîne » via **`HelixControls.json`** ; table Rust **`HX_CATALOG_MODULE_BY_HEX`** / scripts **`scripts/`** ; **`chainHex` vides** à compléter à la main. **Amp+Cab / `module_hex` grille** : voir section dédiée ci-dessous (inférence **`ampHex` + `1a` + `cabHex`**, faux positifs **`c219` / `c319`**). **Prochaine session (UI matrice)** : **`Icons_line.png`** manquant ou incorrect sur **Path 2** (rangée L3) — voir section matrice.

**21 avril 2026** — alignement **valeurs chaîne ↔ lignes du panneau** pour les modèles dont les **`.models`** portent **`assign`** (ordre DSP ≠ ordre du tableau `params[]`, ex. ampli / préampli **Ch Vol** vs **Master**) : fonction **`alignChainValuesToModelParamOrder`** dans **`src/models.ts`** avant le rendu des lignes. Côté Rust, **`parse_assignable_segment_param_blocks`** accepte désormais un segment assignable en **`0x06` ou `0x08`** (comme la validation de la fenêtre Kempline à 20 segments). Commit local de synthèse : **`de27037`** (*Preset chaîne, catalogue HX et panneau paramètres*).

**22 avril 2026** — **Amp+Cab et `chainHex` pour la grille / le panneau** (`lib.rs`, tests unitaires dans le même fichier) : l’UI affiche *Jointure ID impossible : chainHex manquant pour ce slot* quand **`getCatalogModelIdForHex(slot.moduleHex)`** reçoit une chaîne vide — **`module_hex`** vient de **`extract_first_module_from_assignable_chunk`** (grille 16 via **`try_parse_preset_kempline_grid`**) ou du parseur de secours **`parse_preset_slots_internal`**. Travaux réalisés : (1) **Marqueur Amp+Cab** : **`is_amp_cab_assignable_chunk`** exige un segment **`0x06` ou `0x08`** contenant la fenêtre **`85 18 83 17 c3 19`** (aligné USB / Kempline ; les captures utilisent souvent **`0x08`**). (2) **Faux positif `c319`** : l’octet **`0x19`** final du motif **`83 17 c3 19`** n’est **pas** un début d’ID **`19…1a`** — on l’ignore **uniquement** dans ce contexte (ne **pas** élargir à tout **`c3`/`c2` + `19`**, car **`c3`/`c2`** encodent aussi des booléens dans **`read_params_hex`**). (3) **Faux positif `c219`** : en binaire, l’opcode **`c219`** est **`0xc2` + `0x19`** ; le scan des IDs **`19…1a`** peut donc croire voir un ID dont le préfixe hex coïncide avec le **type** du premier bloc **`c219`** (ex. seul **`cd0217`** au lieu du **`chainHex`** combiné catalogue **`cd02171acd0228`**). **`augmented_module_ids_for_assignable_chunk`** : si le segment est Amp+Cab, **`parse_assignable_segment_param_blocks`** rapporte **au moins deux** blocs **`c219`**, et l’extraction ne donne **qu’un** ID **identique** au type ampli inféré depuis les **`c219`**, on remplace par la **paire** inférée. (4) **Plusieurs `c219`** (ampli + bloc interne + cab, etc.) : **`infer_amp_cab_hex_pair_from_segment_hex_body`** parcourt **tous** les types d’argument **`c219`** dans le corps hex du segment ; paire = premier type classé **AmpLike** puis premier **CabLike** après dans le catalogue (**`HX_CATALOG_MODULE_BY_HEX` / `catalog_slot_kind_for_chain_hex`**), sinon repli **premier / dernier** préfixe 6 hex. **`inferred_amp_cab_hex_keys`** : si le parse à blocs échoue ou est incomplet, repli sur cette inférence **sans** compter sur **`blocks.len() == 2`**. (5) **`amp_cab_combined_chain_hex_for_slot_if_better`** : expose le **`chainHex`** combiné **`ampHex` + littéral `1a` + `cabHex`** quand il existe dans le catalogue et que l’ID extrait est vide ou égal au seul ampli. (6) **Catégorie affichée « Amp+Cab »** : si le catalogue porte **Preamp** / **Amp** / **Amp+Cab** pour l’entrée jointe, l’affichage grille force **Amp+Cab** lorsque le marqueur segment est présent. (7) **Parseur de secours** : si un segment assignable Amp+Cab ne produit **aucun** slot dans la boucle **`19…1a`** classique, on pousse le résultat de **`extract_first_module_from_assignable_chunk`** pour ne pas laisser un slot sans **`module_hex`** en vue **flow** (grille Kempline non reconnue). **État** : certains presets / firmwares peuvent encore échouer (catalogue sans entrée **`chainHex`** combinée, cas **`IR`**, ou bruit binaire) — à poursuivre avec un dump segment réel si besoin. Toujours ouvert : **IR**, longueurs de liste / champs internes vs **`params[]`**.

**23 avril 2026** — **validation terrain Amp+Cab, Amp+Cab+EQ, UI matrice et I/O** (Rust + TS + CSS + catalogue). Correctifs appliqués pendant la session : (1) **Amp+Cab sans perte d’ID** : extraction renforcée des paires ampli/cab via signatures **`c219`** et patrons **`19 ... 1a ... 09`** pour éviter les inversions (*amp seul*, *cab seul* ou mauvais `chainHex` cab). (2) **Amp+Cab avec EQ** : lecture des blocs **`0b/0c`** et saut du padding **`00 + num_params`** pour réaligner les valeurs (cas ampli **`cd0207`** + cab **`cd02f0`**). (3) **Formatage Helix** : prise en charge correcte des formats type printf avec texte littéral (ex. **`%.0f deg`** → **`45 deg`**) pour les paramètres comme **Angle** des cab/mic/IR. (4) **Matrice path 2** : correction de l’affichage de **`Icons_line.png`** entre split/merge, y compris le premier séparateur après l’input, et activation des interactions sur ces séparateurs. (5) **Input / Main L/R cliquables + paramètres** : ajout des pseudo-slots I/O et récupération explicite des valeurs côté Rust (**commande Tauri dédiée**). (6) **Jointure stricte par ID (jamais par nom)** : pour I/O, distinction explicite entre **`chainHex`** et **`slotTypeHex`** (affichage **`chainHex: — (slotType XX)`**), avec résolution modèle par **`catalogModelId`** quand le `chainHex` n’est pas disponible dans le segment.

## À quoi sert l’application

**HXLinux** est un éditeur / explorateur de presets pour **Line 6 HX Stomp XL** (et IDs USB voisins listés dans le code), sur **Linux**, en application **desktop Tauri** (Rust + front web).

Fonctions déjà utiles en pratique :

- Connexion **USB** au boîtier, machine d’états côté Rust pour le protocole (inspiré du travail **Kempline / helix_usb**).
- Lecture des **125 noms de presets**, **activation** d’un preset (Program Change), **renommage** depuis l’UI.
- Chargement du **contenu binaire du preset actif**, parsing partiel en **« slots »** (catégorie + nom) pour l’affichage.
- Mise en page type **grille** (16 blocs + routage), données renforcées par **`stomp_layout`** (split/merge, grille USB quand dispo).
- **Panneau paramètres** (sous la grille dans la vue models) : clic sur un bloc → définitions **`.models`** (noms, min, max) + valeurs **chaîne** lues dans le segment binaire du slot (**pas** de requête USB supplémentaire ; tout vient du dump déjà chargé). Les pastilles de la matrice 16 portent **`data-kempline-slot-index`** (0–7 path 1, 8–15 path 2) pour cette corrélation.

**État réel des valeurs chaîne** : (1) décodage aligné avec `user_slot_reader` Kempline dans **`preset_chain_params.rs`** (pointeur après le délimiteur `09`, même séquence que Python `bytes_read`). (2) **Affichage** : le front aligne la liste brute **`chainValues`** sur l’**ordre des `symbolicID` dérivé du catalogue** (`params` imbriqués dans **`HX_ModelCatalog.json`**), puis n’affiche que les paramètres autorisés par ce catalogue, dans l’**ordre du `.models`**. Répli historique : si le catalogue n’a **pas** de liste `params` pour un modèle, l’alignement retombe sur l’ordre **`assign` + sans `assign`** du `.models` complet. Cas encore sensibles : **IR**, champs internes absents du catalogue ; **Amp+Cab** : logique d’inférence **`module_hex`** documentée au **22 avril 2026** (reste perfectible selon preset / catalogue).

Ce qui reste largement ouvert : **édition** des paramètres vers l’appareil, export/import de fichiers presets (voir `README.md`).

---

## Lecture des paramètres « dans la chaîne » (ce qui a été fait — avril 2026)

Les **valeurs** affichées dans la colonne **chaîne** ne viennent **pas** des fichiers `.models` : elles sont **décodées dans le binaire du preset** déjà reçu en USB (`RequestPreset` → accumulation dans `HelixState.preset_data`). Les `.models` fournissent seulement les **métadonnées** (nom du paramètre, min/max du slider HX Edit, défaut, `displayType`, etc.).

### Chaîne de traitement (Rust)

1. **`split_preset_by_8213`** (`lib.rs`) — découpe le flux en segments (marqueur `82 13` côté octets, équivalent au split hex `8213` chez Kempline).
2. **`kempline_grid_window_start_and_seg_count`** — retrouve la **fenêtre de 20 segments** validée comme grille Kempline (même critères que `try_parse_preset_kempline_grid` : segment `00`, `01`, `02`, `03` aux positions attendues, 16 blocs assignables en `06` ou `08`).
3. **`kempline_assignable_segment_bytes(data, slot_index)`** — pour un index **0…15** (ordre UI : path1 slots 0–7, path2 slots 8–15), renvoie le **segment brut** `&[u8]` correspondant à `KEMPLINE_ASSIG_INDICES[slot_index]`.
4. **`parse_assignable_segment_param_blocks`** (`preset_chain_params.rs`) — segment dont le premier octet est **`0x06` ou `0x08`** ; même recalage **`br`** / **`read_params_hex`** que **`user_slot_reader`** + **`read_params`** Kempline (`simple_filter.py`) après le motif **`85188317`** : un ou plusieurs blocs **`c219`** (cas standard **`c219`** seul ; Amp+Cab **`c319`** puis plusieurs **`c219`**). **`chain_param_values_for_assignable_segment`** dans **`lib.rs`** choisit le bloc **ampli** vs **cab** en classant chaque bloc par **`chainHex`** / catégorie catalogue (`HX_CATALOG_MODULE_BY_HEX`, `MODEL_ID_BY_HEX`, candidats hex avant **`1a`** si besoin). **Grille 16 — même binaire** : **`extract_first_module_from_assignable_chunk`**, **`augmented_module_ids_for_assignable_chunk`**, **`inferred_amp_cab_hex_keys`**, **`infer_amp_cab_hex_pair_from_segment_hex_body`** (voir paragraphe **22 avril 2026** en tête de fichier) alimentent **`module_hex`** pour **`get_active_preset_slots`** et la jointure TS **`getCatalogModelIdForHex`**.

Les valeurs renvoyées au front sont une liste **`ChainParamValue`** (sérialisation JSON **untagged** : booléen, nombre, ou chaîne hex pour les blobs).

### Chaîne de traitement (TypeScript)

1. Chaque pastille de la grille 16 a **`data-kempline-slot-index="0"` … `"15"`** (`gridSlotNode` dans `models.ts`).
2. Au clic, **`loadAndShowModelsParamsForSlot`** appelle **`invoke("get_active_preset_slot_chain_param_values", { slotIndex })`** (si l’index est défini), puis charge le JSON **`.models`** (cache + `read_models_definition_file` / fetch selon l’environnement).
3. **`getCatalogModelIdForHex(slot.moduleHex)`** (`hxModelCatalogMeta.ts`) résout l’**`id`** catalogue depuis **`presetMeta.chainHex`** (lookup **`byHex`**, pas de collision entre deux modèles même nom). Si aucun id : message d’erreur explicite (pas de contournement par nom pour charger le `.models`).
4. **`getPresetMetaForId(id)`** fournit **`presetMeta.categoryName`** pour choisir le ou les fichiers **`*.models`** (voir **24 avril 2026 (fin de journée)** ci-dessus).
5. **`findModelDefinitionForSlot`** charge le **`.models`** et ne retient que l’entrée dont **`symbolicID`** = **`id`** catalogue (**jointure stricte par id**).
6. **`renderModelsParamsPane`** : extrait l’ordre des **`symbolicID`** depuis **`HX_ModelCatalog.json`** (`getCatalogParamOrderForId`) ; **filtre** le **`params[]`** du `.models` pour ne garder que ces id ; **trie** les lignes dans l’**ordre du `.models`** ; **`alignChainValuesToModelParamOrder`** mappe **`chainValues`** sur la séquence catalogue (avec ou sans entrées **`stereo-only`** en mono selon la longueur reçue), puis projette par **`symbolicID`** sur les lignes affichées. **Masquage mono** : parmi les lignes affichées, celles avec **`"stereo-only": true`** sont omises si le signal est **mono** (`pickSignal` + `moduleHex`).

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

Pour les `displayType` **non** couverts par `HelixControls.json` (voir section suivante), la colonne **chaîne** reste une **vue brute** (ou légèrement formatée : bool on/off, float arrondi côté Rust) — d’où les écarts d’échelle par rapport à l’écran du Helix.

---

## Panneau paramètres — déjà traité dans `src/models.ts` (mémo pour ne pas refaire la demande)

Tout ceci concerne la vue **models** : grille + panneau **nom | min | cellule | max**. La **cellule** centrale contient soit un **slider** (valeur formatée Helix au-dessus + `<input type="range">`), soit une paire de **boutons Off / On** pour les paramètres booléens (voir ci-dessous). La **valeur brute JSON** de la chaîne reste en **infobulle** sur la ligne (et sur le curseur). Les colonnes sont alignées via **`display: grid` sur `ul.models-params-list`** et **`grid-template-columns: subgrid` sur chaque `li.models-params-row`** ; sans support **subgrid**, repli sur une **table** HTML (`display: table`) à colonnes resserrées.

### En-tête du panneau (`#models-params-pane-header`) — grille 2 colonnes × 2 lignes (titre sur toute la ligne 1)

| Cellule | Contenu | Alignement / style |
|---------|---------|---------------------|
| **(1,1)–(2,1)** | Titre **catégorie** du slot (`#models-params-pane-title`, pleine largeur) | Gauche, **`var(--amber)`** |
| **(1,2)** | Sous-tête modèle (nom catalogue, **`basedOn`**, **`subCategory`**, nom USB si différent, etc.) | Gauche |
| **(2,2)** | **Icône** (`icons_models/` ou repli `icons_category/`) | Droite ; **survol** → popover **`position: fixed`** sur `body` avec la même URL, image **`width`/`height: auto`** jusqu’à **`max-width: 90vw`** / **`max-height: 85vh`** (évite les PNG énormes hors écran) ; fermeture avec léger délai + survol de la popover |

Structure HTML : **trois** blocs enfants directs du `header` (ordre DOM : titre, subhead, wrap icône) ; le placement repose sur **`grid-column` / `grid-row`** en CSS (`src/styles.css`, classes **`.models-params-pane-*`**).

### Format des paramètres (`.models` ↔ ligne UI)

Chaque entrée **`params[]`** est un objet **`ModelParamDefJson`** côté `models.ts` :

- **`symbolicID`**, **`name`**, **`assign`** (optionnel, entier — ordre DSP côté firmware ; sert surtout de **repli** d’alignement quand le catalogue n’a **pas** de liste `params` pour ce modèle), **`displayType`** (clé vers **`HelixControls.json`** quand elle existe).
- **Variantes stéréo** (affichage uniquement, quand le signal catalogue est **stéréo**) : **`displayType_stereo`**, **`min_stereo`**, **`max_stereo`**, **`default_stereo`** remplacent respectivement **`displayType`**, **`min`**, **`max`**, **`default`** pour bornes, défaut et formatage Helix.
- **`valueType`** (usage Line 6) : **`0`** = entier pas slider / incréments entiers ; **`1`** = float ; **`2`** = **bool** (souvent avec **`displayType`** `off_on` côté Helix).
- **`min` / `max`** : en JSON Line 6 ce sont le plus souvent des **nombres** ; pour les bool **`off_on`** le fichier peut porter **`false` / `true`** — le front les accepte (**`number | boolean`**) pour l’affichage des bornes et la logique slider (**slider** uniquement si min/max sont des nombres avec **`max > min`**).
- **`"stereo-only": true`** : ligne **non affichée** en **mono** parmi les paramètres autorisés par le catalogue ; la valeur peut toutefois être présente dans la chaîne binaire (d’où la logique d’alignement avec/sans ces entrées en mono).
- **`default`** : peut être nombre, chaîne ou bool selon le modèle ; **`default_stereo`** si le défaut diffère en stéréo.

**Valeurs chaîne** (`invoke` → **`ChainParamValueJson`**) : **`boolean`**, **`number`**, ou **`string`** (hex blob). Pour l’**UI bool** : la cellule affiche les **boutons Off/On** si la valeur se lit comme bool (**`true`/`false`**) **ou** entier **`0`/`1`** *et* (**`valueType === 2`** **ou** `displayType` normalisé en **`off_on`**). Libellés des boutons : tableau **`format`** à deux chaînes dans **`HelixControls.json`** pour ce `displayType`, sinon défaut **Off / On**. Les clics mettent à jour **l’aperçu local** (texte chaîne + infobulle) en respectant le type d’origine (**bool** vs **0/1**) ; **aucune** écriture vers le Helix (idem que le slider d’aperçu).

**Formatage affiché** (`formatChainParamValueJson`) : les **bool** passent par **`formatHelixFromControl`** quand le `displayType` a une entrée Helix (ex. index 0/1 sur tableau `format`) ; sinon repli **`on` / `off`**.

### Jointure catalogue ↔ `.models`

- **Stricte** : le **`module_hex`** du slot → **`getCatalogModelIdForHex`** lit **`presetMeta.chainHex`** dans **`HX_ModelCatalog.json`** via une map **`chainHex` → entrée** (**`byHex`**), puis l’**`id`** catalogue. En **Amp+Cab**, le binaire joint souvent une seule chaîne **`ampHex` + `1a` + `cabHex`** (cf. **`cab_info_from_module_id`** dans **`lib.rs`** et commentaires équivalents dans **`hxModelCatalogMeta.ts`** ; candidats de lookup côté TS : chaîne complète puis préfixe avant **`1a`**).
- **Fichier `.models`** : après résolution de l’**`id`**, **`presetMeta.categoryName`** sur cette entrée catalogue pilote **`modelsDefinitionFileBasesForCategory`** (ex. **Amp** → **`amp.models`** seulement). Repli sur la catégorie affichée du slot si **`categoryName`** manque.
- **`.models`** : une seule entrée dont **`symbolicID`** = cet **`id`** (**pas** de résolution par nom pour cette étape : deux modèles peuvent partager le même **`name`** avec des **`symbolicID`** et paramètres différents).
- Métadonnées **`presetMeta`** (`basedOn`, `subCategory`, `chainHex` parallèle, etc.) : **`src/hxModelCatalogMeta.ts`** expose aussi **`byId`** (métadonnées + ordre des paramètres catalogue) et **`byCategoryAndName`** (vue historique ; en cas de doublon nom+catégorie, seule la **première** entrée est gardée pour cette clé — la jointure **`chainHex` → id** ne dépend pas de cette map).

### Règles d’affichage : qui apparaît, dans quel ordre

- **Liste affichée** : intersection entre le **`params[]`** du **`.models`** et les **`symbolicID`** listés dans le champ **`params`** du modèle dans **`HX_ModelCatalog.json`** (parcours récursif des objets `{ "SymbolicID": null }` pour produire un ordre de clés). Si le catalogue **ne** définit **pas** de `params` pour ce **`id`**, on affiche **tout** le **`params[]`** du `.models` (comportement de repli).
- **Ordre des lignes** à l’écran : **ordre du `.models`** parmi les paramètres retenus (pas l’ordre du catalogue).
- **Deuxième règle (signal)** : en **mono**, masquer les lignes avec **`"stereo-only": true`** (définition toujours lue dans le `.models`).

### Alignement liste `params` ↔ valeurs chaîne

- **Source d’ordre pour le zip** : quand le catalogue fournit une liste de **`symbolicID`**, **`alignChainValuesToModelParamOrder`** considère cette liste comme l’ordre des valeurs successives dans **`chainValues`** (en **mono**, deux variantes : avec ou sans les id marqués **`stereo-only`** dans le `.models` ; on retient celle dont la longueur est la plus proche de **`chainValues.length`**). Les valeurs sont ensuite assignées aux lignes affichées par **`symbolicID`**.
- **Repli** : catalogue sans `params` → ordre source = **`assign`** croissant puis paramètres **sans** **`assign`** dans l’ordre du **`params[]`** complet du `.models` (même logique mono avec/sans **`stereo-only`**).
- Cela corrige les décalages où l’ordre DSP / JSON **`.models`** ne coïncide pas avec l’ordre réel des valeurs (ex. sync avant **Time** dans le catalogue, **Cosmos Echo**).

### Source des règles d’affichage « chaîne »

- Fichier **`src-tauri/resources/HelixControls.json`** chargé côté front (fetch sur `/src-tauri/resources/HelixControls.json`), cache en mémoire.
- Les clés du JSON correspondent au **`displayType`** du paramètre dans le `.models`.

### Formatage « chaîne » via `HelixControls.json` (pipeline générique)

Pour toute valeur **numérique** dont le **`displayType`** est une clé présente dans `HelixControls.json`, `src/models.ts` applique **`formatHelixFromControl`** :

1. **Exception** optionnelle : objet **`HELIX_DISPLAY_EXCEPTIONS`** (clé = `displayType`) pour court-circuiter le générique si un cas ne colle pas.
2. **`format` = tableau de chaînes** (ex. `off_on`, `sync_note`) → libellé par index **`Math.round(valeur)`** (borné au tableau).
3. **`format` = tableau d’objets** (`lowerBound` / `upperBound`) → choix de la plage **`[lower, upper)`** sur la **valeur brute**, puis fusion des champs `format` / `formatUnits` / `unitsMultiplier` de la plage ; si `format` n’est **pas** un motif `%.…f` → texte littéral (**`Off`**, etc.).
4. Sinon **`format` chaîne** : `valeur × dspToDisplayScale` (si défini), puis `× unitsMultiplier` (si défini), puis **`format`** (`%.…f`) et substitution dans **`formatUnits`** si elle contient un token `%.…f` ; les séquences **`%%`** dans `formatUnits` deviennent un **`%`** littéral (comme sprintf).
5. Sinon entrée **`isDiscrete: true`** sans `format` exploitable → affichage **`Math.round(valeur)`** ; sinon repli numérique brut.

Détails d’implémentation utiles :

- **`alias`** dans `HelixControls.json` (ex. `time_ms_20_1800` → `time_ms`) : résolu **au chargement** ; la map expose la définition complète pour chaque clé.
- **Plages `format[]` + `dspToDisplayScale`** (ex. `time_ms`) : le choix de la plage utilise **`valeur_brute × dspToDisplayScale`** (unité d’affichage, ex. ms), puis on réapplique le même facteur pour le formatage final — les bornes du JSON sont alignées sur l’affichage, pas sur le brut secondes.

Les cas déjà validés manuellement (**`generic_knob`**, **`generic_knob_1to1`**, **`frequency`**, **`eq_low_cut`**, **`eq_high_cut`**) restent couverts par ce même moteur ; tout autre `displayType` présent dans Helix et dans les `.models` est **automatiquement** formaté selon sa définition JSON (sauf exception ajoutée dans `HELIX_DISPLAY_EXCEPTIONS`).

### Split A/B et Split Y : échelle sur fil **0…1** vs Helix **−100…+100**

Pour les blocs **Split A/B** et **Split Y**, le binaire / JSON de chaîne expose souvent **`RouteTo`** et les **balances** comme un **réel dans [0, 1]** (pas comme l’entier ou l’échelle **−100…+100** lue sur l’écran du Helix). Exemples côté fil : **A 100** → **`0`**, **A 50** → **`0.25`**, **Even split** → **`0.5`**, **B 50** → **`0.75`**, **B 100** → **`1`** — soit **101** valeurs discrètes si le pas est **0.01**.

Les entrées **`HelixControls.json`** (**`split_ab_route_to`**, et **`split_balance`** qui pointe sur les bandes **`pan`**) décrivent les libellés sur l’**axe Helix −100…+100**. Dans **`src/models.ts`**, pour le **formatage** (colonne chaîne, min/max affichés via `formatParamBoundForDisplay` quand Helix s’applique) :

1. Si le paramètre a l’un de ces **`displayType`** et que la valeur numérique est dans ~**[0, 1]**, on passe **`formatHelixFromControl(v × 200 − 100, …)`** (linéaire : 0 → −100, 0.5 → 0, 1 → +100).
2. Si la valeur est **hors** de cette plage, on ne modifie pas le nombre (évite d’altérer un autre cas où le même contrôle serait déjà en unités Helix).

Le **slider d’aperçu** du panneau paramètres garde **`min` 0** et **`max` 1** issus du **`.models`** (`io.models`), avec **`step = 0.01`** forcé pour ces `displayType` (l’incrément déduit du JSON Helix ne correspond pas au domaine 0…1). **Split Crossover** et **Split Dynamic** : pas cette étape — leurs valeurs chaîne suivent déjà l’échelle attendue par Helix.

### UI debug

- **Infobulle** sur chaque ligne (et sur le curseur d’aperçu) : en règle générale la **valeur brute JSON** reçue de la chaîne. **Exception** : **`split_ab_route_to`** et **`split_balance`** avec **`min` 0 / `max` 1** — l’infobulle reprend le **libellé Helix** (comme la colonne chaîne), tout en restant sur le fil en **0…1** pour la position du slider.
- **Logs jointure ID** : `localStorage.setItem("models_debug_id_join", "1")` → `console.warn` si aucune entrée **`.models`** ne correspond à l’**`id`** catalogue résolu (après essai des fichiers par catégorie).
- **TODO** : mode debug optionnel (longueur `chainValues`, stratégie mono, variante stéréo par param) — voir **`TODO.md`**.

### Flags front utiles (session actuelle)

- **Trace sync UI / flash** : `localStorage.setItem("models_debug_sync_trace", "1")` dans la fenêtre **Models** → **`[ModelsSync]`** dans la console Web **et** le terminal Tauri (`log_frontend_message` → `eprintln!`).
- **Re-dump preset USB depuis soft-sync (optionnel)** : par défaut **pas** de poll (clé absente). `localStorage.setItem("models_hw_usb_preset_poll_ms", "2500")` active un re-dump périodique (bornes **500..120000** ; **`0`** = explicite « jamais »).
- **Sync matériel périodique (front)** : `localStorage.setItem("models_hw_sync_interval_ms", "200")` (borne code : **100..5000 ms**).  
  - `0` ou clé absente = désactivé ; recommandé : **200 ms**.
- **Write live (expérimental)** : `localStorage.setItem("models_live_write_probe", "1")`  
  - avec `models_live_write_enabled` absent ou ≠ `"1"` : invoke `probe_live_param_write` (log seulement).  
  - avec `models_live_write_enabled = "1"` : invoke `write_live_param` (USB brut) **ou** `write_live_param_midi_cc` si `models_live_write_transport = "midi_cc"` (voir bloc **26 avril 2026 (fin de soirée)** + commandes ci-dessus).
- **Désactivation rapide** : `localStorage.removeItem("models_hw_sync_interval_ms")` et/ou `localStorage.removeItem("models_live_write_probe")` et/ou `models_live_write_enabled` / `models_live_write_transport`.

Notes implémentation :
- Le cycle 200 ms est en mode **soft refresh** (`runHardwareSyncSoftRefresh` dans `src/models.ts`) :  
  - **grille** : **`renderSlots`** uniquement après un **`request_preset_content`** dans ce cycle (relecture = RAM `preset_data` à jour), ou au **chargement preset** (`requestLoadForPreset`) — pas de re-parse `get_active_preset_slots` entre deux lectures pour reconstruire la matrice ;  
  - **panneau params** / **sélection HW** : snapshot **`lastHwSyncNormalizedSlots`** + rafraîchissement in-place quand le slot sélectionné reste le même (évite flash).

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
│   │   ├── lib.rs          # Commandes Tauri, AppState, parse preset, fenêtre Kempline 20 segments, Amp+Cab / `module_hex`, invoke
│   │   ├── preset_chain_params.rs  # parse segment slot 0x06|0x08 : 85188317 / c219 / read_params (serde → UI)
│   │   ├── stomp_layout.rs # Layout stomp + routing (split/merge cols) aligné USB / heuristiques
│   │   └── helix/          # Protocole : modes, USB, paquets ; `live_write.rs` + `live_write_config.rs`, ressource `resources/HelixLiveWrite.json`
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
| `get_active_preset_slots` | Slots **`[catégorie, nom, module_hex]`** (triplet JSON) quand le dump est prêt **et** cohérent avec `active_preset` ; **`module_hex`** = chaîne entre **`19…1a`** ou **`ampHex` + `1a` + `cabHex`** inféré pour Amp+Cab (voir **22 avril 2026**). |
| `get_active_preset_slots_debug` | Idem + coords grille debug. |
| `get_active_preset_routing_markers` | Entrées routing type Split/Merge si présentes dans le parse. |
| `get_active_preset_stomp_layout` | Objet `ActivePresetStompLayout` (grille OK, split/merge cols, etc.). |
| **`get_active_preset_slot_chain_param_values`** | **`{ slotIndex: 0..15 }`** → `Vec<ChainParamValue>` ou `null` : valeurs décodées `read_params` pour le segment assignable Kempline du slot (voir `preset_chain_params.rs`). |
| `read_models_definition_file` | Lecture d’un `resources/models/{base}.models` côté bundle (nom de base alphanumérique). |
| `get_preset_data_hex` | Dump brut hex (debug). |
| `request_active_preset_name` | Resync nom preset actif. |
| `probe_live_param_write` | Write live expérimental : log UI→backend (pas d’USB write). |
| `write_live_param` | Write live USB ED03 : bool/discret **`23`**, float **`27`** (jambe B = valeur chaîne si `chainMin`/`chainMax`) ; voir **6 mai 2026 (soir)** + `HelixLiveWrite.json`. |
| `write_live_param_midi_cc` | Write live via **MIDI CC** sur endpoint USB `0x02` (nécessite Controller Assign). |

Le flux côté `models.ts` : après changement de preset → `request_preset_content` → boucle d’attente → `get_active_preset_slots` + routing + `get_active_preset_stomp_layout` pour `renderGrid16`. Au clic sur un slot avec modèle → `get_active_preset_slot_chain_param_values` si `data-kempline-slot-index` est défini, fusion avec le JSON `.models` chargé (fetch ou `read_models_definition_file`).

## Fichiers Rust à connaître pour le preset / UI grille

- **`lib.rs`** — `parse_preset_slots`, `split_preset_by_8213`, `kempline_grid_window_start_and_seg_count`, `kempline_assignable_segment_bytes`, `try_parse_preset_kempline_grid`, `KEMPLINE_ASSIG_INDICES`, **`is_amp_cab_assignable_chunk`**, **`extract_first_module_from_assignable_chunk`**, **`augmented_module_ids_for_assignable_chunk`**, **`inferred_amp_cab_hex_keys`**, **`infer_amp_cab_hex_pair_from_segment_hex_body`**, **`amp_cab_combined_chain_hex_for_slot_if_better`**, tests **`assignable_*`** / **`extract_first_module_amp_cab_inference_tests`**, commentaires `[PresetDebug]`.
- **`preset_chain_params.rs`** — `parse_assignable_segment_param_blocks`, `read_params_hex`, enum sérialisable `ChainParamValue` (bool, float IEEE via `ca`, u8, blob `1bda`).
- **`stomp_layout.rs`** — `split_merge_from_usb_preset_body`, `compute_stomp_layout_from_kempline_grid_with_usb`, tests ; colonnes split/merge consommées par `models.ts`. Le helper `merge_after_col_from_usb_preset_body` n’existe qu’en build test (`#[cfg(test)]`) pour éviter un warning `dead_code` en `cargo build` lib.

## Front — matrice stomp 4×20 (`renderGrid16` dans `models.ts`)

Grille **20 colonnes × 4 lignes**, cellules **56×56 px** (`NUM_COLS = 20`, `NUM_ROWS = 4`, `CELL_PX = 56`). Nomenclature des lignes dans le code :

| Ligne | Rôle |
|-------|------|
| **L1** | Path 1 — slots 0–7, I/O Input / Output, traits horizontaux **`Icons_line.png`** entre colonnes paires, pastille `Icons_split_merge.png` aux colonnes **jonction** (split/merge issus du routing). |
| **L2** | Description Path 1 — textes catégorie ; aux colonnes split/merge, petite barre verticale `Icons_vertical_line.png`. |
| **L3** | Path 2 — slots 8–15 si branche B ; aux mêmes colonnes, icônes coin **`Icons_link_split.png`** / **`Icons_link_merge.png`** (alignées sur `stomp_layout`). **À corriger** : réintroduire ou aligner les **traits horizontaux `Icons_line.png`** sur cette rangée (équivalent visuel L1) — actuellement **manquant / incomplet sur Path 2** ; l’asset est dans **`src-tauri/resources/icons_category/Icons_line.png`**. |
| **L4** | Description Path 2 — catégories path B. |

- **Colonne 20** : numéros de ligne grille (debug lisible).
- **Colonnes « jonction »** : dérivées des frontières split/merge (1…8) via `matrixEvenColForRoutingBoundary` (colonnes paires 2…18 côté UI).
- **`ENABLE_MATRIX_VSPAN_ON_PATH2`** (`models.ts`) : par défaut **`false`**. Un overlay `vspan` vertical sur Path 2 partageait la même `grid-area` que les icônes lien ; les deux se superposaient. Laisser à `true` uniquement pour un revert visuel expérimental (commentaires **REVERT** à côté).
- **Ancienne mise en page (5 lignes + rangée 3 « séparateur » 0 px)** : le retour est documenté en blocs commentés **REVERT** dans `models.ts` et `styles.css` (constantes de lignes, hauteurs de rangées, boucle séparateur, classes `row-line-debug-sep`, etc.).

Panneau paramètres : **`ul.models-params-list`** = grille à 4 colonnes partagées ; chaque **`li.models-params-row`** = **subgrid** sur ces colonnes, enfants directs **nom | min | cellule | max** (valeur formatée dans **`.models-params-slider-cell`** : slider **ou** **`.models-params-bool-toggle`** + **`.models-params-bool-btn`** ; classes `.models-params-row-min`, `-chain`, `-max`).

Le CSS associé est sous **`.models-pane .hx-matrix-*`** et **`.models-params-*`** dans `styles.css`. Des régressions visuelles passent souvent par : parent sans `.models-pane`, ou styles inline dupliqués dans `models.html` vs `styles.css`.

## Ressources et métadonnées Line 6

- **`src-tauri/resources/HX_ModelCatalog.json`** — catalogue modèles.
- **`src-tauri/resources/icons_models/`** — icônes par modèle.
- **`src-tauri/resources/icons_category/`** — icônes catégories + assets maison pour la matrice : `Icons_line.png`, `Icons_split_merge.png`, `Icons_vertical_line.png`, `Icons_link_split.png`, `Icons_link_merge.png`, ainsi que les icônes I/O (`icon-input-category.png`, etc.).
- **`src-tauri/resources/models/*.models`** — définitions JSON Line 6 (params, min/max, `displayType`, `valueType`, etc.) ; utilisées pour le panneau paramètres et le matching id catalogue ↔ `symbolicID`.
- **`src-tauri/resources/HelixControls.json`** — données controls (fichier ajouté au bundle ; brancher dans l’app si besoin).

Chemins côté front pour les PNG sous Tauri : souvent `/src-tauri/resources/...` (comme dans `models.ts` pour les I/O).

### Catalogue HX — `presetMeta`, `chainHex`, mono / stéréo (mémo session)

- Chaque modèle du JSON peut porter un objet **`presetMeta`** : notamment **`chainHex`** (une chaîne hex **ou** un tableau **`[mono, stéréo]`**) et **`signal`** en parallèle (`["mono", "stereo"]`) quand le même bloc existe en deux variantes.
- **`src-tauri/src/lib.rs`** : au build, **`HX_CATALOG_MODULE_BY_HEX`** est rempli **uniquement** depuis **`include_str!("../resources/HX_ModelCatalog.json")`** en parcourant tous les `presetMeta.chainHex` (chaîne ou tableau) + nom court du modèle. C’est cette table qui sert à résoudre l’UID hex du segment preset vers **catégorie + nom** affichés.
- **`src/hxModelCatalogMeta.ts`** : `fetch` du catalogue sous `/src-tauri/resources/HX_ModelCatalog.json` (cache au premier chargement ; **recharger l’app** après édition du JSON). Trois index en mémoire : **`byHex`** (`chainHex` → entrée, jointure slot → **`id`**), **`byId`** (**`id`** → `presetMeta`, image, **`catalogParamOrder`** = liste ordonnée des **`symbolicID`** extraits du champ **`params`** du catalogue), **`byCategoryAndName`** (première entrée par paire catégorie+nom, usages historiques). Helpers **`getCatalogModelIdForHex`**, **`getCatalogParamOrderForId`**, **`getPresetMetaForId`**, **`pickSignal` / `pickBasedOn` / `formatSubCategoryForHeader`**.
- **`scripts/apply_mono_stereo_pairs_to_catalog.py`** : lit **`HX_ModelCatalog.json`**, détecte les paires mono/stéréo à partir des fiches qui ont déjà un **`chainHex`** et un libellé **`(mono)`** / **`(stereo)`** / **`(stéréo)`**, met à jour la fiche : `chainHex` + `signal` en tableaux.
- **`scripts/enrich_catalog_preset_meta.py`** — complète les champs texte vides de **`presetMeta`** depuis le seul champ **`name`** du modèle (ne remplit pas **`chainHex`**).
- **Travail restant (manuel)** : compléter les entrées **`"chainHex": ""`** dans **`HX_ModelCatalog.json`** (compter avec `rg '"chainHex":\\s*""' src-tauri/resources/HX_ModelCatalog.json`) en lisant l’hex sur le boîtier ou une autre source fiable.

**Note** : une copie **`External files/HX_ModelCatalog.json`** peut exister hors bundle ; les commits UI « légers » peuvent l’**exclure** (diff très volumineux) — resynchroniser à la main si tu t’en sers comme miroir.

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

**Commits / contexte** : sur la branche **`refactor/multithread`**, le commit **`f79be40`** reste une référence pour `preset_chain_params` + première itération UI min | chaîne | max. Un commit local ultérieur (**`dd9ee9f`**, message **`feat(models): panneau paramètres et en-tête catalogue`**) regroupe notamment **`index.html`**, **`models.html`**, **`src/models.ts`**, **`src/styles.css`**, **`src/hxModelCatalogMeta.ts`** (en-tête 2×2, toggles bool, aperçu survol icône, etc.). Le **21 avril 2026**, commit **`de27037`** (*Preset chaîne, catalogue HX et panneau paramètres*) : alignement **`assign`** côté TS, segments **`0x06|0x08`**, évolutions **`lib.rs`** / catalogue / scripts / styles, suppression **`modules_by_id.json`**. Les gros diffs **`HX_ModelCatalog.json`** / **`TODO.md`** / **`description.md`** peuvent rester hors commit jusqu’à message dédié — voir **`git log`** / **`git status`**.

Todo : 
  * ⬜ **`HX_ModelUsbAssign.json`** : compléter les **autres modèles** (captures + entrées) ; **revoir la structure** — champs **`edOpcode`** / **`bulkKind`** (et **`chainHexHint`**) non utilisés par le chargeur Rust (**seuls** `id`, `variant`, `bulkHex`) ; picker TS utilise `name` / `category` / `subCategory`. **Importer `basedOn` et `image`** depuis **`HX_ModelCatalog.json`**. **Note** : fusion USB long `chainHex` lit encore le catalogue ; **`chainHexHint`** non branché — et ordre picker = ordre fichier → voir **`hardwareOrder`** / **`TODO.md`**.
  * ⬜ **Refactor nommage « Kempline » (à planifier)** : le code et l’UI portent encore beaucoup de **`kempline_*`**, **`KemplineCell`**, **`data-kempline-slot-index`**, commande **`get_active_preset_kempline_flow_*`**, etc. — héritage du vocabulaire du projet open source **[kempline/helix_usb](https://github.com/kempline/helix_usb)**. HXLinux s’en est **éloigné** sur le fond (USB slot focus, live write, anti-flash, etc.). **Objectif** : renommer vers un lexique neutre (**grille 16**, **`preset_slot_index`**, **`grid16_*`**, etc.) et documenter dans **`README.md`** / commentaires que le reverse initial s’inspire de helix_usb **sans** impliquer que chaque ligne reste comparable aux analyses Kempline — pour éviter qu’un contributeur croie à tort à une erreur en comparant mot à mot. Voir aussi **`TODO.md`**.
  * ✅ **Merge (traité le 25 avril 2026)** : récupération fiabilisée des valeurs chaîne et du panneau paramètres pour les blocs **Merge**. Correctif Rust dans **`parse_info_slot_block_value_bytes`** (**`preset_chain_params.rs`**) : validation stricte de la signature bloc flow **`83 02 <num_params>`** après marqueur `0x07` pour éviter les faux positifs et préserver le typage bool de **B Polarity** (**Normal / Invert**). Couvert par **`merge_flow_segment_03_from_usb_capture_parses`**.
  * ✅ **Campagne de validation manuelle (25 avril 2026)** : tous les modèles **Distortion / Mono** ont été testés sur le hardware et validés côté lecture (remontée des valeurs OK dans l’UI).



## Reprise rapide après redémarrage

1. Lire **`README.md`** + ce **`description.md`**.
2. Lancer **`npm run tauri dev`** (ou `npm run dev` pour le front seul sur `http://localhost:1420`).
3. Pour toute modification UI models : **`src/models.ts`** + **`src/styles.css`** ; vérifier que **`models.ts` importe bien `./styles.css`** si tu travailles sur `models.html`.
4. Pour protocole / parsing preset / valeurs chaîne : **`src-tauri/src/lib.rs`** + **`preset_chain_params.rs`** + **`stomp_layout.rs`** + modules **`helix/`**.


## 28 avril 2026 (soir) — sync slot actif + stabilité changement preset

### Ce qui a été implémenté

- **Sync slot actif hardware -> UI** branché et validé:
  - parsing côté Rust de `82 62 SS 1a` dans le flux IN (`helix/mod.rs`) avec mapping bus -> index UI,
  - expose `get_active_hardware_slot_state`,
  - côté front (`models.ts`), application de la sélection du slot quand un nouvel événement hardware est détecté.
- **Sync slot UI -> hardware** ajouté via commande dédiée:
  - `switch_active_hardware_slot` (`lib.rs`) envoie la trame `1d ... 80 10 ed 03 ... 83 66 cd 03 ... 82 62 SS 1a ...`,
  - garde-fou pour ignorer pendant `preset_content_only` (lecture preset en cours).
- **Mapping path 2 corrigé**:
  - Path 1: index 0..7 -> bus `0x01..0x08`,
  - Path 2: index 8..15 -> bus `0x0b..0x12`,
  - appliqué en lecture et en écriture (`helix/mod.rs`, `lib.rs`, `helix/live_write.rs`).
- **Diag USB enrichi**:
  - commande `set_usb_io_diag` pour tracer OUT/IN, type de paquet, compteur, succès/erreur.

### Problèmes observés pendant la session

- Après plusieurs changements de preset, la lecture peut se bloquer.
- Logs vus:
  - `UsbWriter erreur écriture : Operation timed out`
  - `ModeLoop ignore RequestPresetName while content_only`
- Le blocage n'est pas lié à un mauvais layout du preset précédent, mais à un **embouteillage d'écritures** pendant les phases sensibles.

### Ce qu'on a tenté pour stabiliser

- Pause explicite de la boucle front 200 ms pendant `requestLoadForPreset` (reprise en fin de lecture preset).
- Suppression du fallback "sélection auto du premier slot UI" (ça désynchronisait avec le hardware).
- Désactivation des keepalive **x80** pendant `preset_content_only` (`helix/keep_alive.rs`) pour réduire les timeouts sur `out_keepalive_x80_or_ed03`.

### Point important à vérifier à la reprise

- Reproduire le scénario "enchaînement de changements de preset" avec `set_usb_io_diag` actif.
- Confirmer si les `Operation timed out` diminuent après la désactivation x80 en `content_only`.
- Si blocage persiste:
  - ajouter un watchdog backend pour sortir de `preset_content_only` en timeout contrôlé,
  - corréler la file OUT (id diag) avec les IN reçus pour identifier la famille exacte qui sature.

## 29 avril 2026 — investigation timeouts preset + récupération UI/USB

### Ce qui a été confirmé en logs

- Les timeouts apparaissent surtout pendant la cinématique de lecture preset sur la famille **x80/ED03** (`out_keepalive_x80_or_ed03`), avec des trames **16** et **36 octets** (dont `19 ... 80 10 ed 03` = requête preset).
- Dans certaines rafales, après un premier timeout x80/ED03, **x2** puis **x1** peuvent aussi timeout momentanément (effet "queue OUT saturée"), puis reprendre.
- Le symptôme récurrent côté mode loop reste: **`ignore RequestPresetName while content_only`**.
- Changement de câbles USB testé (plusieurs câbles, retour au câble habituel): **problème toujours présent** -> cause probablement majoritairement logicielle (la couche physique peut amplifier, mais n'explique pas tout).

### Correctifs appliqués pendant cette session

- `src/models.ts`:
  - ajout d'un **cooldown** entre `request_preset_content` (`REQUEST_PRESET_MIN_GAP_MS`) + coalescing (`pendingPresetIndex`) pour limiter les rafales.
  - ajout d'une **récupération temporisée** après échec/timeout (`REQUEST_PRESET_RECOVERY_DELAY_MS`, réglé à **800ms**).
  - ajout d'un **soft-stall** avant timeout long (`REQUEST_PRESET_SOFT_STALL_TRIES`) pour déclencher la récupération plus tôt.
  - ajout d'un **heartbeat UI** dans la barre de statut ("Lecture du preset actif..." / "Sablier: recuperation USB en cours...") pour éviter l'effet "appli figée" quand les logs backend se taisent.
  - ajout d'une **escalade hard** après plusieurs récupérations (`REQUEST_PRESET_HARD_RECOVERY_AFTER`): appel backend de reset lecteur preset.
- `src-tauri/src/lib.rs`:
  - nouvelle commande **`force_recover_preset_reader`**:
    - force sortie de `preset_content_only`,
    - reset `preset_data`, `preset_data_ready`,
    - reset compteurs de requête (`preset_pkt_counter`, `request_preset_session_id`) + `new_session_no()`,
    - switch en `ModeRequest::Standard`.

### État observé après patchs

- Amélioration partielle: les crashs sont moins systématiques, mais des cas persistent.
- Cas dur observé: boucle avec plusieurs logs **`[PresetDebug][recover] force_recover_preset_reader applied`** sans retour stable -> recovery encore insuffisante dans certains enchaînements.

### Décision / suite de reprise

- Garder l'UX actuelle (heartbeat + sablier), utile pour signaler que l'app n'est pas bloquée.
- Prochaine itération prioritaire:
  1. **(partiellement fait le 29 avril soir)** empêcher le **spam de recover** : cooldown + « en vol » + ignores sur requêtes trop rapprochées dans **`force_recover_preset_reader`** ; voir section **« 29 avril 2026 (suite soirée) »**. Reste à valider en stress test et affiner si des boucles persistent encore.
  2. exposer un petit **état diag backend** (mode courant, `preset_content_only`, timestamp dernier recover) pour piloter la relance front sur état réel plutôt que timing seul,
  3. si nécessaire, séquence de recovery backend plus stricte (stop/restart keepalive ciblé + transition mode contrôlée).

### Capture Wireshark (rappel filtre utile)

- Filtre validé pour réduire le bruit:
  - `(usb.src == "1.1.1" || usb.dst == "1.1.1") && (usb.capdata contains 80:10:ed:03 || usb.capdata contains 02:10:f0:03 || usb.capdata contains 01:10:ef:03)`
- Recommandation capture HX Edit:
  - sessions courtes (2–5 min), pas besoin de 50 changements preset,
  - objectif: comparer cinématique Line 6 vs HXLinux dans les mêmes moments de stress.

## 29 avril 2026 (fin de soirée) — compteur de génération preset read

### Problème traité

Perte non-déterministe des trames USB après N lectures de presets (variable : 5 à 50).  
Cause racine : les threads watchdog (2000 ms) et timer (20 ms) armés dans `RequestPreset` peuvent franchir leur `recv_timeout` **après** l'appel à `cancel_watchdog()` / `cancel_timer()`. Le message `Standard` orphelin arrivait dans le mode loop et appelait `shutdown()` sur la lecture **suivante** en cours, l'avortant en plein milieu.

### Correctifs appliqués

- **`src-tauri/src/helix/mod.rs`**
  - Ajout de `pub preset_read_generation: u64` dans `HelixState` (init `0`).
  - Ajout de `ModeRequest::StandardPresetRead(u64)` : variante émise par le timer/watchdog interne de `RequestPreset` à la place de `Standard` pour les lectures `content_only`.

- **`src-tauri/src/helix/modes/request_preset.rs`**
  - `arm_watchdog(…, generation: u64)` et `arm_timer(…, generation: u64)` capturent la génération au moment de l'armement.
  - Sur timeout `content_only` : envoient `StandardPresetRead(generation)` au lieu de `Standard`.
  - Tous les call sites passent `state.preset_read_generation`.

- **`src-tauri/src/lib.rs`** (mode loop)
  - Handler `RequestPreset` : incrémente `s.preset_read_generation` avant `m.start()` → invalide tous les messages en vol des lectures précédentes.
  - Nouveau handler `StandardPresetRead(gen)` : si `gen != s.preset_read_generation` → log orphelin + `continue` (ignoré) ; sinon traitement identique à `Standard`.

### Résultat observé

- **Vitesse de traitement nettement améliorée** (effet secondaire positif inattendu).
- La perte de trame persiste encore dans certains cas (investigation à poursuivre).

---

## 29 avril 2026 (suite soirée) — anti-rafales USB, recover, logs `RequestPresetName`

### Travail effectué (code)

- **`src-tauri/src/lib.rs`**
  - **`force_recover_preset_reader`** : garde-fous anti-spam (recover déjà en vol, **cooldown ~1,5 s**, requête preset trop récente ignorée, pas de recover si session `content_only` déjà terminée avec données prêtes).
  - **`request_preset_content`** : **ne pas relancer** tant que `preset_content_only` est actif ; **throttle** entre deux invocations réelles (`PRESET_REQUEST_MIN_GAP_MS` ≈ **260 ms**) ; `last_preset_request_at` mis à jour seulement quand une lecture est réellement lancée.
  - **Boucle modes** : **déduplication** des `ModeRequest::RequestPresetName` **consécutifs** (comme pour `RequestPreset`), log `dropped duplicate RequestPresetName xN` si rafale.
- **`src/models.ts`**
  - Suppression de l’invoke **`request_active_preset_name`** déclenché automatiquement après chaque chargement de preset (réduire la pression sur la file OUT pendant enchaînements rapides).
  - **Fallback sélection panneau** si aucun contexte restauré : auto-sélection du premier slot non vide, avec **délai ~240 ms** pour laisser passer la synchro « slot actif hardware » avant de retomber sur un slot par défaut (évite en partie le flash immédiat sur slot 0).

### Points encore ouverts (constat utilisateur / logs)

1. **Flash de slot** : transition visible **slot 1 (ou premier slot « utile ») → slot actif réel** sur certains changements de preset.
2. **Presets où le slot actif ne se positionne pas** : après chargement, aucune sélection cohérente avec le hardware (cas corrélé avec rafales / timeouts dans les logs).
3. **Perte des échanges avec le hardware** après **x** changements de preset successifs : logs **`UsbWriter` erreur écriture : Operation timed out** puis comportement instable (`content_only`, ignores `RequestPresetName`, etc.).

### Suite logique proposée

- Throttle / file unique côté Rust pour **`RequestPresetName`** émis depuis **`Standard`** (pas seulement dédup consécutif sur la file de modes).
- Corréler preset « problématique » avec capture Wireshark + `set_usb_io_diag` sur la fenêtre du timeout.
- Poursuivre l’item « état diag backend » déjà listé au **29 avril (matinée)** pour piloter le front sur l’état réel plutôt que sur des timings.

## 30 avril – 2 mai 2026 — stabilisation protocole ED03, slot actif hardware (blocs spéciaux)

### Bug 1 — Réponse Phase 1 rejetée à 64 octets (✅ résolu)

**Symptôme** : environ 1 lecture de preset sur 50 échouait, le watchdog expirait, boucle en `StandardPresetRead` avec `bytes=0`.  
**Cause racine** : `request_preset.rs` testait `data.len() == 68` pour détecter la réponse Phase 1. Le device envoie parfois **64 octets** (HX Edit aussi, comme confirmé sur les captures `Preset1 to 8 from HXEdit.json` vs `HXLinux.json`). La condition rejetait donc ces réponses valides.  
**Correctif** (`src-tauri/src/helix/modes/request_preset.rs`) :
```rust
// AVANT
if data.len() == 68 && sub == 0x04 {
// APRÈS
if sub == 0x04 && data.len() >= 36 {
```
**Validation** : plus de 50 changements de preset enchaînés sans watchdog.

---

### Bug 2 — ACK starvation LED-change après ~50 changements (✅ résolu)

**Symptôme** : après ~50 changements de preset, le device cessait complètement de répondre à la Phase 1.  
**Cause racine** : le device envoie des notifications **LED color change** (ED03, sub=`0x04`, 16 octets) en continu. Le mode `Standard` les ACKait, mais `RequestPreset::data_in()` les ignorait silencieusement. Après ~50 notifications sans ACK, le device saturait et bloquait Phase 1.  
**Correctif** — ajout dans `RequestPreset::data_in()`, avant le bloc `waiting_phase1_response`, après validation du header ED03 :
```rust
if sub == 0x04 && data.len() == 16 {
    state.increase_session_quadruple_x11();
    let sq = state.session_quadruple;
    let cnt = state.next_x80_cnt();
    state.send(OutPacket::with_delay(vec![
        0x08, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt, 0x00, 0x08,
        sq[0], sq[1], sq[2], sq[3],
    ], 0));
    return true;
}
```
**Règle générale** : tout mode qui traite des paquets ED03 doit ACKer les notifications LED-change 16 octets, sinon starvation garantie sur usage intensif.

---

### Bug 3 — Slot actif non mis à jour après MIDI PC (✅ résolu)

**Symptôme** : après un changement de preset via MIDI Program Change, le panneau restait sélectionné sur le slot du preset précédent pendant et après le chargement.  
**Cause racine** : le MIDI PC ne génère pas de paquet x2 `82 62 XX 1a` depuis le device → `hw_active_slot_index` jamais réinitialisé.  
**Correctif** (`src-tauri/src/lib.rs`, fonction `activate_preset`) :
```rust
s.hw_active_slot_index = None;
s.hw_active_slot_bus = None;
s.hw_active_slot_sequence = s.hw_active_slot_sequence.wrapping_add(1);
```

---

### Découverte — valeurs `slot_bus` pour les blocs structurels (✅ confirmé par captures)

Captures réalisées : `Input HXEdit.json`, `Split HXEDit.json`, `Merge HXEdit.json`, `OutPut HXEdit.json`.  
Marqueur recherché : `82 62 XX 1a` dans les paquets x2 longs.

| Bloc | slot_bus |
|------|----------|
| Input | `0x00` |
| Output | `0x09` |
| Split | `0x0a` |
| Merge | `0x13` |

Les slots effet restent : Path 1 `0x01–0x08` → index 0–7 ; Path 2 `0x0b–0x12` → index 8–15.

---

### Implémentation — détection et sélection UI des blocs spéciaux (✅ résolu)

**Problème** : `slot_bus_to_kempline_index()` retournait `None` pour les 4 blocs structurels → `hw_active_slot_index` inchangé, pas de mise à jour UI.

**Backend (`src-tauri/src/helix/mod.rs`)** :
- `is_special_slot_bus(slot_bus: u8) -> bool` : reconnaît `0x00 | 0x09 | 0x0a | 0x13`.
- Nouveau champ `hw_active_slot_bus: Option<u8>` dans `HelixState`.
- `ingest_hw_slot_notify_in` : compare par `slot_bus` (plus fiable que `slot_index`) ; pour les blocs spéciaux : `hw_active_slot_index = None`, `hw_active_slot_bus = Some(bus)`, sequence++.

**Backend (`src-tauri/src/lib.rs`)** :
- `HardwareActiveSlotState` : nouveau champ `slot_bus: Option<u8>` → `slotBus` en JSON.
- `switch_active_hardware_slot` : mise à jour optimiste de `hw_active_slot_bus`.

**Frontend (`src/models.ts`)** :
- `pendingHardwareSelectedSlotBus: number | null` parallèle à `pendingHardwareSelectedKemplineSlotIndex`.
- `selectParamsPaneByHwSlotBus(bus)` : requête `[data-hw-slot-bus="${bus}"]`.
- `consumePendingHardwareSlotSelection` : tente kempline index en premier, puis slot bus.
- Guards anti-fallback mis à jour dans `tryAutoSelectFallbackParamsPaneAfterRender` et `armAutoSelectFallbackParamsPaneAfterRender`.
- `clearSelectedParamsContext` : remet aussi `pendingHardwareSelectedSlotBus = null`.
- Attributs `data-hw-slot-bus` ajoutés au rendu : Input (col=1) → `"0"`, Output (col=19) → `"9"`, Split → `"10"`, Merge → `"19"` (sur tous les séparateurs de frontière, col=2 et col=4..18).

---

### Prochaine étape convenue

- Captures Wireshark pour les opérations d'**édition de preset** (insert model dans slot vide, change model, save preset) avant toute implémentation.
- UI : sélecteur modèle en deux niveaux (`categoryName` → `subCategory`) à développer en parallèle des captures.

---

## Todo à faire dans le Hardware avec capture WireShark

### Priorité code — Phase C (voir § **19 mai 2026** en tête de fichier)

1. ⬜ Implémenter **ingest IN param** (`85:62…1c:PP:77`) + événement front + MAJ panneau sans dump
2. ⬜ Decode IN **modèle / vide** → MAJ cellule grille
3. ⬜ Tests unitaires hex depuis `Slot0_Change_param_#0/#1/#2json`

### Priorité captures (complément)

1. ⬜ Capturer **insertion d'un model** dans un slot vide
2. ⬜ Capturer **sauvegarde d'un preset** (bouton Save dans HX Edit)
3. ⬜ **Slot clear** / bool / discret sur HW (réf. § 19 mai)

### Suite
4. ⬜ Tester une suppression de model dans un slot (si pas couvert par clear)
5. ⬜ Tester un déplacement de model du path 1 au path 2 et inversement.
6. ⬜ Tester une modification de parametre sur split et merge
7. ⬜ Tester un déplacement de split et merge

### Déjà capturé / analysé
- ✅ Slot actif : blocs spéciaux Input/Output/Split/Merge (`slot_bus` identifiés, voir section **30 avril – 2 mai 2026**)
- ✅ Changement de preset hardware → UI (captures `Preset1 to 8`, `Preset1 to 2`)
- ✅ Changement de slot hardware → UI (captures `Slot1 to slot2 hardware`)
- ✅ **Twist param slot 0** (HX Edit) : `Slot0_Change_param_#0.json`, `#1.json`, `#2json` — `param_selector` 0x00/01/02 + float `77:ca`
- ✅ **2× changement modèle slot 0** (HX Edit) : `Slot0_Change_Model_2_Time.json`

## Test

1. Finir l'ensemble des tests des models. Seul les distortions mono ont été testé.
await window.__TAURI__.core.invoke("set_usb_io_diag", { enabled: true })

## Correction a effectuer
1. Le slot vide sur patrh 2 juste avant le merge n'a pas son icone.
2. Deplacement sur un slot vide ne fonctionne pas. coté UI et Hardware.
3. il faut que le panneau module utilise toute la largeur. Peut être jouer sur la largeur de l'icone icons_line.png pour compler l'espace. Ou tracer une ligne... a voir
4. Pour les slider de type selecteur,  ne pas mettre la valeur mini et maxi. Cela n'a pas de sens.