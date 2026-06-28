# Analyse scroll → dump modèle (HX Stomp XL) — pourquoi ça ne dumpe pas

> Document de référence **définitif**, sourcé octet par octet sur la capture HX Edit
> `stomp_running_start_hxedit_one_notch.json` (Preset Test, slot 0, 1 cran de molette).
> Objectif : ne plus jamais re-dériver ces faits. Toute reprise du sujet part d'ici.
>
> Dernière mise à jour : juin 2026.

## Contexte

Quand on tourne la molette de modèle : le device pousse `IN 1d` puis `IN 1f`
(lane `f0:03:02:10`), HXLinux envoie un `OUT 1b` de pull sur la lane `ed`, et le
device **devrait** dumper le modèle (`IN 53/54` puis bulk 272o). Chez nous il ne
dumpe pas : il renvoie un `IN 08` (ack nu) puis un `IN 21` (notif d'assignation
hardware), et le pull échoue.

## Méthode

Parsing des `usb.capdata` (bulk EP `0x01` OUT / `0x81` IN) de la capture HX
canonique. Indices = position dans la liste des paquets bulk (249 au total).
Région de scroll : indices 191–212. Région bootstrap : 0–60.

---

## FAIT 1 — Le pull de scroll est (quasi) identique HX ↔ nous. **Ce n'est pas le bug.**

HX, pull qui déclenche le dump (paquet [195]) :
```
1b 00 00 18 80 10 ed 03 00 1d 00 04 7e 1c 00 00 01 00 06 00 0b 00 00 00 83 66 cd 03 f1 64 2d 65 81 62 01 00
```
Nous (run avec go-live) :
```
1b 00 00 18 80 10 ed 03 00 37 00 04 7e 1c 00 00 01 00 06 00 0b 00 00 00 83 66 cd 03 f5 64 2d 65 81 62 01 00
```
Différences : byte 9 = compteur x1 (sans objet) ; double bytes 28–29 = `f1` (HX)
vs `f5` (nous), **toléré par le device** (déjà établi). **La lane est identique
`7e:1c`.** Tout le reste est bit à bit identique.

➜ **Conséquence** : toutes les hypothèses « compteur de lane en retard », « pull
mal formé », « double à corriger » sont **RÉFUTÉES**. Ne plus y revenir.

---

## FAIT 2 — La réponse du device diffère radicalement.

| | HX | HXLinux |
|---|---|---|
| Après `OUT 1b` pull | `IN 53` 92o (dump, lane `ed:03:80:10`) → `IN 21` → `OUT 19` pulls → `IN 08` 272o bulk | `IN 08` 16o (ack nu) → `IN 21` → rien |

HX [197] (le dump) : `53 00 00 18 ed 03 80 10 ... 83 66 cd 03 f1 67 ...` — il **écho
le double f1 du pull** et embarque les données modèle. Chez nous, le `1b` est juste
acquitté (`08 ... ed 03 80 10 ... 16 03`) sans dump.

➜ Le device ne dumpe que dans un certain **état interne**. Le pull est correct ;
c'est l'état qui manque.

---

## FAIT 3 — L'état « éditeur vivant » = un flux `1d` de fond sur la lane f0.

Comptage sur toute la capture HX :
- **41** `IN 1d` de fond (lane `f0:03:02:10`), dont **33 ACKés** (`OUT 08` f0 immédiat).
- Le flux **démarre juste après le trailer phase 4** (HX paquet [60], puis dense
  à partir de [86], = dès la phase noms).

Chez nous : **~0** `1d` de fond. Le device n'émet ce flux **à aucun moment** de la
session (vérifié `USB_PACKET_TRACE=1 delta_only=0`). Les seuls `1d` qu'on voit sont
ceux, ponctuels, déclenchés par le geste de molette lui-même.

➜ Le scroll ne dumpe **que** quand le device est dans cet état de streaming `1d`.
HX y est en permanence ; nous, jamais.

---

## FAIT 4 — « go-live » n'est PAS le déclencheur. (Hypothèse précédente réfutée.)

Chronologie HX :
```
trailer 7a [59] → dialogue post-1a / PHASE B [60–84]
  → flux 1d de fond DÉMARRE [60] puis ACKé dès [86]   ← AVANT go-live
  → requête noms [85]
  → dump noms [94+]
  → go-live ef03 [121] (19) et [123] (1b)              ← APRÈS le flux 1d
  → lecture preset actif → scroll
```
Le flux `1d` (l'état éditeur) est **déjà actif avant** go-live. Donc go-live ne peut
pas l'avoir créé. Notre test le confirme : go-live part, le device répond aux 2
commandes, mais **aucun flux `1d` n'apparaît**. Cohérent — on visait la mauvaise cause.

go-live HX vs nous (pour archive) :
```
HX  #1: 19 ... 01 10 ef 03 ... be 1d 00 00 ... 83 66 cd 03 eb 64 70 65 c0 00 00 00
HX  #2: 1b ... 01 10 ef 03 ... cf 1d 00 00 ... 83 66 cd 03 ec 64 0d 65 81 65 02 00
nous#1: 19 ... 01 10 ef 03 ... 10 1c 00 00 ... 83 66 cd 03 f1 64 70 65 c0 00 00 00
nous#2: 1b ... 01 10 ef 03 ... 21 1c 00 00 ... 83 66 cd 03 f2 64 0d 65 81 65 02 00
```
(lane hi HX=1d vs nous=1c ; doubles eb/ec vs f1/f2 — mais sans objet puisque go-live
n'est pas la cause).

---

## FAIT 5 — CAUSE RACINE PROBABLE : abonnement `0c`/`11` manquant sur la lane f0.

Au bootstrap, HX abonne **les trois** lanes avec la poignée `0c` (subscribe) + `11` :
```
[0]  OUT 0c ... 01 10 ef 03   (ef)   + [2] OUT 11 ef
[8]  OUT 0c ... 80 10 ed 03   (ed)   + [10] OUT 11 ed + [11] IN 11 ed
[12] OUT 0c ... 02 10 f0 03   (f0)   + [15] OUT 11 f0 + [16] IN 11 f0   ← ABONNEMENT F0
[17] OUT 0c ... 01 10 ef 03   (ef, 2e tour) + [20] OUT 11 ef
```
Forme du subscribe (identique sur chaque lane) :
```
0c 00 00 28 <LANE> 00 00 00 02 00 01 00 21 00 10 00 00
```

Côté HXLinux (`reconfigure_x1.rs`) : on ne fait le `0c`/`11` **que sur ef03**.
La lane **f0 ne reçoit qu'un ARM nu** (`08 ... 02 10 f0 03 ... 09 10`, via
`amorcage::send_arm_f0`), **jamais le `0c` d'abonnement**. La lane ed : à vérifier
dans `connect.rs` (non disponible lors de cette analyse).

Or `f0:03:02:10` est **exactement** la lane du flux `1d` de fond manquant (FAIT 3).

➜ **Hypothèse de travail (la plus solide à ce jour, sourcée) :** sans le `0c`+`11`
sur f0, le device ne s'abonne pas aux notifications encodeur → pas de flux `1d` →
pas d'état éditeur vivant → le scroll ne dumpe jamais.

---

## Test proposé (prochaine étape, à garder flag-gardé)

1. **Vérifier `connect.rs`** : fait-il déjà un `0c`/`11` sur `ed` et/ou `f0` ?
   (HX fait ed à [8] et f0 à [12].) Si f0 est absent → c'est le trou.
2. **Ajouter l'abonnement f0** (et ed si absent) `0c`+`11` au bootstrap, **avant**
   les ARM `09:10`, en miroir de HX [8–16]. Forme exacte ci-dessus.
3. Critère de succès en trace : après le trailer, voir apparaître des `IN 1d`
   `f0:03:02:10` **de fond** (hors geste molette), puis au scroll un `IN 53/54`
   au lieu du `IN 08` nu.

Risque : modifie un bootstrap qui marche par ailleurs (presets OK). Donc flag
d'environnement + run témoin/test comme d'habitude. Le reste de la séquence (ARM,
PHASE B, noms, lecture preset) ne change pas.

---

## Ce qui est RÉFUTÉ (ne plus explorer)

- Pull de scroll mal formé / lane / double à corriger → FAIT 1.
- go-live comme déclencheur de l'état éditeur → FAIT 4.
- « stale counter hi » sur le pull → FAIT 1 (HX = `7e:1c` aussi).

## Captures de référence

- `stomp_running_start_hxedit_one_notch.json` — HX canonique (boot+PHASE B+scroll qui dumpe).
- `stomp_running_start_linux_one_notch.json` — notre app (même geste, renvoie 21, pas de dump).