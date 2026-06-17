# Compte rendu — Reverse-engineering du protocole `ed:03` (HXLinux)

> **Contexte.** HXLinux, éditeur Linux natif pour le Line 6 HX Stomp XL (Rust/Tauri). Aucune doc Line 6 : toute la compréhension du protocole `ed:03` vient de mes propres captures USB (usbmon / Wireshark → JSON). Référence d'implémentation : `kempline/helix_usb`.
>
> **Principe de travail.** *Capture d'abord, jamais de spéculation.* Chaque hypothèse est validée sur trace avant la moindre ligne de code, chaque changement de comportement est gardé derrière un flag à témoin (`=0` restaure l'ancien comportement), et les fausses pistes sont documentées et fermées au même titre que les trouvailles confirmées.

---

## Vue d'ensemble

| # | Problème | Cause racine | Correctif | Flag (défaut ON) | État |
|---|---|---|---|---|---|
| 1 | Amorçage qui cale, 0 preset lu | FSM `Waiting68o` figée sur des préambules de forme non standard (presets à snapshots) | Reconnaissance **par nature** (chunk partiel) au lieu de têtes/longueurs en dur | — (structurel) | ✅ Résolu (125/125) |
| 2 | Freeze au scroll multi-cran | Saturation ED03 + lane mal couplée | Couplage `double`+`ctr` vivants + throttle de settling | `HX_PULL_COUPLE_LANE` | ✅ Résolu |
| 3 | Décrochage au-delà de ~256 chunks (**BUG C**) | Compteur de chunks 16 bits ; retenue ignorée (octet 14 figé à 0) | Octet 14 = vrai octet haut, retenue sur débordement de l'octet 13 | `HX_LANE_B14_CARRY` | ✅ Résolu (confirmé terrain) |
| 4 | Désync du double éditeur au wrap (**BUG A**) | HX saute la valeur `lo=0x00` au double wrap, pas nous | Saut du `0x00` sur le `lo` du double éditeur | `HX_EDITOR_DOUBLE_SKIP_00` | ✅ Résolu |
| 5 | Décrochage récurrent au tournant de page `05→06` (**§5**) | Abonnement « lane vivante » jamais armé : il manque le tour de fermeture PHASE B | Émission du commit `1b 0c f1` + `ec` en `sub=0c` | `HX_PHASEB_COMMIT` | 🟡 Implémenté, **test en attente** |

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

Le compteur de chunk n'est **pas** l'octet 13 seul : c'est une valeur **16 bits little-endian sur byte13 (lo) + byte14 (hi)**. HXLinux avait byte14 **codé en dur à `0x00`** dans les trois builders d'ACK ED03. Tant que byte13 restait sous `0xff`, ça passait. Mais au franchissement `fe → ff → 00`, le compteur doit **retenir dans byte14** ; figé à 0, il retombait à une valeur basse → désync → le device abandonne.

**Preuve capture (HX).** byte12 figé, retenue dans byte14 :

```
95 fe 00      byte12=95   byte13=fe   byte14=00
95 ff 00                  byte13=ff   byte14=00
95 00 01   ← byte13 repasse à 00, byte14 RETIENT → 01
```

**Le comment.** byte14 devient un vrai octet haut, incrémenté à chaque débordement de byte13 (`ff→00`). L'`advance` du compteur renvoie maintenant `[byte12, byte13, byte14]` (au lieu de deux octets). Gardé derrière `HX_LANE_B14_CARRY` (`=0` = byte14 figé à 0 = bug d'origine).

> *Confirmé terrain : franchissement passé au paquet près (`c5:ff:00 → c5:00:01`), dump complet. Deux fausses pistes fermées par cette même capture : la retenue n'allait **pas** dans byte12, et il n'y avait **pas** de saut du `0x00` sur byte13.*

---

## 4. BUG A — le saut du `0x00` sur le double éditeur

**Symptôme.** Désync du `double` éditeur au moment de son wrap, provoquant un échec de lecture quelques crans trop tôt.

**Le pourquoi.** Le `double` éditeur est une valeur 16 bits dont le `hi` est épinglé à `0x64`. Au wrap du `lo`, **HX saute la valeur `0x00`** : il fait `0x64ff → 0x6401`, jamais `0x6400`. HXLinux, lui, émettait `0x6400` → un cran de décalage avec ce qu'attend le device.

**Le comment.** Au wrap du `lo`, si `lo == 0x00` alors `lo = 0x01`. Gardé derrière `HX_EDITOR_DOUBLE_SKIP_00`.

> *Champ confirmé : sans le saut, échec à la lecture 19 ; avec, lectures survivantes jusqu'à 23. **Ce n'était pas** la cause du décrochage de fond (les lectures après `0x6400` étaient propres) — c'est un alignement de fidélité HX, distinct du §5.*

---

## 5. §5 — la « lane vivante » et le décrochage au tournant de page

**Symptôme.** Une fois BUG C et BUG A réglés, les lectures franchissent le wrap mais **décrochent toujours**, invariablement quand le compteur de page du device (lane IN, byte13) passe **`05 → 06`** — indépendamment du nombre de lectures, de la valeur du double ou du contenu du preset.

**Le pourquoi.** Fait mesuré, brutal :
- HX reçoit **exactement un heartbeat `19 04` par lecture** (25 lectures = 25 heartbeats), porté par la lane vivante du device (`hi = 0x67`) ;
- HXLinux en reçoit **zéro**.

Les paquets OUT en régime de lecture (Phase-1/2, ACK, keepalive) sont **octet pour octet identiques** entre HX et HXLinux. Le §5 n'est donc **pas** un paquet par-lecture : c'est un **mode/abonnement à armer au moment de la connexion**. Sans cet abonnement, la lane `0x67` s'endort, plus de heartbeats, et le device lâche au tournant de page.

Ta capture de connexion l'a montré au paquet près. La PHASE B est bien atteinte et propre, mais il manque la fin :

| frame | sens | paquet | double | note |
|---|---|---|---|---|
| f389 | OUT | `1b` sub=**04** | 0x64ec | `ec`, `76:0e` — HX met `sub=0c` (divergence 1) |
| f413 | OUT | `19` sub=0c | 0x64**f0** | finalisation ed |
| f415 | OUT | `19` sub=04 | 0x64e9 | finalisation ef |
| **f417** | **IN** | **`1b` sub=04 ep=ed** | **0x67f0** | **le `1b 04 f0` du device** |
| f419 | IN | `26` sub=04 ep=ef | 0x67e9 | → ton FSM part `Done` ici |

Chez HX, sur le `1b 04 f0` (f417), l'hôte **répond** par un tour de fermeture `1b 0c f1` (queue `81 76 0f 00`) et attend un `23 04`. HXLinux, lui, loguait « 1/2 » et filait `Done` au `26 ef`. **Ce tour de fermeture manquant = le « commit » qui arme l'abonnement persistant.**

**Le comment.** Deux corrections, derrière `HX_PHASEB_COMMIT` :
1. le `ec` proactif passe en `sub=0c` (comme HX et comme ses frères `ed`/`ee`) ;
2. sur le `1b 04 f0`, on entre dans un nouvel état `PbCommit` qui émet `1b 0c f1` (double `f1`, lane +0x17, queue `81 76 0f 00`) et n'achève la PHASE B qu'au `23 04` (avec `26 ef` / 68o Linux en filets, plus le timeout secours 2 s).

Flux PHASE B après finalisation :

```
IN 1b 04 f0 (f417)  →  OUT 1b 0c f1 (76:0f, commit)  →  IN 23 04 f1  →  Done
```

**Test décisif (en attente).** Une capture connexion + ~25 lectures, filtre large. Trois critères : (1) le `1b 0c f1` sort-il, et le device répond-il `23 04` ? (2) les heartbeats `19 04` (lane `0x67`, un par lecture) apparaissent-ils ? (3) franchit-on `05→06` sans décrocher ? Si oui aux trois → §5 confirmé et réglé. Sinon, on élimine proprement le commit et on repivote sur les snapshots.

---

## Incident de parcours — décalage de versions (leçon de méthode)

Au moment d'appliquer le correctif §5, je t'ai livré un `phase4_state.rs` basé sur une **copie disque périmée**. Conséquence : ton **correctif `Waiting68o` (point 1) a été écrasé** → l'amorçage recalait, *« ça ne lit plus rien »*, retour à `0 non-empty`.

Le signe annonciateur était une erreur de compilation : ton `usb_listener.rs` appelait `handle_in_passive(&mut s, …)` (signature `&mut HelixState`), alors que ma copie avait `handle_in_passive(&mut Phase4Step, …)`. **Cette divergence de signature prouvait à elle seule que ma base était plus ancienne que la tienne.** Correctif : rebaser tous les ajouts sur **ton** fichier courant exact.

> *Leçon : toujours repartir du fichier réel du dépôt, jamais d'une copie de travail antérieure. Un mismatch de signature entre deux fichiers livrés est un signal de version-skew à traiter avant tout, pas à contourner.*

---

## Principes consolidés

- **Capture d'abord.** Aucune hypothèse sans trace ; les fausses pistes (retenue→byte12, saut-0x00 sur byte13, byte13=0x00 terminateur, théories scroll v1–v4) sont documentées et fermées formellement.
- **Couplage multi-variables.** Aligner une seule de deux variables couplées échoue *et* fabrique de fausses causes racines (démontré au scroll).
- **Asymétries du device.** Il valide `19` strictement contre sa lane vivante, mais sert `1b` de façon lâche — cette asymétrie a bloqué la Proposition A.
- **Flag-gating systématique** avec témoin (`=0` restaure l'ancien comportement).
- **Niveaux de confiance explicites.** La surconfiance n'a pas sa place ; les incertitudes sont posées directement.
- **Prédicats par nature, pas par valeurs figées** (généralisation du correctif `Waiting68o`).

---

## État actuel & prochaines étapes

| Sujet | État |
|---|---|
| BUG C (octet 14) | ✅ Clos, confirmé terrain |
| BUG A (saut 0x00) | ✅ Clos |
| Blocage `Waiting68o` | ✅ Clos (125/125) |
| Freeze scroll | ✅ Clos |
| §5 commit (`HX_PHASEB_COMMIT`) | 🟡 Implémenté sur ta base saine — **à tester** |
| Vigilance snapshot en `WaitIn1b26` | ⚠️ Fragilité tête/len en dur signalée, à traiter par reconnaissance structurelle **une fois la forme observée** |
| `WaitIn1b26` / `WaitInb26` fragilité | ⚠️ Surveillance |

*Prochaine action concrète : lancer le test décisif du §5 (capture large connexion + ~25 lectures), juger sur les trois critères ci-dessus. Si le §5 tombe, l'hypothèse snapshot du décrochage tombe avec ; sinon, on aura éliminé le commit proprement et on attaquera les snapshots sur une base d'amorçage saine.*