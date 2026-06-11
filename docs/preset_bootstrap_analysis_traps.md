# HX Linux — Amorçage preset : pièges d'analyse et leçons terrain

*One-shot par connexion — pas de seconde chance si l'enveloppe phase 4 est mal reconnue.*

> **English:** [preset_bootstrap_analysis_traps.en.md](./preset_bootstrap_analysis_traps.en.md) · **Fermeture USB propre:** [quitter_sans_figer_hardware.md](./quitter_sans_figer_hardware.md)

> **En une phrase** : la lecture complète des presets est un **amorçage one‑shot**. Le dump intégral (272×N) et la liste des 125 noms ne se lisent **qu'une seule fois par connexion USB**, dans une séquence figée. Ensuite l'éditeur vit en mode `Standard` et ne fait plus que des lectures **ciblées** (corps du preset actif au changement). Conséquence directe : un échec dans cette unique séquence est **fatal pour toute la session** — il n'y a pas de re‑dump automatique. Corollaire (cf. §7) : **toute reconnaissance d'enveloppe figée en dur** (préambule comme trailer) est une bombe à retardement, car la forme de l'enveloppe dépend du preset actif au branchement.

## 1. La séquence, et ce qui n'arrive qu'une fois

Chaîne d'amorçage (une exécution par branchement) :

```
Connect → ReconfigureX1 → amorcage (phase 4 + settle)
        → RequestPresetNames → RequestPresetName → RequestPreset → Standard
```

| Étape | Rôle | Fréquence |
|---|---|---|
| phase 4 (dump 272×N + trailer + PHASE B) | amener l'éditeur « vivant », vider l'état initial | **une fois / connexion** |
| RequestPresetNames | lire les 125 noms | **une fois / connexion** |
| RequestPresetName / RequestPreset | nom + corps du preset actif | une fois à l'amorçage, puis **à chaque changement de preset** |
| Standard | runtime : ACK, événements HW/UI, lectures ciblées | en continu |

Donc « la lecture ne se fait qu'une fois » = **le dump bootstrap + la liste des noms**. Le **corps** d'un preset, lui, se relit à chaque sélection — mais en ciblé, pas un re‑dump complet.

## 2. Le dump phase 4 et son trailer

Le device pousse le dump en rafale de chunks de 272 o, **clos par un chunk partiel** (le « trailer ») :

```
préambule : 92o(54) → 40o(1f) → 68o(3x)         ctr 1a:02 … 3f:02   (handshake pré-dump)
dump      : N × 272o   head=08  sub=04           ctr 50:02
trailer   : 1 × <272o  head=xx  sub=04  len<272  ctr 50:02          ← FIN DE DUMP
```

**La taille du trailer dépend du preset actif** (= taille totale modulo la frontière de chunk). Formes observées en capture : `140/84`, `132/7a`, `116/6a`, mais aussi `104/5f`, `224/d7`, `28/14`… Il faut donc le reconnaître **par sa nature** — un chunk de données (`sub=0x04`) plus court que 272 — jamais par une longueur en dur, sous peine d'intermittence par preset. **Le même principe vaut pour le préambule** (cf. §7) : sa taille et son head varient aussi.

**Gabarit partagé (préambule ou trailer)** — prédicat structurel, pas une liste de head :

```
ed:03:80:10, sub[11]=0x04, 17 ≤ len < 272, hors keepalive 16o (sub=10/00)
```

Implémenté dans `phase4_state.rs` (FSM) et `is_preset_dump_stream_chunk_in` (`preset_dump_stream_ack.rs`) pour les chunks 272 pleins.

## 3. Le « go‑live » : pourquoi le trailer est critique

Le trailer déclenche la **PHASE B** (dialogue éditeur post‑dump : `1b 76:0e`, `1c 76:cc`, `1a`, `19 ed/ef`…). Cette PHASE B **réveille** le device en mode éditeur. Tant qu'elle n'a pas eu lieu :

- le device reste **vivant** (il ping `50:02` / `09:02`),
- mais il **ignore** les requêtes de lecture (`1d` noms, `19` corps).

Donc : **trailer reconnu → PHASE B → go‑live → lectures servies.** Trailer manqué → device muet aux lectures, alors qu'il a l'air vivant.

> **Note (cf. §8.2)** : le trailer **n'est pas acquitté** par l'hôte (son head — `84`, `5f`, … — ne matche pas le gabarit d'ACK `08:01`). Le go‑live ne dépend donc **pas** d'un ACK terminal : il dépend de la **reconnaissance FSM** du trailer. Les trailers classiques ne sont pas ACKés non plus, et le go‑live passe.

## 4. Pourquoi un échec est fatal (et paraissait « intermittent »)

Comme tout est **one‑shot**, il n'y a pas de seconde tentative dans la session : si le préambule **ou** le trailer du preset actif n'est pas reconnu, l'amorçage timeoute (gate phase 4, 3500 ms), le settle est forcé, l'éditeur n'est jamais « vivant », les noms reviennent **vides** et les lectures de corps tournent en watchdog.

D'où le symptôme « marche N fois puis subitement plus rien » : chaque connexion n'a **qu'un seul essai**, et le résultat dépend du **preset actif** au moment du branchement. Un preset dont l'enveloppe (préambule/trailer) tombe sur une forme reconnue passe ; un autre gèle toute la session. Le reboot du Stomp n'y change rien — c'était une lacune de reconnaissance **côté hôte**.

## 5. Les compteurs (lane ED03)

Tout le dialogue éditeur — requêtes **et** ACK des chunks — doit rouler sur **une seule lane** :

| Lane (octets 12‑13) | Usage | Progression |
|---|---|---|
| `editor_ed03_lane` | requêtes `19`/`1b`/`1c` + ACK chunks dump | `9d:10 → 9d:11 → … → 9d:1b` (lo figé, hi +1/chunk) |

L'erreur passée : acquitter les chunks sur une lane **distincte** figée à `f4:1d`. Le device, qui valide strictement les `19`, n'aime pas la discontinuité de lane. **Aligner les deux sous‑compteurs (lo + hi) simultanément est obligatoire** — n'en corriger qu'un échoue silencieusement.

**Revert témoin (debug HW uniquement)** : `HX_DUMP_ACK_LANE=f4` force les ACK dump sur la lane figée `f4:1d` au lieu de `editor_ed03_lane`. Utile pour comparer un run « cassé » historique ; ne pas laisser en prod.

## 6. Règles à ne pas réapprendre

- **Trailer = chunk partiel**, jamais une longueur en dur (sinon intermittence par preset).
- **Préambule = chunk partiel aussi** (cf. §7), jamais une liste de head/len en dur — c'était le dernier endroit qui listait des head (`Waiting68o`).
- **One‑shot = pas de filet** : protéger l'amorçage, car aucun re‑dump ne rattrape un échec en cours de session.
- **Go‑live d'abord** : le device ne sert les lectures qu'après la PHASE B ; un device qui ping n'est pas un device prêt. Et le go‑live ne réclame **pas** d'ACK du trailer (cf. §3, §8.2).
- **Lane unique** : requêtes et ACK du dump sur `editor_ed03_lane`, alignement lo + hi simultané.
- **Une seule définition « chunk dump 272 »** partagée entre la FSM et `preset_dump_stream_ack` (`is_preset_dump_stream_chunk_in`) — pas deux gabarits divergents.

## 7. La variante snapshot : le préambule varie aussi

> **En une phrase** : un preset snapshot embarque la **même enveloppe phase 4** (préambule partiel → N×272 → trailer partiel), mais le **préambule** prend une forme inattendue. Le gate `Waiting68o` ne reconnaissait que `68o head=39|3c` ; il manquait les variantes, la FSM n'atteignait jamais `WaitingDump`, le trailer (pourtant déjà reconnu structurellement) n'était jamais testé → pas de PHASE B → session one‑shot perdue.

### 7.1 Le symptôme

Boot avec un preset snapshot actif (ex. **TX WOODY BLUE**, index 27) → `125 slots, 0 non-empty`, UI vide, `WARN timeout phase4 (3500 ms)`. Le dump arrivait **entièrement** sur le fil (chunks 272 ACKés `9d:10 … 9d:1b`), le trailer aussi — mais la FSM restait figée en `Waiting68o`, en silence (cet état ne loggait même pas les IN ignorés).

### 7.2 Les formes observées

Sur deux runs, **trois formes de préambule** et plusieurs trailers — tous des chunks `ed` partiels (`sub=0x04`, `len<272`), seuls la taille et le head changent :

| Élément | Classique | Variantes observées |
|---|---|---|
| Préambule (post‑`1f`) | `68o head=39\|3c` | `68o head=3b`, `72o head=3e` |
| Trailer (fin de dump) | `140/84`, `132/7a`, `116/6a` | `104/5f`, `224/d7`, `28/14` |

Une liste de head/len en dur les rate tour à tour — exactement le piège déjà identifié pour le trailer (§2), mais cette fois côté **préambule**.

### 7.3 Trace (run snapshot, après fix)

Logs **verbatim** (`HX_INIT_TRACE=1`) — les lignes `[phase4_fsm]` restent en français dans le binaire :

```
[phase4_fsm] Waiting1fA -> Waiting68o (IN len=40 head=1f)
IN 68o : 3b 00 00 18 ed 03 80 10 00 06 00 04 …            ← préambule (sub=04, partiel)
[phase4_fsm] Waiting68o — préambule 68o head=3b (chunk partiel ed) → WaitingDump
N × IN 272o : 08 01 00 18 ed 03 80 10 … 04 …             ← chunks dump (ACKés 9d:10…9d:1b)
IN 104o : 5f 00 00 18 ed 03 80 10 … 04 … (…SNAPSHOT 4…)  ← trailer (partiel)
[phase4_fsm] trailer 104o head=5f (chunk partiel) → PostArm (PHASE B proactive)
…
[RequestPresetNames] finish_transfer: 125 slots, 125 non-empty
```

Avant fix, le même preset poussait un trailer `224o head=d7` — déjà structurellement valide, mais jamais atteint car la FSM restait en `Waiting68o`.

### 7.4 Le fix (structurel, pas une liste de head)

Dans `phase4_state.rs`, arm `Waiting68o` :

| État | Ancienne règle (cassante) | Nouvelle règle (structurelle) |
|---|---|---|
| `Waiting68o → WaitingDump` | `len==68 && head∈{39,3c}` | **(a)** chunk `ed` partiel : `sub==0x04`, `17≤len<272`, hors keepalive ; **OU (b)** 1er **chunk 272** reconnu via `is_preset_dump_stream_chunk_in` (prédicat partagé) |

- **(a)** capte le préambule quelle que soit sa forme — symétrique de la règle trailer en `WaitingDump`.
- **(b)** filet de sécurité : si le préambule a une forme totalement imprévue mais que le 1er vrai chunk `272` (`08:01…`) arrive, on bascule quand même en `WaitingDump` — le device a commencé le dump, il *faut* y être pour capter le trailer. Le chunk 272 est sans équivoque (jamais confondable avec le trailer, qui est partiel).
- Pas d'ambiguïté préambule/trailer : structurellement identiques, mais distingués par la **position** (préambule en `Waiting68o`, trailer en `WaitingDump`). C'est le rôle de la FSM.
- `Waiting68o` ajouté à la liste de log « IN ed ignore » + `sub` tracé, pour ne plus jamais geler en silence ici.

*Aucune modif côté ACK : la couche `preset_dump_stream_ack` est inchangée. Débloquer le bootstrap snapshot ≠ implémenter l'édition snapshot — deux chantiers.*

### 7.5 Point de vigilance restant

`WaitIn1b26` (fin de PHASE B) a **la même fragilité** historique : `len==68 && head∈{3c,39}` sur le chemin Linux. Le run snapshot l'a évité (PHASE B passée par le chemin HX `1b/36o → 26/48o → Done`), donc il n'a pas mordu — mais un autre snapshot empruntant le chemin Linux avec une forme décalée y calerait. **À traiter de la même façon (reconnaissance structurelle) une fois la forme observée en capture**, pas avant.

## 8. Notes ouvertes (non bloquantes)

### 8.1 Index des noms : fallback séquentiel, étiquetage de grille potentiellement désaligné

`extract_preset_index` échoue sur **tous** les records de la liste (`idx_6b=-1 idx_6c=-1`) ; on tombe sur le **fallback séquentiel**. La grille n'est plus vide, mais le fallback suppose *ordre de transfert = ordre des slots* — ce qui est **faux** : l'octet après `81 cd 00` saute (`12, 0f, 10, 0d, 00, 01, 02, 13, 26 …`). Donc des noms peuvent atterrir sur les **mauvais slots** dans la grille.

Ça ne s'est pas vu parce que (a) la grille est remplie et (b) le nom du preset **actif** vient d'un autre chemin (`RequestPresetName` → `6c cd 00 1b` → 27 = `TX WOODY BLUE`, correct), pas de la liste.

Piège pour l'affinage : l'octet après `81 cd 00` **n'est pas non plus directement le slot** — `TX WOODY BLUE` porte `…00 25` (37) dans la liste, alors que la requête active dit `…00 1b` (27). **Deux espaces d'index distincts** (`81 cd …` liste vs `6c cd …` actif) à démêler sur une capture dédiée. Format des records : `81 cd 00 <?> 84 cd 00 6d <len> <nom…>`.

*Bilan : le **chargement** d'un preset est correct (`RequestPreset` utilise le vrai index), c'est l'**étiquetage de la grille** qui est à fiabiliser. Plutôt « bug d'affichage latent » que pur cosmétique.*

### 8.2 Trailer bootstrap non acquitté

Le trailer (`5f:…`, `14:…`, …) n'est **pas** ACKé : son head ne matche pas le gabarit `08:01` de `is_preset_dump_stream_chunk_in`. La FSM suffit au go‑live (cf. §3), et les trailers **classiques** (`84`, `7a`, `6a`) ne sont jamais ACKés non plus — donc risque faible. **À surveiller** uniquement si un preset venait à bouder un ACK terminal manquant.

### 8.3 `WaitingDump - IN ed ignore len=272` : comportement voulu

Ce log est **normal**, pas une anomalie. En `WaitingDump`, un chunk `272` ne matche pas la règle trailer (`272` n'est pas `< 272`) → il tombe dans le `else` → tracé « ignored ». Les 272 pleins sont des chunks de dump, pas des trailers ; seul le partiel terminal déclenche `PostArm`.

## 9. Carte code et debug terrain

| Fichier | Rôle |
|---|---|
| `src-tauri/src/helix/phase4_state.rs` | FSM phase 4 : préambule (`Waiting68o`), trailer (`WaitingDump`), fallback écho ACK (§10), PHASE B |
| `src-tauri/src/helix/preset_dump_stream_ack.rs` | ACK chunks dump sur `editor_ed03_lane` ; `is_preset_dump_stream_ack_echo_in` (§10) ; `HX_DUMP_ACK_LANE` |
| `src-tauri/src/helix/modes/request_preset.rs` | Lecture corps preset : fin sur chunk `< 256` ou écho ACK post-rafale 272 (§10) |
| `src-tauri/src/helix/amorcage.rs` | Enchaînement amorçage → requêtes noms / preset actif |
| `src-tauri/src/helix/mod.rs` | Orchestration session, gate phase 4 (3500 ms), `phase4_dump_full_272_count` |

Variables d'environnement utiles au diagnostic :

| Variable | Effet |
|---|---|
| `HX_INIT_TRACE=1` | Trace FSM phase 4 (`[phase4_fsm]`, IN ignorés + `sub`) |
| `HX_PRESET_DUMP_STREAM_ACK_DEBUG=1` | Détail ACK dump (lane, compteurs) |
| `PRESET_DEBUG_VERBOSE=1` | Liste noms, index, corps preset |
| `USB_PACKET_TRACE=1` / `USB_PACKET_TRACE_BOOT=1` | Paquets USB bruts (boot) |

## 10. Modification du 10 juin 2026 — preset Amp+Cab en slot 0 (WhoWatt)

> **Contexte terrain** : preset actif au branchement avec **Amp+Cab en première case** (slot Kempline 0, ex. WhoWatt 100). Le scroll matériel sur les blocs Amp+Cab avait été corrigé la veille ; le **bootstrap preset** (phase 4 + `RequestPreset*`) restait en échec sur cette configuration.

### 10.1 Symptômes observés

| Symptôme | Log typique |
|---|---|
| Amorçage phase 4 incomplet | `[amorcage] WARN timeout phase4 (3500 ms) — settle forcé` |
| FSM bloquée en fin de dump | `[phase4_fsm] WaitingDump - IN ed ignore len=272` (répété), puis `IN ed ignore len=16 head=08 sub=08` — **sans** `trailer … → PostArm` |
| Liste des noms vide | `finish_transfer: 125 slots, 0 non-empty` |
| Corps preset absent | `RequestPreset::shutdown preset_data_ready=false bytes=0` |
| Scroll OK | `[ScrollModelPull]` / `models:hardware-slot-changed` — le device **ping**, mais ne **sert** pas les lectures |

Conséquence §4 : **session one-shot perdue** jusqu'au prochain branchement USB.

### 10.2 Problèmes identifiés (point par point)

**1. Fin de dump phase 4 sans trailer partiel (cœur du bootstrap)**

Le modèle §2 suppose : `N × 272o` puis **un** chunk `ed` partiel `sub=0x04`, `17 ≤ len < 272`.

Sur la capture WhoWatt / Amp+Cab slot 0 : **12 chunks pleins 272 o** ACKés (`9d:10` … `9d:1b`), puis un seul `IN 16 o` `ed:03:80:10` **`sub=0x08`** (écho de l'ACK host du dernier chunk) — **aucun** paquet terminal `sub=0x04` plus court que 272.

La FSM restait donc en `WaitingDump` jusqu'au timeout gate (3500 ms) : règle trailer structurelle jamais satisfaite → pas de `PostArm` → pas de PHASE B → pas de go-live §3.

**2. Taille de dump multiple de la frontière 256 o (`RequestPreset`)**

`RequestPreset` terminait le transfert quand `chunk_data_len < 256` (payload après l'en-tête 16 o). Si la taille totale du preset est un **multiple exact de 256 o utiles**, **tous** les chunks font 272 o pleins : la condition de fin n'est jamais vraie. Même cause racine que (1), appliquée au chemin `RequestPreset` (relecture ciblée après amorçage).

**3. Grille Kempline après réception du binaire (hors phase 4, même preset)**

Les segments Amp+Cab sont **longs** ; un `82 13` accidentel dans les paramètres, suivi d'un octet `00`…`08`, provoquait un faux découpage `split_preset_by_8213` et désalignait la fenêtre fixe de 20 segments (marqueurs `01`/`03` aux indices 9/19). Symptôme : preset reçu mais grille illisible quand l'Amp+Cab est tôt dans la chaîne — **distinct** du timeout phase 4, mais même famille « gros blob Amp+Cab ».

**4. Encodage WhoWatt `c319` (parsing slot, pas USB)**

Certains Amp+Cab exposent `85188317c319` puis `<amp> 1a <cab> 09` **sans** préfixe `0x19` ni blocs `c219` classiques — le parseur grille ne résolvait pas le `module_hex` (hors bootstrap strict, mais visible une fois le corps chargé).

### 10.3 Solutions apportées

| Zone | Fichier | Changement |
|---|---|---|
| Fin de dump phase 4 (fallback) | `phase4_state.rs` | En `WaitingDump`, **après** la règle trailer partiel inchangée (prioritaire) : compteur `phase4_dump_full_272_count` sur chaque `272 o` via `is_preset_dump_stream_chunk_in` ; si `count > 0` et `is_preset_dump_stream_ack_echo_in` → `PostArm`. Log : `fin dump (écho ACK sub=08 après N×272o) → PostArm`. |
| Détection écho ACK | `preset_dump_stream_ack.rs` | `is_preset_dump_stream_ack_echo_in` : `len==16`, `ed:03:80:10`, `sub[11]==0x08`. |
| Fin `RequestPreset` (fallback) | `modes/request_preset.rs` | Après un chunk plein (256 o utiles), flag `await_dump_end_after_full_chunk` ; complétion sur le même écho `sub=0x08` 16 o (en plus du trailer partiel `< 256` et du FDT `0xa1`). |
| Découpage grille | `lib.rs` | `split_preset_by_8213` : ne coupe sur `82 13` que si l'octet suivant est un en-tête de segment Kempline (`00`…`08`). |
| Inférence Amp+Cab | `lib.rs` | `infer_amp_cab_hex_pair_from_c319_1a_09_tail` pour le format WhoWatt. |
| Scroll (veille / lié) | `scroll_model_pull.rs` | Réutilisation de `extract_module_hex_for_hw_scroll_dump` — **hors** bootstrap ; ne modifie pas la FSM phase 4. |

**Ce qui n'a pas été touché** : lane `editor_ed03_lane` §5, ACK chunks via `preset_dump_stream_ack`, règle préambule structurelle §7.4, gate 3500 ms.

### 10.4 Rapport au modèle des §2–§8

| Principe doc | Statut après modif |
|---|---|
| Trailer = chunk partiel `sub=0x04` | **Conservé en priorité absolue** — le fallback écho ne s'évalue qu'ensuite |
| Pas de head/len en dur pour trailer | **Respecté** pour le chemin principal ; le fallback utilise une **forme fixe** `16 o sub=08` (pas un head trailer) |
| Go-live sans ACK du trailer §8.2 | **Tension** : le fallback **utilise** un écho post-ACK host comme signal de fin — ce n'est **pas** le trailer `sub=0x04` documenté |
| §8.3 — les `272` ignorés pour trailer | **Toujours vrai** pour la règle trailer ; ils alimentent seulement le compteur du fallback |
| One-shot §4 | Inchangé — un faux positif sur l'écho reste **fatal** pour la session |

### 10.5 Risques et vigilance

1. **Faux positif `PostArm` prématuré** — si un `IN 16 sub=08` arrive **au milieu** de la rafale (pas seulement après le dernier 272), la FSM pourrait enclencher la PHASE B trop tôt et désynchroniser l'éditeur. Sur la capture WhoWatt, un seul écho en fin de rafale ; **non généralisé** à d'autres presets.

2. **Variante non validée sur HX Edit** — le doc §2 postule **toujours** un trailer partiel `sub=0x04`. Hypothèse du 10 juin : certains presets lourds (Amp+Cab slot 0) closent par **rafale de 272 pleins + écho ACK** sans trailer données. **À confirmer** par capture Wireshark HX Edit sur le même preset avant de graver cette règle comme définitive.

3. **Si HX Edit envoie bien un trailer partiel** — le bug est ailleurs (timing ACK, lane, entrée trop tôt en `WaitingDump` sur le 68 o métadonnées « Preset Test », etc.) et le fallback **masque** la vraie cause. Dans ce cas : **retirer** le fallback et corriger la reconnaissance du trailer réel.

4. **`RequestPreset` echo** — même risque de complétion trop tôt ; même besoin de validation multi-presets.

5. **Travaux connexes non-bootstrap** — `split_preset_by_8213` et scroll Amp+Cab corrigent la **lecture après** amorçage ; ils ne remplacent pas un bootstrap phase 4 réussi.

### 10.6 Prochaine étape recommandée

1. Capture **HX Edit** : même preset WhoWatt, Amp+Cab slot 0, fenêtre phase 4 — chercher un `IN` terminal `sub=0x04`, `len < 272` **ou** confirmer l'absence et la fin par écho seul.
2. Si confirmé → traiter comme **§7 bis** (variante d'enveloppe) avec garde-fous renforcés (ex. `count ≥ 2`, délai sans nouveau `sub=0x04`).
3. Si infirmé → revert du fallback écho, diagnostic lane / timing sur le trailer manqué.

Log de succès attendu après fix (fallback) :

```
[phase4_fsm] fin dump (écho ACK sub=08 après 12×272o) → PostArm
[PhaseB] PostArm on_enter — ARM_ef …
[RequestPresetNames] finish_transfer: 125 slots, 125 non-empty
```

---

*La lecture des presets n'est pas une opération qu'on relance : c'est un rite d'initiation unique par connexion. Tout ce qui peut casser cette séquence doit être traité comme critique, parce qu'il n'y aura pas de deuxième chance avant le prochain branchement. Et puisque l'enveloppe phase 4 — préambule **comme** trailer — change de forme selon le preset actif, on ne la reconnaît jamais par une taille ou un head en dur : seulement par sa nature (chunk `ed` partiel) et sa position dans la FSM.*