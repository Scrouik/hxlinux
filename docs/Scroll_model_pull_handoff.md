# Scroll → dump modèle (HX Stomp XL) — Document de transmission / impasse

> **Addendum juin 2026 (grab-53) :** les sections **§0, §5, §6 et §12** de ce document sont
> **remplacées** par
> [scroll_model_pull_handoff_addendum.md](./scroll_model_pull_handoff_addendum.md).
> Garder **les deux fichiers** côte à côte : ce handoff conserve l'historique du
> raisonnement et l'impasse d'origine ; l'addendum fige l'état validé sur matériel.
>
> **English:** [scroll_model_pull_handoff_addendum.en.md](./scroll_model_pull_handoff_addendum.en.md)

> **Statut : SUSPENDU.** La fonctionnalité « refléter en direct dans l'éditeur le modèle
> changé à la molette sur le Stomp » est **désactivée par défaut**. Le code reste dans le
> repo (derrière le flag `HX_PULL_COUPLE_LANE`, OFF par défaut) mais n'est pas branché en
> production. Ce document remplace et clôt la lignée `scroll_dump_analysis_1..5.md`
> (toutes contenaient des conclusions partiellement ou totalement fausses — voir §9).
>
> Dernière révision : juin 2026. Auteur : Scrouik + assistance reverse-engineering.
>
> **Commit d'analyse (clôture chantier) :** [`Scroll_model_pull_handoff`](https://github.com/Scrouik/hxlinux/commit/b94508e39d6536702159275cb689b8de351e38a8)
> — branche `fix/none-sur-3894283` (`b94508e`).
>
> **English:** [Scroll_model_pull_handoff.en.md](./Scroll_model_pull_handoff.en.md)
>
> **Addendum (juin 2026, grab-53) :**
> [scroll_model_pull_handoff_addendum.md](./scroll_model_pull_handoff_addendum.md) ·
> [scroll_model_pull_handoff_addendum.en.md](./scroll_model_pull_handoff_addendum.en.md)
>
> **Commit témoin** (pull scroll qui dumpe parfois, ACK 272) : `d6eb2b1` —
> `fix(helix): pull scroll modèle HW, ACK flux 272 et garde standard`. Archives extraites :
> `docs/reference/*_d6eb2b1.rs`.

---

## 0. TL;DR pour qui reprend

- On sait **déclencher** un dump du modèle (le device répond `IN 53` + le model-id), mais
  **de façon intermittente et instable** : selon un compteur de lane qu'on ne sait pas
  dériver correctement, le device soit **dumpe**, soit répond `IN 21` (reject), soit **se
  fige** (reboot obligatoire).
- La cause profonde est un **état de session interne au device** (compteurs de lane ED03)
  qu'on ne peut pas reconstruire de façon fiable **sans les specs Line 6** (qu'on n'aura
  jamais) et que **kempline ne couvre pas** (son analyse s'arrête bien avant).
- Décision pragmatique : **ne pas émettre le pull**. L'éditeur ne reflétera pas les
  changements de modèle faits *au pied* sur le Stomp. Consigne utilisateur : **ne pas
  manipuler les commandes du Stomp pendant que l'éditeur est connecté** (faire les
  changements depuis l'éditeur). Sans pull émis → pas de freeze.

---

## 1. La fonctionnalité visée

Quand l'utilisateur tourne la molette « modèle » sur le HX Stomp XL, le device pousse une
notification (`IN 1d` puis `IN 1f` sur la lane `f0:03:02:10`). L'éditeur voudrait alors
**lire le nouveau modèle** pour mettre à jour la grille à l'écran, en émettant un **pull**
(`OUT 1b` sur la lane `ed`, `80:10:ed:03`). Le device **devrait** répondre par un dump
(`IN 53` ~92 o contenant le model-id, puis éventuellement des bulks `272` o).

C'est une primitive **non couverte par kempline** : kempline lit les presets/noms et
renomme, mais ne fait pas de lecture live du modèle au scroll. Cette partie a été
**entièrement reverse-engineerée** depuis des captures HX Edit (Windows).

---

## 2. Le symptôme

```
Molette → IN 1d (pré-scroll) → IN 1f (trigger)
  → HXLinux : OUT 1b (pull) + OUT 08 (interstitiel f0)
  → Device, selon les cas :
      (a) IN 53 (dump, contient le model-id)  → SUCCÈS
      (b) IN 21 (44 o, notif)                 → REJECT, pas de dump
      (c) … puis, après quelques dumps : plus rien → FREEZE (reboot device obligatoire)
```

Les trois comportements coexistent dans une même session selon la valeur d'un compteur
(voir §5). On n'a **jamais** atteint un état « dumpe à tous les coups, sans freeze ».

---

## 3. Anatomie du pull (ce qu'on émet)

Séquence visée (calquée sur HX Edit `stomp_running_start_hxedit_one_notch.json`) :

```
OUT 1b  80:10:ed:03  … ctr(12-13) … 83:66:cd:03 <lo> 64 … 2d:65:81:62 <slot> 00   (déclenche)
OUT 08  02:10:f0:03  … (interstitiel f0)
IN  53  ed:03:80:10  … 83:66:cd:03 <lo> 67 … 19 <model-id> 1a …                   (DUMP)
IN  21  / IN 1d      (notifications device)
OUT 19  80:10:ed:03  … (réponse #1)
OUT 19  80:10:ed:03  … (réponse #2)
IN  272 …            (bulks complémentaires, écho du dernier double)
```

Deux compteurs voyagent dans chaque OUT `1b`/`19`, **tous deux sur la lane ED03** :

| Champ | Octets | Rôle | Pas observé (HX) |
|---|---|---|---|
| `double cd:03` | 28-29 | `<lo>:64` ; le device le ré-écho en `<lo>:67` | **+1 par OUT** (f1→f2→f3) |
| `ctr` (lane ED03) | 12-13 | position de transaction sur la lane `ed03` | `+0x4b` après `1b`, `+0x31` après `19` |

HX one_notch, octet par octet (référence canonique) :

```
[1b] ctr=0x1c7e  double=f1:64   → le dump (IN 53, echo f1:67) part sur CE 1b
[19] ctr=0x1cc9  double=f2:64   (+0x4b ; +1)
[19] ctr=0x1cfa  double=f3:64   (+0x31 ; +1)
```

Le device **tolère la valeur absolue du double** (un pull qui dumpe peut partir de `f1`
comme de `f8`). Ce qui compte est la **cohérence avec sa session**. Le `ctr`, lui, est le
nœud du problème (§5).

**Note (53 vs 272) :** sur le fil, le `IN 53` arrive **avant** les 272 ; le chainHex y est
déjà lisible. En revanche, **s’arrêter au 53 sans clôturer la transaction** (`19`, drain/ACK
des 272 comme HX) **fige le hardware** — ce n’est pas « pas de 53 sans 272 », mais « pas de
53 seul exploitable sans risque ».

---

## 4. État runtime côté HXLinux (fichier `scroll_model_pull.rs`)

- Pipeline : couche `ScrollModelPull` placée **avant** `FirmwareScroll`. `IN 1f` non-None →
  `Consumed` (émet le pull), `IN 1d`/`IN 21` → `Ignored`.
- Compteurs dans `HelixState` :
  - `editor_ed03_double` (double cd:03 partagé, ≈ `0x64f2` après PHASE B),
  - `editor_ed03_lane` (lane ED03 partagée, octets 12-13 ; ancrage `0x1009`, `+0x17`/commande
    en PHASE B),
  - `hw_model_pull_ed03_double` / `hw_model_pull_ctr` (graines locales du pull).
- **Correctifs acquis et VALIDES** (à garder si quelqu'un reprend) :
  - **+1 par OUT réellement émis** sur le double (motif HX), `hi` figé `0x64`, wrap
    `cd 03→04` géré au passage de `lo` au-dessus de `0xff`. (Le « +3 entre pulls » d'une
    vieille analyse n'est qu'un artefact : 3 OUT × +1.)
  - **Abort propre sur `IN 21`** reçu à l'étape 1 : on ne laisse plus de transaction `1b`
    pendante (évite l'empilement de transactions ED03 non refermées).

---

## 5. LE MUR — la règle du `ctr` (octets 12-13) est inconnue

C'est ici que tout coince. Le `ctr` du pull décide (en partie) si le device dumpe, rejette
ou gèle. On a testé trois familles de valeurs :

| `ctr` du `1b` | Page | Dump ? | Freeze ? | Commentaire |
|---|---|---|---|---|
| `0x6cbd` (hérité de `live_write_ctr`) | `0x6c` | **oui, intermittent** | **oui** | dumpe parfois (f3,f6,f9…) mais rejette d'autres (f5,f8) ; gèle après quelques crans |
| `0x1c7e` (constante, = HX one_notch) | `0x1c` | **jamais** | non | rejet systématique mais session stable |
| `editor_ed03_lane` vivant (≈ `0x1c10`) | `0x1c` | **jamais** | non | idem : continue notre lane réelle, rejet systématique |
| **HX Edit** : `0x1c7e` | `0x1c` | **OUI** | non | HX dumpe en page `0x1c` avec la même valeur que nous rejetons |

**La contradiction insoluble** : HX dumpe en page `0x1c` (`0x1c7e`) ; nous rejetons en page
`0x1c` mais dumpons (par intermittence) en page `0x6c`. Avec un **double identique** (ex.
`f3`), changer le seul `ctr` de `0x6cbd` → `0x1c10` fait passer le device de **dump** à
**reject**. Donc :

1. Les octets 12-13 **font partie** du discriminant (prouvé : même double, ctr différent →
   résultat opposé).
2. Mais la **valeur attendue** ne suit aucune règle qu'on ait pu dériver : ni « page `0x1c`
   comme HX », ni « continuer notre `editor_ed03_lane` », ni une constante.

**Hypothèse la plus probable (non prouvable sans specs)** : le device compare le `ctr` du
pull à un **registre de lane ED03 interne** dont l'évolution dépend de **tout** l'historique
ED03 de la session (PHASE B + lectures preset + …). Notre modèle `editor_ed03_lane` ne
reflète pas fidèlement ce registre — et la valeur de HX Edit (`0x1c7e`) était simplement
celle de SA session à cet instant, pas une constante universelle. Notre session aboutit à
un autre état interne, qu'on n'observe pas.

**Preuve que ce n'est pas que les octets du pull** : la valeur `f8` a **dumpé** dans un run
et a été **rejetée** dans un autre. Le comportement dépend d'un état device non visible dans
le flux bulk.

---

## 6. LE 2ᵉ MUR — le freeze est dans la traîne de dump

Observation décisive : **le freeze n'apparaît QUE quand des dumps ont lieu** (page `0x6c`).
En page `0x1c` (zéro dump), la session survit indéfiniment (25+ crans, aucun gel).

Ce que montre la capture device d'un run qui dumpe puis gèle :

```
OUT 1b → IN 53 (dump) → OUT 19#1 → IN 39 (echo du 19#1) → OUT 19#2 → … plus rien
   → boucle de keep-alive 16 o à l'infini → FREEZE (reboot obligatoire)
```

- Le device **ré-écho notre double avec `hi=0x67`**, en **FIFO** et **avec retard** : l'écho
  d'un OUT du pull N arrive parfois pendant le pull N+1 (lag observé ~2,3 s).
- Quand on envoie `19#2` trop tôt (les deux `19` enchaînés sur la première réponse), le
  device se fige juste après. HX, lui, attend ses notifications post-`53` (`IN 21`, `IN 1d`)
  avant d'envoyer ses `19`. Notre cadence/ordonnancement diffère.

Donc **deux bugs distincts** : (a) la règle du `ctr` (dump vs reject) du §5, et (b) la
gestion de la traîne post-dump (ordonnancement des `19`, ACK des `272`, lag du device) qui
finit par geler. Régler (a) sans (b) ramènerait le freeze.

---

## 7. Tous les angles essayés (chronologie)

Lignée historique (docs v1→v5, **toutes réfutées**) :

1. **Abonnement `0c`/`11` manquant sur la lane f0** → réfuté (présent, identique HX).
2. **Timing de l'ARM f0** (envoyé trop tard) → réfuté (testé `HX_F0_ARM_EARLY`, sans effet).
3. **« État éditeur vivant » = flux `1d` de fond** → réfuté (les `1d` sont produits par le
   geste de molette, pas un abonnement spontané : 0 `1d` de fond sans scroll, 41 avec 1 cran).
4. **Le double `cd:03` (f3 vs f1)** → réfuté (aligné sur HX, toujours `21`).
5. **« double + ctr découplés » (la v5 se croyait RÉSOLUE)** → réfuté : ça dumpait en
   cold-boot sur un commit témoin mais ne tenait pas, et reposait sur `ctr=0x6cbd` qui s'est
   avéré faux.

Itérations de cette session (reverse depuis captures HX + traces Linux) :

6. **+1 par OUT au lieu d'un `+3` aveugle en finalize** → correct (le `+3` avançait même sur
   un pull raté → désync). **Gardé.**
7. **Wrap `cd 03→04`** géré dans tous les modes (avant : `hi` roulait à `0x65`). **Gardé.**
8. **Abort propre sur `IN 21`** (plus de transaction pendante). **Gardé.**
9. **Envoi des deux `19` d'affilée** (miroir HX) → **régression** : gel au 1er cran (on
   envoie `19#1` avant le `IN 21` du device, qui répond `39` puis gèle sur le `19#2`).
   → à revenir à « un `19` par réponse device » si on reprend.
10. **`ctr` : `0x6cbd` → `0x1c7e` → `editor_ed03_lane`** → le mur du §5. `0x6cbd` dumpe+gèle,
    page `0x1c` rejette toujours.

---

## 8. Ce qui est ACQUIS (vrai, vérifié — ne pas re-déterrer)

- Le model-id se lit dans le `IN 53` (~92 o) via le motif `… 19 <id> 1a …` (ex. `cd01fe`).
  Le `IN 21` (44 o) ne contient **jamais** le model-id.
- Le device ré-écho le double host en `hi=0x67` (le host émet en `hi=0x64`).
- Le double tolère la valeur absolue ; il avance de **+1 par OUT** côté HX.
- Le `ctr` avance `+0x4b` (après `1b`) / `+0x31` (après `19`) — deltas confirmés sur HX.
- Le freeze est corrélé aux dumps (donc à la traîne post-dump), pas au `ctr` en soi.
- Les `1d` de fond sont **provoqués par le geste de molette**, pas par un abonnement.
- Aucune requête de contrôle USB spéciale n'est en jeu (HX ne fait que `GET_DESCRIPTOR` +
  `SET_CONFIGURATION`).

---

## 9. RÉFUTÉ définitivement (ne pas réexplorer)

- Abonnement f0 manquant ; timing ARM f0 ; « flux 1d de fond » prérequis ; double seul ;
  `SET_INTERFACE`/contrôle USB ; « ctr = constante page 0x1c (0x1c7e) » ; « ctr = continuation
  naïve de `editor_ed03_lane` ». Tous testés, tous insuffisants.
- Idée que les octets du pull seuls suffisent : faux (`f8` dumpe OU rejette selon la session).

---

## 10. Pourquoi on est vraiment bloqué

- **Pas de specs Line 6** (et on ne les aura jamais). C'est du reverse pur.
- **kempline ne va pas jusque-là** : son code Python est une analyse superficielle qui a
  permis de démarrer (handshake, lecture noms, rename) mais s'est révélée fausse à de
  nombreuses reprises sur les détails de session. Il **n'implémente pas** le pull modèle.
- Le discriminant dump/reject vit dans un **état de session ED03 interne au device**, non
  observable dans le flux bulk, et **dépendant de tout l'historique** de la session. Le
  reconstruire exactement nécessiterait soit les specs, soit une campagne de captures
  beaucoup plus large (matrice {valeur de ctr} × {état précis de la lane} × réponse device).

---

## 11. Pistes concrètes pour un repreneur

Si quelqu'un veut reprendre ce point précis :

1. **Capturer un run qui REJETTE** (page `0x1c`) et disséquer les trames `IN 21` (44 o) :
   y a-t-il, dedans, un compteur/écho révélant la valeur de lane que le device **attendait** ?
   C'est le moyen le plus direct d'obtenir la vraie règle du §5 plutôt que de la deviner.
2. **Tracer l'état device** : le device écho son double en `hi=0x67` et ses réponses ont un
   `ctr` propre en page `0x03` (sens device→host). Cartographier la relation entre le `ctr`
   qu'on émet (host→device, ED03) et celui que le device renvoie pourrait révéler le registre
   interne.
3. **Matrice contrôlée** : à double figé, balayer le `ctr` du `1b` (différentes pages/valeurs)
   et noter dump/reject — pour cerner empiriquement la fenêtre acceptée par CETTE session.
4. **Si dump obtenu** : attaquer le §6 (freeze) — revenir à **un `19` par réponse device**
   (pas les deux d'affilée), attendre le `IN 21`/`IN 1d` post-`53` comme HX, et vérifier
   l'ACK complet des bulks `272`.
5. **Capturer côté Linux/usbmon** (jamais macOS/Windows VM — déjà tranché). Filtre
   Wireshark `usb.idVendor == 0x0e41`.

---

## 12. Décision opérationnelle (état livré)

- Le pull modèle scroll est **débranché** (flag `HX_PULL_COUPLE_LANE` OFF ; aucun `1b` émis
  sur `IN 1f`). **Sans pull émis, pas de freeze.**
- L'éditeur **ne reflète pas** les changements de modèle faits *au pied* sur le Stomp.
- **Consigne utilisateur** : ne pas manipuler les commandes du Stomp pendant que l'éditeur
  est connecté ; faire les changements depuis l'éditeur. (À mettre dans le README.)
- Le code et les correctifs valides (§4) restent en place pour un futur repreneur.

---

## 13. Captures de référence

- `stomp_running_start_hxedit_one_notch.json` — **HX Edit, le pull qui dumpe** (référence
  protocole canonique : `1b` ctr=`0x1c7e`, double `f1`, → `IN 53`).
- `stomp_running_start_linux_multi_notch_crash.json` — HXLinux, runs successifs (dump
  intermittent + freeze en page `0x6c` ; rejet stable en page `0x1c`).
- Lignée d'analyse antérieure : `scroll_dump_analysis_1..5.md` (historique des fausses pistes
  — utile pour ne pas reboucler, mais conclusions à ignorer).

## 14. Fichiers / points de code clés

- `src-tauri/src/helix/scroll_model_pull.rs` — toute la logique du pull (builders `1b`/`19`,
  step machine `ingest_pull_capture`, abort `IN 21`, wrap, compteurs).
- `src-tauri/src/helix/mod.rs` — `HelixState` : `editor_ed03_double`, `editor_ed03_lane`,
  `hw_model_pull_*`.
- Flag d'activation : `HX_PULL_COUPLE_LANE=1`. Debug : `HX_SCROLL_PULL_DEBUG=1`,
  `HX_INIT_TRACE=1`.

---

*Leçon générale : en reverse sans specs, vérifier le run témoin AVANT de théoriser, ne
changer qu'une variable à la fois, et accepter que certains états firmware soient hors de
portée. Ce mur-ci en fait partie.*