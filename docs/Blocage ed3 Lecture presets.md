# Compte rendu — Reverse-engineering du protocole `ed:03` (HXLinux)

> **Contexte.** HXLinux, éditeur Linux natif pour le Line 6 HX Stomp XL (Rust/Tauri). Aucune doc Line 6 : toute la compréhension du protocole `ed:03` vient de mes propres captures USB (usbmon / Wireshark → JSON). Référence d'implémentation : `kempline/helix_usb`.
>
> **Principe de travail.** *Capture d'abord, jamais de spéculation.* Chaque hypothèse est validée sur trace avant la moindre ligne de code, chaque changement de comportement est gardé derrière un flag à témoin (`=0` restaure l'ancien comportement), et les fausses pistes sont documentées et fermées au même titre que les trouvailles confirmées.
>
> **English:** [Blocage ed3 Lecture presets.en.md](./Blocage%20ed3%20Lecture%20presets.en.md)

---

## Vue d'ensemble

| # | Problème | Cause racine | Correctif | Flag (défaut ON) | État |
|---|---|---|---|---|---|
| 1 | Amorçage qui cale, 0 preset lu | FSM `Waiting68o` figée sur des préambules de forme non standard (presets à snapshots) | Reconnaissance **par nature** (chunk partiel) au lieu de têtes/longueurs en dur | — (structurel) | ✅ Résolu (125/125) |
| 2 | Freeze au scroll multi-cran | Saturation ED03 + lane mal couplée | Couplage `double`+`ctr` vivants + throttle de settling | `HX_PULL_COUPLE_LANE` | ✅ Résolu |
| 3 | Décrochage au-delà de ~256 chunks (**BUG C**) | Compteur de chunks 16 bits ; retenue ignorée (octet 14 figé à 0) | Octet 14 = vrai octet haut, retenue sur débordement de l'octet 13 | `HX_LANE_B14_CARRY` | ✅ Résolu (confirmé terrain) |
| 4 | Désync du double éditeur au wrap (**BUG A**) | HX saute la valeur `lo=0x00` au double wrap, pas nous | Saut du `0x00` sur le `lo` du double éditeur | `HX_EDITOR_DOUBLE_SKIP_00` | ✅ Résolu |
| 5 | Décrochage au tournant de page `05→06` (**§5**) | Abonnement « lane vivante » jamais armé : tour de fermeture PHASE B incomplet ou court-circuité | Commit `1b 0c f1` + attente `23 04` ; FSM sans `Done` prématuré sur `26 ef` | `HX_PHASEB_COMMIT` | 🟡 Handshake **validé log** ; terrain **en attente** |

---

## 1. Amorçage bloqué — `Waiting68o` et les presets à snapshots

**Symptôme.** À la connexion, la FSM phase 4 calait en `Waiting68o`, partait en timeout (3500 ms → settle forcé), l'éditeur n'était jamais « vivant » et **aucun nom de preset n'était lu** (`125 slots, 0 non-empty`). Intermittent : ça marchait sur certains presets, pas sur d'autres.

**Le pourquoi.** Juste avant un dump, le device envoie un « préambule » dont la **taille et la tête varient selon le preset actif** :
- preset classique : 68 o, tête `39`/`3c` ;
- preset à snapshots : 68 o tête `3b`, ou 72 o tête `3e`, etc.

L'ancien code reconnaissait ce préambule par une **liste de têtes/longueurs en dur**. Dès qu'un preset présentait une forme absente de la liste, la FSM ne basculait pas vers `WaitingDump` → blocage. C'est exactement le piège du « trailer figé » qu'on avait déjà eu ailleurs.

**Le comment.** On reconnaît désormais le préambule **par sa nature, pas par ses valeurs** : un chunk `ed` **partiel** (`sub=0x04`, `17 ≤ len < 272`), hors keepalive. La FSM lève l'ambiguïté préambule/trailer non par la forme (ils sont structurellement identiques) mais par la **position** : le préambule arrive en `Waiting68o` (avant tout chunk 272), le trailer en `WaitingDump` (après). Filet de sécurité : si la forme est inattendue mais que le 1ᵉʳ vrai chunk 272 (`08:01`) arrive, on bascule quand même en `WaitingDump`.

> *Leçon générale, réutilisée partout depuis : un prédicat de FSM doit identifier par nature, pas par valeurs figées.* Résultat confirmé : **125 slots, 125 non-empty**.

---

## 2. Freeze au scroll multi-cran

**Symptôme.** En faisant défiler les modèles (scroll), un freeze apparaissait sur les mouvements multi-crans rapides.

**Le pourquoi.** Deux causes couplées :
1. **Couplage de lane incomplet.** Aligner *une seule* des deux variables couplées (le `double` éditeur **et** le `ctr`) suffit à produire un échec — et, pire, une fausse conclusion sur la cause racine. Le `ctr` devait être initialisé à `0x6cbd` et le `double` (`editor_ed03_double`) maintenu vivant.
2. **Saturation ED03.** Les crans rapides empilaient des transactions plus vite que le device ne draine sa fenêtre (~300–400 ms), d'où le gel.

**Le comment.**
- Couplage activé via `HX_PULL_COUPLE_LANE=1` (double vivant + `ctr` init `0x6cbd`).
- Throttle de settling : `post_pull_settling_ms()` par défaut à **500 ms** quand le coalescing est actif (`PULL_THROTTLE_SETTLING_MS=500`) — soit ~1,3–1,6× de marge sur la fenêtre de drain du device.

> *La « Proposition A » (fermer les transactions comme HX Edit) a été parquée ici : le device valide le `19` strictement contre sa lane vivante, ce qui la rendait impossible sans craquer le §5. C'est le même mur qu'on a fini par attaquer de front au point 5.*

---

## 3. BUG C — la retenue de l'octet 14 (compteur de chunks)

**Symptôme.** Au-delà d'un certain nombre de chunks cumulés (~256), une lecture décrochait net en plein dump.

**Le pourquoi.** Pendant un dump, l'hôte acquitte chaque chunk de 272 o par un OUT `08 ed03 sub=08`. Trois octets y portent la position :

| octet | rôle |
|---|---|
| **12** | position de transaction — **indépendante**, figée pendant le dump |
| **13** | compteur de chunk, octet **bas** (lo) |
| **14** | compteur de chunk, octet **haut** (hi) |

Le compteur de chunk n'est **pas** l'octet 13 seul : c'est une valeur **16 bits little-endian sur byte13 (lo) + byte14 (hi)**. HXLinux avait byte14 **codé en dur à `0x00`** dans les trois builders d'ACK ED03 (`RequestPreset`, `preset_dump_stream_ack`, FDT). Tant que byte13 restait sous `0xff`, ça passait. Mais au franchissement `fe → ff → 00`, le compteur doit **retenir dans byte14** ; figé à 0, il retombait à une valeur basse → désync → le device abandonne.

**Preuve capture (HX).** byte12 figé, retenue dans byte14 :

```
95 fe 00      byte12=95   byte13=fe   byte14=00
95 ff 00                  byte13=ff   byte14=00
95 00 01   ← byte13 repasse à 00, byte14 RETIENT → 01
```

**Le comment.** byte14 devient un vrai octet haut, incrémenté à chaque débordement de byte13 (`ff→00`). L'`advance` du compteur renvoie `[byte12, byte13, byte14]`. Gardé derrière `HX_LANE_B14_CARRY` (`=0` = byte14 figé à 0 = bug d'origine).

> *Confirmé terrain : franchissement passé au paquet près (`c5:ff:00 → c5:00:01`), dump complet. Deux fausses pistes fermées par cette même capture : la retenue n'allait **pas** dans byte12, et il n'y avait **pas** de saut du `0x00` sur byte13.*

> *Ne pas confondre avec le §5 : BUG C = compteur de chunks au-delà de `0xff` ; §5 = abonnement lane vivante à la connexion.*

---

## 4. BUG A — le saut du `0x00` sur le double éditeur

**Symptôme.** Désync du `double` éditeur au moment de son wrap, provoquant un échec de lecture quelques crans trop tôt.

**Le pourquoi.** Le `double` éditeur est une valeur 16 bits dont le `hi` est épinglé à `0x64`. Au wrap du `lo`, **HX saute la valeur `0x00`** : il fait `0x64ff → 0x6401`, jamais `0x6400`. HXLinux, lui, émettait `0x6400` → un cran de décalage avec ce qu'attend le device.

**Le comment.** Au wrap du `lo`, si `lo == 0x00` alors `lo = 0x01`. Gardé derrière `HX_EDITOR_DOUBLE_SKIP_00` (et pin hi `0x64` via `HX_EDITOR_DOUBLE_PIN_HI`).

> *Champ confirmé : sans le saut, échec à la lecture 19 ; avec, lectures survivantes jusqu'à 23+. **Ce n'était pas** la cause du décrochage de fond — alignement de fidélité HX, distinct du §5.*

---

## 5. §5 — la « lane vivante » et le décrochage au tournant de page

**Symptôme.** Une fois BUG C et BUG A réglés, les lectures franchissent le wrap du compteur de chunks mais **décrochent toujours** (observé historiquement) quand le compteur de page du device (lane IN, byte13) passe **`05 → 06`** — indépendamment du nombre de lectures, de la valeur du double ou du contenu du preset.

**Le pourquoi.** Fait mesuré sur captures HX :
- HX reçoit **exactement un heartbeat `19 04` par lecture** (25 lectures = 25 heartbeats), porté par la lane vivante du device (`hi = 0x67`) ;
- HXLinux n'en recevait **aucun** tant que le commit PHASE B n'était pas correctement achevé.

Les paquets OUT en régime de lecture (Phase-1/2, ACK, keepalive) sont **octet pour octet identiques** entre HX et HXLinux. Le §5 n'est donc **pas** un paquet manquant par lecture : c'est un **mode/abonnement à armer une fois à la connexion**. Sans cet abonnement, la lane `0x67` s'endort, plus de heartbeats, et le device lâche au tournant de page.

**Ce que montrent les captures (pas un cache local).** HX Edit ne consulte pas une table des 125 corps de preset en RAM : à chaque changement de preset UI, la capture `02_change_preset_*_HXEdit.json` montre la même séquence two-phase `RequestPreset` (`19 ed:03` phase 1 puis phase 2 + chunks). Seuls le **dump bootstrap phase 4** et la **liste des 125 noms** sont one-shot par connexion (cf. [`preset_bootstrap_analysis_traps.md`](./preset_bootstrap_analysis_traps.md)).

### 5.1 Diagnostic initial — le commit manquant

La PHASE B est atteinte, mais l'ancienne FSM terminait trop tôt :

| frame | sens | paquet | double | note |
|---|---|---|---|---|
| f389 | OUT | `1b` sub=**04** | 0x64ec | `ec`, `76:0e` — HX met `sub=0c` (divergence 1) |
| f413 | OUT | `19` sub=0c | 0x64**f0** | finalisation ed |
| f415 | OUT | `19` sub=04 | 0x64e9 | finalisation ef |
| **f417** | **IN** | **`1b` sub=04 ep=ed** | **0x67f0** | **le `1b 04 f0` du device** |
| f419 | IN | `26` sub=04 ep=ef | 0x67e9 | → ancienne FSM : `Done` ici ❌ |

Chez HX, sur f417, l'hôte **répond** par `1b 0c f1` (queue `81 76 0f 00`) et attend **`IN 23 04 ed`**. **Ce tour de fermeture = le commit qui arme l'abonnement persistant.**

**Première vague de correctifs** (`HX_PHASEB_COMMIT`, défaut ON) :
1. `ec` proactif en `sub=0c` (comme HX et les frères `ed`/`ee`) ;
2. état `PbCommit` : émission `1b 0c f1` sur `IN 1b 04 ed` device ;
3. chemin Linux alternatif : `IN 68o ed` → `PbCommit` (au lieu de `Done` direct).

Fichiers : `phase4_state.rs`, `usb_listener.rs`.

### 5.2 Deuxième vague — le piège du `26 ef` (défaut de conception corrigé)

Après la première vague, le commit **partait** sur le fil mais la FSM se fermait quand même **sans confirmation d'abonnement** — reproduisant le §5 par un autre chemin.

**Symptôme log (terrain).** ~2 ms après l'envoi du commit :

```
WaitIn1b26 -> PbCommit (IN 1b 04 ed device)
OUT 1b 0c f1 (commit)
PbCommit -> Done (IN 26/48o ef)    ← faux positif
```

**Raisonnement.** Le `IN 26 ef` (f419) est l'**écho de la finalisation `19 ef`**, souvent déjà en vol quand le device émet son `1b 04 f0`. Ce n'est **pas** la confirmation du commit. L'accepter comme « filet » dans `WaitIn1b26` **ou** `PbCommit` envoyait le commit puis clôturait PHASE B sur l'écho → abonnement **non confirmé** — exactement le §5 qu'on voulait tuer.

**Correctifs (juin 2026, commit `f09f12c`)** :

| État | Avant | Après |
|---|---|---|
| `WaitIn1b26` + `IN 26 ef` | `Done` immédiat (ou avant le `1b` device) | **Reste** en attente ; log « 1/2, commit en attente du 1b device » |
| `WaitIn1b26` + `IN 1b 04 ed` | partiel | → `PbCommit` → `OUT 1b 0c f1` |
| `PbCommit` + `IN 26 ef` | `Done` (filet) | **Ignoré** — log explicite |
| `PbCommit` + `IN 23 04 ed` | — | **`Done`** (confirmation HX) |
| Timeout secours | armé surtout à `PostArm` | réarmé à `PostArm`, `WaitIn1b26`, `PbCommit` (2 s) |

**Séquence validée sur hardware (log `InitTrace`)** :

```
WaitIn1b26 -> PbCommit (IN 1b 04 ed device, commit HX)
OUT 1b 0c f1 (commit) lane=10:1e double=f1:64
PbCommit — IN 26 ef ignoré (attente 23 04 ed, écho f419 ?)
PbCommit -> Done (IN 23 04 ed, commit confirmé)   ~11 ms après le commit
```

➜ **Handshake PHASE B fidèle à HX, validé au paquet près.** Cela prouve que le commit est bien formé et reconnu par le device. **Cela ne prouve pas encore** que les heartbeats `19 04` (lane `0x67`) apparaissent à chaque lecture ni que le passage `05→06` tient.

### 5.3 Niveaux de confiance §5

| Niveau | Affirmation | État |
|---|---|---|
| Prouvé log | `1b 0c f1` émis ; `26 ef` ignoré ; `23 04 ed` reçu ; `Done` propre | ✅ |
| Hypothèse forte | Abonnement lane vivante armé → heartbeats + tenue `05→06` | 🟡 **à confirmer terrain** |
| Vigilance | `23 04` doit arriver à **chaque** connexion (sinon timeout 2 s) | ⚠️ surveiller |
| Vigilance | Chemin Linux `68o ed → PbCommit` : têtes/len en dur comme ex-`Waiting68o` | ⚠️ reconnaissance structurelle si forme observée |

**Test décisif (toujours en attente).** Connexion + ~25 lectures. Trois critères :
1. `1b 0c f1` + `23 04` à chaque connexion ;
2. heartbeats `IN 19 04` lane `0x67` (un par lecture) ;
3. passage `05→06` sans décrochage.

Tant que (2) et (3) ne sont pas verts, on ne déclare pas le §5 **opérationnellement** réglé — seulement le **handshake** validé.

---

## 6. Faux filets et fausses pistes (fermées)

| Piste | Verdict |
|---|---|
| Retenue chunk sur **octet 12** (ancien `HX_LANE_HI_CARRY`) | ❌ Réfutée — capture `out_only.txt` |
| « Saut du 0x00 » sur byte13 seul | ❌ Réfutée — même capture |
| HX Edit = cache des 125 presets en RAM | ❌ Les captures montrent un `RequestPreset` à chaque switch UI |
| `reset_editor_ed03_lane()` dans `force_recover` | ❌ Retiré — le compteur chunks (13+14) est **global** ; reset host seul aggrave la désync (§3) |
| Lenteur perçue = protocole USB | ❌ Surtout latence host (poll 200 ms, throttles) ; dump USB comparable à HX |

---

## Incident de parcours — décalage de versions (leçon de méthode)

Au moment d'appliquer le correctif §5, je t'ai livré un `phase4_state.rs` basé sur une **copie disque périmée**. Conséquence : ton **correctif `Waiting68o` (point 1) a été écrasé** → l'amorçage recalait, *« ça ne lit plus rien »*, retour à `0 non-empty`.

Le signe annonciateur était une erreur de compilation : ton `usb_listener.rs` appelait `handle_in_passive(&mut s, …)` (signature `&mut HelixState`), alors que ma copie avait `handle_in_passive(&mut Phase4Step, …)`. **Cette divergence de signature prouvait à elle seule que ma base était plus ancienne que la tienne.** Correctif : rebaser tous les ajouts sur **ton** fichier courant exact.

> *Leçon : toujours repartir du fichier réel du dépôt, jamais d'une copie de travail antérieure. Un mismatch de signature entre deux fichiers livrés est un signal de version-skew à traiter avant tout, pas à contourner.*

---

## Principes consolidés

- **Capture d'abord.** Aucune hypothèse sans trace ; les fausses pistes sont documentées et fermées formellement.
- **Couplage multi-variables.** Aligner une seule de deux variables couplées échoue *et* fabrique de fausses causes racines (démontré au scroll).
- **Asymétries du device.** Il valide `19` strictement contre sa lane vivante, mais sert `1b` de façon lâche — asymétrie qui a bloqué la Proposition A.
- **Flag-gating systématique** avec témoin (`=0` restaure l'ancien comportement).
- **Niveaux de confiance explicites.** Handshake validé ≠ comportement opérationnel validé.
- **Prédicats par nature, pas par valeurs figées** (généralisation du correctif `Waiting68o`).
- **Un « filet » FSM mal choisi peut recréer le bug** (cas du `26 ef` en `PbCommit`).

---

## État actuel & prochaines étapes

| Sujet | État |
|---|---|
| BUG C (octet 14) | ✅ Clos, confirmé terrain |
| BUG A (saut 0x00) | ✅ Clos |
| Blocage `Waiting68o` | ✅ Clos (125/125) |
| Freeze scroll | ✅ Clos |
| §5 handshake (`1b 0c f1` → `23 04`) | ✅ Validé log terrain |
| §5 opérationnel (heartbeats, `05→06`, ~25 lectures) | 🟡 **En attente** |
| Vigilance `23 04` à chaque connexion | ⚠️ Surveillance |
| Vigilance snapshot sur chemin `68o → PbCommit` | ⚠️ Si commit absent, checker forme IN |

*Prochaine action : enchaîner ~25 changements de preset ; vérifier heartbeats `19 04` lane `0x67` et tenue au tournant `05→06`. Si oui → §5 clos opérationnellement ; sinon, diagnostic sur une base dont le handshake est sain.*
