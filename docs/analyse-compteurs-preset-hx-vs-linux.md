# Analyse dos à dos HX Edit / HXLinux — compteurs ED03 & ACK preset

**Date** : mai 2026  
**Objectif** : caler Linux sur HX Edit pour la **première lecture preset**, le **changement de preset**, et les **ACK de chunks** — en séparant les compteurs et en reproduisant l’ordre des trames, pas en extrapolant Kempline.

**Script** : `scripts/analyze_ed03_captures.py`  
**Captures utilisées** : `src/Paquets Json/` (voir matrice §2).

---

## 1. Synthèse (confirmée sur captures)

| Lane | Où sur le fil | HX Edit (observé) | HXLinux (observé) | Verdict |
|------|----------------|-------------------|-------------------|---------|
| **Éditeur / assign** | octets **28–29** des OUT **36 o** avec `83:66:cd:03` | `e8:64` → `f4:64` (2ᵉ octet ≈ `64`) | souvent `e7:64`, sauts, puis `f7:64`, `04:65` | Linux **hors plage** / mélange |
| **ACK chunk preset** | OUT **16 o**, `sub=08` @ byte **11**, double @ **13–14** | `9d:11`, `9d:12`, … `9d:1b`, puis `64:1c` | `ed:64`, `ee:64`, … `f7:64` | Linux met le **compteur éditeur** dans l’ACK |
| **Session phase 2** | même ACK, byte **12** | `9d` (stable par dump) | `81` | OK (aléatoire côté host) |
| **Phase 1 / keep-alive** | OUT 16 o, `sub=10` | `17:45`, etc. (≠ `64xx`) | idem | lane distincte |

**Conclusion** : un seul `preset_pkt_counter` dans `HelixState` est **incorrect**. HX Edit maintient au minimum **trois lanes** (voir tableau Phase A §4) :

1. **`editor_ed03_double`** — plage **`0x64xx`**, octets 28–29 des 36 o `cd:03` (phase 4, pull `1b`, etc.).
2. **`preset_dump_ack_ctr`** — plage **`0x00xx`** (ex. `11:00`, `12:00`…), octets 13–14 des ACK `sub=08` après chaque IN 272.
3. **`live_write_ctr`** — octets 12–13 des trames live write / pull (déjà isolé en code ; ne pas fusionner avec les deux ci-dessus).

Le débordement du pull modèle (`IN 1c` au lieu de `53`) vient très probablement du fait que **`RequestPreset` appelle `next_preset_data_packet_double()` sur chaque chunk** et fait monter le **même** registre que le lane éditeur.

---

## 2. Matrice de captures (à compléter)

### 2.1 Convention de nommage

```
{nn}_{scénario}_{origine}_{plateforme}.json
```

| Segment | Valeurs |
|---------|---------|
| `nn` | `01` … `99` (numéro scénario ci-dessous) |
| `origine` | **`UI`** = action depuis le **programme** (HX Edit / HXLinux) · **`HW`** = action depuis le **boîtier** (pédales, encodeurs, presets footswitches, etc.) |
| `plateforme` | `HXEdit` · `Linux` |

Exemples :

- `02_change_preset_A_to_B_UI_HXEdit.json`
- `02_change_preset_A_to_B_HW_HXEdit.json`
- `04_slot_change_same_preset_HW_Linux.json`

**Fiche par capture** (dans le nom de fichier ou un `README-captures.txt` à côté) :

- modèle Stomp, firmware si connu ;
- preset(s) A / B, slot actif début / fin ;
- action exacte (« footswitch preset +1 », « clic preset 12 dans la liste », etc.) ;
- **origine** UI ou HW.

### 2.2 Deux axes obligatoires

Chaque scénario « action utilisateur » existe en **deux variantes** quand c’est possible sur le matériel :

| Origine | Qui déclenche | Intérêt protocole |
|---------|----------------|-------------------|
| **UI** | HX Edit ou HXLinux (souris / écran) | Référence pour ce qu’on doit **émettre** côté app |
| **HW** | Stomp seul (HX Edit connecté en écoute) | Trames **IN** de notif (`82:62`, `1d`/`1f`, MIDI PC, etc.) et ce que HX Edit **répond** (dump complet ou lecture ciblée ?) |

Sans la paire UI/HW, on risque de caler Linux sur un flux « éditeur actif » alors que l’écoute temps réel doit surtout suivre le **HW**.

**Exceptions** (une seule capture suffit) :

| Scénario | Pourquoi |
|----------|----------|
| 01 cold boot + 1ʳᵉ lecture | pas d’action HW distincte ; lecture auto à la connexion |
| 07 idle 30 s | aucune action |
| 08 fermeture app | côté host uniquement |
| 06 nom preset seul | souvent UI uniquement (si pas d’équivalent HW) |

### 2.3 Tableau complet — Windows (HX Edit) puis Linux (HXLinux)

Pour **chaque ligne** avec UI **et** HW : faire d’abord **HX Edit**, analyser, corriger si besoin, puis **même scénario + même origine** sous Linux.

| # | Scénario | UI — HX Edit | HW — HX Edit | UI — Linux | HW — Linux |
|---|----------|--------------|--------------|------------|------------|
| 01 | Cold boot → connexion → 1ʳᵉ lecture preset auto | `01_connect_first_preset_HXEdit.json` | — | `01_connect_first_preset_Linux.json` | — |
| 02 | Changement preset A → B | `02_change_preset_A_to_B_UI_HXEdit.json` | `02_change_preset_A_to_B_HW_HXEdit.json` | `02_…_UI_Linux.json` | `02_…_HW_Linux.json` |
| 03 | Retour preset B → A | `03_change_preset_B_to_A_UI_HXEdit.json` | `03_…_HW_HXEdit.json` | `03_…_UI_Linux.json` | `03_…_HW_Linux.json` |
| 04 | Changement **slot** seul (même preset) | `04_slot_change_UI_HXEdit.json` | `04_slot_change_HW_HXEdit.json` | `04_…_UI_Linux.json` | `04_…_HW_Linux.json` |
| 05 | Changement **modèle** sur slot actif | `05_slot_model_UI_HXEdit.json` | `05_slot_model_HW_HXEdit.json` | `05_…_UI_Linux.json` | `05_…_HW_Linux.json` |
| 06 | Lecture **nom** preset seul | `06_preset_name_UI_HXEdit.json` | (si applicable) | `06_…_UI_Linux.json` | — |
| 07 | Idle 30 s après connect | `07_idle_30s_HXEdit.json` | — | `07_idle_30s_Linux.json` | — |
| 08 | Fermeture programme (câble branché) | `08_close_hxedit_HXEdit.json` | — | `08_close_hxlinux_Linux.json` | — |

**Captures déjà dans le dépôt** (à renommer / compléter selon cette grille) :

| Fichier actuel | Rapprochement |
|----------------|---------------|
| `Start_Model_change.json` | ≈ 01 HX Edit |
| `Start_Model_change_Linux.json` | ≈ 01 Linux |
| `Change_preset.json` | ≈ 02 ? (vérifier UI vs HW + plateforme) |
| `Slot0_Change_Model_2_Time.json` | ≈ 05 UI HX Edit |
| `Slot0_Change_Model_*_Linux.json` | ≈ 05 UI Linux (à compléter) |

### 2.4 Procédure UI (HX Edit ou HXLinux)

1. Stomp branché, éditeur connecté (HX Edit sous Windows pour la référence ; HXLinux pour la vérif).
2. Démarrer la capture **juste avant** l’action (ou inclure ~1 s de contexte déjà connecté).
3. Déclencher **une seule** action depuis l’interface : clic preset dans la liste, clic slot dans la grille, changement de modèle, etc.
4. **Ne pas** enchaîner d’autres clics ; stopper la capture **1–3 s** après le dernier bulk IN/OUT visible.
5. Noter : nombre d’IN 272, séquence ACK `sub=08`, évolution des doubles `28–29` (`cd:03`) — ce sera la référence pour ce que l’app **doit émettre**.

Même discipline que la procédure HW : une action = un fichier JSON.

### 2.5 Procédure HW (HX Edit en écoute)

1. Connecter HX Edit, **ne pas toucher** la souris pendant la fenêtre de capture.
2. Déclencher **une seule** action sur le Stomp (ex. un pas preset, un clic encodeur slot).
3. Stopper la capture 1–3 s après la dernière IN bulk visible.
4. Noter si HX Edit a **relancé un dump preset** ou seulement des trames légères (slot / modèle) — comparer au scénario **UI** du même #.

### 2.6 Statut actuel (résumé)

| # | UI HX | HW HX | UI Linux | HW Linux |
|---|-------|-------|----------|----------|
| 01 | partiel | — | partiel | — |
| 02–06 | incomplet | manquant | incomplet | manquant |
| 07–08 | manquant | — | manquant | — |

**Commande analyse** :

```bash
python3 scripts/analyze_ed03_captures.py
```

Comparer en priorité les séquences :

```bash
# ACK chunks HX
rg '80:10:ed:03:00:[0-9a-f]+:00:08' "src/Paquets Json/Start_Model_change.json" | head -25
# ACK chunks Linux
rg '80:10:ed:03:00:[0-9a-f]+:00:08' "src/Paquets Json/Start_Model_change_Linux.json" | head -25
```

---

## 3. Résultats détaillés — boot + modèle (`Start_Model_change*`)

### 3.1 Lane éditeur `28–29` sur OUT 36 `cd=03`

| | HX Edit | HXLinux |
|---|---------|---------|
| Séquence `cd=03` | `e8:64` `e9:64` `ea:64` `eb:64` … `f4:64` (13 pas) | `e7:64` `ea:64` `eb:64` puis `04:65`, `f4:64`… |
| Premier double | `e8:64` | `e7:64` (**−1**) |
| Après phase 4 | monte linéairement dans `64xx` | mélange `cd=01/02/04` avec doubles `f5:64`… |

HX reste dans une **fenêtre étroite** ; Linux **réutilise** le compteur pour d’autres `cd` (dump, focus, etc.).

### 3.2 ACK chunks preset (`sub=08`)

Exemples bruts :

**HX** (`Start_Model_change.json`) :

```
…:00:08:9d:11:00:00
…:00:08:9d:12:00:00
…
…:00:08:9d:1b:00:00
…:00:08:64:1c:00:00
```

**Linux** (`Start_Model_change_Linux.json`) :

```
…:00:08:81:ed:64:00
…:00:08:81:ee:64:00
…:00:08:81:f7:64:00
```

→ Linux encode **`0x64ed`** dans l’ACK ; HX encode **`0x0011`** (octet bas qui s’incrémente, octet haut = contexte session).

### 3.3 Changement preset (`Change_preset.json` — Linux)

ACK pendant dump : `d6:47`, `d6:48`, … `d6:57` — **pattern HX-like** (session + compteur bas).  
À la fin : OUT 36 `28-29=04:64` (lane éditeur pour autre opcode).

→ Sur une capture Linux « propre », l’ACK peut être correct **si** le compteur dump n’a pas été pollué par la phase 4 ; sur `Start_Model_change_Linux`, la pollution est visible.

### 3.4 Scénario 08 — fermeture application (état Stomp)

Lié au problème d’**état semi-dysfonctionnel** après fermeture de HXLinux (voir `docs/todo-analyse-trames-communes.md` : séquence de fin / trames communes encore non identifiées).

**Captures** : `08_close_hxedit_HXEdit.json`, `08_close_hxlinux_Linux.json` — fenêtre depuis ~2 s **avant** la fermeture jusqu’à ~5 s **après** (câble USB toujours branché).

**À relever sur le fil** :

- Derniers OUT ED03 / keep-alive / `ed→ef→f0` émis par l’éditeur avant exit.
- Trames éventuelles **non** envoyées aujourd’hui par HXLinux mais présentes chez HX Edit à la fermeture.
- Comportement du Stomp **sans** éditeur : répond-il encore aux commandes **manuelles** (preset / slot au pied) ?

Cette section alimente directement la checklist Phase D (fermeture propre).

---

## 4. Plan d’alignement code (après captures §2)

### Phase A — Modèle d’état (Rust)

Dans `helix/mod.rs`, **scinder** les registres — ne plus tout passer par `preset_pkt_counter` (Kempline).

| Registre (proposé) | Rôle sur le fil | Plage observée (HX) | Utilisé par |
|--------------------|-----------------|---------------------|-------------|
| **`editor_ed03_double`** | Octets **28–29** des OUT **36 o** avec `83:66:cd:03` | `0x64e8` … `0x64f4` (lane étroite) | Phase 4, pull `1b` / assign, etc. |
| **`preset_dump_ack_ctr`** | Octets **13–14** des ACK **16 o** `sub=08` après IN 272 | `0x0011`, `0x0012`, … (`xx:00`) | `RequestPreset` uniquement |
| **`live_write_ctr`** | Octets **12–13** des OUT live write / pull `1b` (déjà séparé en code) | propre à la lane live write | `live_write.rs`, `slot_model_hw_pull` |

**`live_write_ctr`** ne doit **pas** être mélangé avec `editor_ed03_double` : ce sont deux champs sur des offsets différents dans la même trame `1b`, mais des **séquences d’incrément** distinctes (les trames live write avec `sub=08` / lane live write utilisent leur propre compteur — à confirmer sur captures §2 #5 UI).

Valeurs **init** et **reset** : voir §5 (capture obligatoire).

Fonctions suggérées :

- `editor_transaction_double()` / `next_editor_transaction_double()`
- `preset_dump_ack_double()` / `next_preset_dump_ack_double()`
- conserver `live_write_ctr` + helpers existants, sans les faire passer par les deux registres ci-dessus

Supprimer l’usage de `next_preset_data_packet_double()` dans `request_preset.rs` sur le compteur éditeur.

### Phase B — Séquence `RequestPreset`

Aligner sur capture HX pour :

1. Phase 1 `19` — double @ 14–15 : **sans** incrément éditeur.
2. Phase 2 `19` — double dump, pas éditeur.
3. Chaque IN 272 → ACK 16 o `sub=08` avec **dump ctr** uniquement.
4. Fin dump — ne pas copier `last_ack_double` dans l’éditeur ; optionnellement **figer** `editor_ed03_double` à la valeur post-phase-4 HX (`0x64ec`).

### Phase C — Phase 4 bootstrap

Conserver la logique HX déjà documentée (`e8`/`ea`/`eb` + réutilisation `1a`) sur **`editor_ed03_double` uniquement**.

### Phase D — Validation matériel

Checklist par scénario :

- [ ] Après connect : premier OUT 36 `cd=03` = `e8:64` (pas `e7:64`).
- [ ] Pendant dump : ACK = `xx:00` ou `session:xx` style HX, **pas** `xx:64`.
- [ ] Après dump : pull `1b` → IN **`53`** (pas `1c`).
- [ ] Changement preset : comparer nombre d’OUT 36 / IN 272 / ACK avec HX.
- [ ] **Scénario 08** : après fermeture **propre** de HXLinux (ou séquence de fin alignée HX Edit), le Stomp répond **normalement** aux commandes **manuelles** (preset / slot au boîtier) **sans** reset matériel (débrancher / rebrancher).

---

## 5. Méthode de travail recommandée

### Règle d’or

> **Toute valeur initiale hardcodée** (`0x64e8`, `0x001e`, `live_write_ctr` au boot, etc.) **doit être justifiée par une capture HX Edit**, pas par Kempline ni par une ancienne hypothèse.  
> Si la capture manque → on capture d’abord, on code ensuite.

C’est la leçon principale de ce chantier : le modèle « un seul `preset_pkt_counter` » venait d’une analyse tronquée ; le fil réel montre **plusieurs lanes**.

### Étapes

1. **Capturer** les paires manquantes (§2) dans les **mêmes conditions** (UI et HW).
2. **Extraire** avec `analyze_ed03_captures.py` + `rg` sur `00:08` et `cd:03`.
3. **Tableau différentiel** : pour chaque OUT/IN type, noter offset du double, règle d’incrément, dépendance session.
4. **Implémenter** Phase A→D ; test `cargo test` + une session `HX_PRESET_DEBUG=1`.
5. **Documenter** dans ce fichier la valeur initiale **observée** + référence du fichier JSON + numéro de paquet si utile.

---

## 6. Références code actuel

| Fichier | Rôle |
|---------|------|
| `helix/mod.rs` | `preset_pkt_counter` unique (à scinder) |
| `modes/request_preset.rs` | `next_` par chunk L.276 |
| `editor_phase4_bootstrap.rs` | phase 4 lane `0x64xx` |
| `slot_model_hw_pull.rs` | `pull_preset_double()` |
| `keep_alive.rs` | double @ 12–13 (session + compteur — à revoir vs lanes §4) |
| `live_write.rs` | `live_write_ctr` @ 12–13 (registre séparé) |

---

*Prochaine action utilisateur utile* : export **`Change_preset_HXEdit.json`** + renommer/clarifier `Change_preset.json` ; relancer le script et compléter le tableau §2.
