# Drag & Drop de blocs FX — protocole `ed:03` HX Stomp XL

Reverse-engineering du déplacement de blocs FX dans la grille (HXLinux).
Couvre le déplacement intra-path (même path) et inter-path (path1 ↔ path2),
la relecture du preset après mutation, et la cascade de bugs résolue.

Toutes les affirmations sont issues de captures usbmon/Wireshark personnelles
(voir §10). Références courantes : `d&d_*.json`, `Preset_Test_D_D.json`,
`SVT3PRO_Willis.json`.

---

## 1. Principe fondamental

**Le D&D est UNE seule trame de mutation. C'est le HARDWARE qui déplace le bloc.**

Le programme n'orchestre pas une séquence copier / coller / supprimer. Il émet
une trame `1d` décrivant « déplace le bloc de la source vers la destination », et
le device effectue le remaniement de la grille lui-même, puis (en inter-path)
pousse spontanément un dump complet du preset réindexé.

Conséquence directe : la position où le device place un split (en inter-path)
n'est **pas** contrôlée par le programme. Toute « règle » sur la position du
split est une déduction contextuelle, pas une commande.

---

## 2. La trame de mutation (`1d`, 40 octets, sub=04)

```
1d 00 00 18 80 10 ed 03 | 00 CC 00 04 | NN DD DD 00 | …queue…
```

| Offset | Valeur | Rôle |
|--------|--------|------|
| 0–7    | `1d 00 00 18 80 10 ed 03` | header de mutation |
| 9      | `CC` | compteur `next_x80_cnt()` |
| 11     | `04` | sub |
| 12     | `NN` | n° de session |
| 13–14  | `DD DD` | `double` (`preset_data_packet_double`) |
| 18–20  | `06 … 0d` | **octets FIXES** — ce ne sont PAS les indices src/dst |
| 27     | `cd`-variant | `04` = même path, `03` = inter-path |
| 28     | `yy` | `live_write_yy` |
| queue  | `82 4b <src_bus> 4c <dst_bus>` | coordonnées réelles src/dst |

Les vraies coordonnées source/destination sont encodées dans la queue via
`kempline_index_to_slot_bus`, **pas** aux offsets 18–20 (erreur d'analyse
initiale corrigée).

Exemple capturé (move slot 3 → path2) :
```
1d 00 00 18 80 10 ed 03 00 43 00 04 65 f3 64 00 01 00 06 00
0d 00 00 00 83 66 cd 03 f9 64 4e 65 82 62 04 1a 00 00 00 00
                        ^^ cd03 = inter-path
```

---

## 3. Intra-path vs inter-path

### Intra-path (même path, ex. slot1 → slot2)
- Trame `1d` avec octet 27 = `cd04`.
- **Pas** de dump auto poussé par le device.
- La relecture UI (`request_preset_content`) est **uniquement host-initiated** :
  pas de collision avec un dump spontané → transfert propre si demandé.

### Inter-path (path1 ↔ path2)
- Trame `1d` avec octet 27 = `cd03`.
- Suivie de l'ACK `08`, puis de **deux commits `19`** (sub=0c) :
  - **SPLIT** : colonne (offset 30) = `0x17`
  - **MERGE** : colonne (offset 30) = `0x16`
  - ancre `64 XX 65 c0`
- Après les deux commits, le device **PUSH spontanément** un dump complet du
  preset réindexé (~11 chunks de 272 o), sans qu'on le demande.
- HXLinux appelle quand même `request_preset_content` pour rafraîchir l'UI :
  le dump auto et la relecture host peuvent se chevaucher → gardes §6 bugs A/C.

Incrément du compteur des commits (mesuré sur 2 captures) :
- `split_lo = session_no + 0x11`
- `merge_lo = session_no + 0x42` (soit `0x11 + 0x31`)

L'écart split→merge sur l'octet bas est `0x31` (pas `0x11`).

---

## 4. Relecture après mutation

Pour rafraîchir la grille UI, on relit le preset actif via
`request_preset_content` (mode `RequestPreset`, two-phase ED03) :

1. **Phase 1** (`sub=04`, octet 30 = `0x17`) : demande nom + index.
   Réponse = enveloppe 36–68 o, head `19`/`1c`/`39`, portant `83 66 cd` à
   l'offset 24.
2. **Phase 2** (`sub=0c`, octet 30 = `0x16`) : déclenche le dump.
3. Le device envoie le dump en chunks de 272 o (256 o utiles), head `08:01`.
4. Fin de transfert : FDT (32 o, `a1`) ou chunk partiel ou écho `sub=08`.

La relecture lit l'**état d'édition courant** (le dump frais), pas la mémoire
preset stockée sur le device. C'est donc bien le résultat du D&D qui est relu.

### Dump auto (inter-path) + relecture host

En inter-path, deux flux peuvent coexister :

| Flux | Origine | Rôle |
|------|---------|------|
| Dump **auto** | Device, juste après les commits `19` | Réindexation interne ; chunks drainés pendant Phase 1 (garde C) |
| Dump **relecture** | Host, Phase 2 de `request_preset_content` | Snapshot autoritaire pour l'UI |

Sans le garde C, le chunk 272 auto est pris pour la réponse Phase 1 → Phase 2
prématurée → les deux dumps se concatènent dans `preset_data` (bug grille).

### `preset_data` : buffer de transport, pas cache périmé

`preset_data` côté Rust est le **buffer du dump USB**. Il ne doit pas être
**réutilisé tel quel** après une mutation sans nouveau `request_preset_content`
(ancien contenu = grille/params faux).

En revanche, **après** une relecture hardware réussie, parser `preset_data` et
appeler `hydrate*` est correct : le buffer contient alors l'état d'édition
courant du device, pas un snapshot du chargement initial.

Côté UI : le dump frais est **autoritaire**. On reconstruit le cache session
avec `hydrateSlotChainSessionFromPresetData` +
`hydrateSlotDualPartsSessionFromPresetData` (await-ées après D&D). Le chemin
optimiste `relocateMatrixSlotSessionData` a été supprimé (bug B).

---

## 5. Le parsing de grille

`try_parse_preset_kempline_grid` (→ 16 slots, `renderGrid16`) attend une fenêtre
de **20 segments** précise (`split_preset_by_8213`), avec :
- `w[0]` commence par `00`, `w[9]` par `01`, `w[10]` par `02`, `w[19]` par `03`
- les 16 indices assignables commencent par `06`/`08`/`07`

Si la fenêtre ne tombe pas juste, repli sur `parse_preset_slots_internal`
(parseur « flux », `renderSlots(flow)`, nombre de slots variable) → grille buggée.

**Le parser gère le dual-path** : `SVT3Pro Willis` (réellement dual-path) parse
correctement en grille 16. Donc un échec de parsing post-D&D ne vient PAS du
dual-path en soi, mais d'un `preset_data` corrompu (voir §6).

Note : `hydrateSlotDualPartsSession dualSlots=N` compte les slots **Amp+Cab /
Cab-dual** (deux sous-modèles dans UN bloc), **PAS** le routing dual-path.
`dualSlots=0` sur un preset dual-path sain est normal.

---

## 6. Cascade de bugs et solutions

Les bugs se sont révélés en cascade : chaque correctif débloquait l'étage
suivant. Ordre chronologique de découverte.

### Bug A — Lane du dump ACK non amorcée (inter-path)
**Symptôme** : relecture impossible après D&D, device gelé.
**Cause** : le dump auto post-commit est acquitté via la lane `editor_ed03_lane`,
**différente** de `live_write_ctr` utilisée par les commits. `matrix_slot_move`
ne touchait jamais `editor_ed03_lane` → ACK avec lo résiduel → gel.
**Fix** : `prime_dump_ack_lane_after_interpath()` amorce
`editor_ed03_lane = (lo = session_no + 0x42, hi = double[0] + 1, b14 = 0)` après
`send_branch_commit_pair`. Flag `HXLINUX_DD_DUMP_ACK_PRIME` (défaut ON).
**Statut** : validé hardware. *Effet de bord* : a débloqué le dump auto, qui a
ensuite causé le bug B.

### Bug B — Branche UI `relocate` au lieu de `hydrate`
**Symptôme** : grille pas rafraîchie (`slots=0/16`).
**Cause** : la branche `hardwareRefreshAfterEdit` appelait
`relocateMatrixSlotSessionData` (déplace le cache params, chemin optimiste) au
lieu de reconstruire depuis le dump frais.
**Fix** : remplacer par `hydrateSlotChainSessionFromPresetData` +
`hydrateSlotDualPartsSessionFromPresetData` (await-ées). `relocateMatrixSlotSessionData`
supprimé de `models.ts`.

### Bug C — Deux dumps concaténés dans `preset_data`
**Symptôme** : `bytes=5376` (≈ 21 chunks) au lieu de `3072` (12 chunks).
Parser échoue → `renderSlots(flow)`, `rawSlots=5`.
**Cause racine** : pendant `waiting_phase1_response`, la condition
`sub==0x04 && data.len()>=36` accepte un **chunk 272** (272 ≥ 36) comme réponse
Phase 1 → Phase 2 prématurée → le dump auto puis la relecture sont concaténés.

Timeline capturée (`Preset_Test_D_D.json`) :
```
[148] OUT Phase1
[150] IN  chunk272 (lane 91)  ← dump AUTO, pris pour réponse Phase1
[152-170] IN chunk272 ×10     ← accumulés
[176] OUT Phase2              ← déclenchée par le chunk
[208] IN  vraie réponse Phase1 (head 19) ← trop tard
[214+] IN dump relecture (lane c8) ← accumulé par-dessus
→ 10 + 11 = 21 chunks = 5376 o
```

**Fix** : garde `HX_DD_DUMP_AUTO_GUARD` (défaut ON). Pendant
`waiting_phase1_response`, un chunk reconnu par
`is_preset_dump_stream_chunk_in` (head `08:01`) est acquitté
(`ack_dump_chunk_without_storing`) mais **ni accumulé ni traité comme Phase 1**.
La vraie réponse Phase 1 arrive ensuite, seule la relecture est capturée.
**Statut** : validé hardware (`drained 11 chunks`, `bytes=2816`,
`renderGrid16 slots=16`).

### Bug D — Trame non-272 décodée en index hors plage (le « flash »)
**Symptôme** : la grille s'affiche correctement puis disparaît ~160 ms après.
Log : `index=147 name='???@???@…'` puis `renderEmpty "Aucun preset actif."`.
**Cause** : le dump auto contient aussi une trame **non-272** (capture `[190]` :
head `cb`, 212 o, params bruts `c0 93 c2 40 …`) que le garde anti-chunk laisse
passer (head ≠ `08:01`). `decode_from_ed03_packet` y lit l'octet `0x93 = 147`
comme index et des `c2/c3/40` comme nom. `state.preset_index = 147` →
l'UI voit `active(147) >= names.length(125)` → `renderEmpty` efface la grille.

Condition UI déclenchante (`models.ts`) :
```ts
if (active < 0 || active >= names.length) {
  renderEmpty("Aucun preset actif.");
  return;
}
```

**Fix** : borne `PRESET_COUNT = 125` dans `request_preset.rs`. Dans la branche
Phase 1, un index décodé `>= PRESET_COUNT` n'est **pas** appliqué — `preset_index`
(déjà positionné à 14 par `request_preset_content`) est conservé, la grille reste
affichée.
**Statut** : validé hardware.

---

## 7. Récapitulatif des flags

| Flag | Défaut | Rôle | `=0` |
|------|--------|------|------|
| `HXLINUX_DD_DUMP_ACK_PRIME` | ON | amorce `editor_ed03_lane` après les commits inter-path | témoin (lane non amorcée) |
| `HX_DD_DUMP_AUTO_GUARD` | ON | draine les chunks 272 du dump auto pendant Phase 1 | témoin (chunk = faux Phase 1) |
| `HX_DUMP_END_CONFIRM_MS` | 150 | fenêtre de confirmation fin-de-dump §10 | clôture immédiate sur écho `sub=08` |

La borne `PRESET_COUNT` (bug D) n'est pas derrière un flag : c'est une simple
validation d'index.

---

## 8. Points de vigilance résiduels

- **Débordement du dump auto en phase transfert** : le garde `HX_DD_DUMP_AUTO_GUARD`
  ne couvre que la fenêtre Phase 1. Si un dump auto plus lent débordait **après**
  Phase 2, des chunks auto pourraient se mélanger à la relecture. Non observé dans
  les captures actuelles — ne pas coder de parade tant que ce n'est pas capturé
  (capture-first). Si ça survenait : filtrer par lane de transaction (auto `91`
  vs relecture `c8`) en phase transfert.

- **Pré-positionnement de l'index** : le garde du bug D suppose que
  `request_preset_content` positionne `preset_index` avant la lecture (cas actuel,
  `active_preset=14`). Si cette séquence d'init change, revérifier l'hypothèse.

- **Nom Phase 1 non capturé en inter-path** : la trame portant le vrai nom
  (`[202]`, « Preset Test ») arrive après l'envoi de Phase 2 et n'est pas décodée.
  Sans conséquence : le nom provient de la liste à l'index actif. Ne pas s'étonner
  de son absence dans les logs.

- **Incrément `0x31` des commits** (split→merge) non « corrigé » explicitement :
  le device tolère l'octet 12 des commits, et l'amorçage du bug A force la bonne
  valeur indépendamment. Documenté pour mémoire.

---

## 9. Fichiers concernés

| Fichier | Rôle |
|---------|------|
| `matrix_slot_move.rs` | émission de la trame `1d` + commits + `prime_dump_ack_lane_after_interpath` |
| `request_preset.rs` | mode `RequestPreset` two-phase, gardes des bugs C et D |
| `preset_dump_stream_ack.rs` | `is_preset_dump_stream_chunk_in`, couche d'ACK des chunks |
| `lib.rs` | parsers (`try_parse_preset_kempline_grid`, fenêtre 20 segments), `EXPECTED_PRESET_COUNT` |
| `models.ts` | rendu grille (`renderGrid16` / `renderSlots(flow)`), `hydrate*`, condition `renderEmpty` |

---

## 10. Captures de référence

| Fichier | Contenu |
|---------|---------|
| `captures/usb-wireshark/d&d_same_path_slot1_to_2.json` | D&D intra-path slot 1 → 2 |
| `captures/usb-wireshark/d&d_same_path_slot2_to_3.json` | D&D intra-path slot 2 → 3 |
| `captures/usb-wireshark/d&d_same_path_slot3_to_8.json` | D&D intra-path slot 3 → 8 |
| `captures/usb-wireshark/d&d_path1_to_path2.json` | D&D inter-path path1 → path2 |
| `captures/usb-wireshark/d&d_path2_to_path1.json` | D&D inter-path path2 → path1 |
| `captures/usb-wireshark/d&d_path1_to_path1.json` | D&D intra-path path1 |
| `captures/usb-wireshark/d&d_path2_to_path2.json` | D&D intra-path path2 |
| `captures/usb-wireshark/d&d_split.json` | Commit split seul |
| `captures/usb-wireshark/d&d_merge.json` | Commit merge seul |
| `captures/usb-wireshark/SVT3PRO_Willis.json` | Preset dual-path (parse grille 16) |
| `captures/usb-wireshark/Preset_Test_D_D.json` | Timeline bug C (dump auto + relecture) |

---

*Méthode : capture-first (aucune hypothèse sans preuve wire), fichiers complets,
changements flag-gated, un pas vérifiable à la fois, hardware = vérité terrain.*