# Analyse scroll → dump modèle (HX Stomp XL) — pourquoi ça ne dumpe pas

> Document de référence **définitif**, sourcé octet par octet sur la capture HX Edit
> `stomp_running_start_hxedit_one_notch.json` (Preset Test, slot 0, 1 cran de molette)
> et sur `connect.rs` / `amorcage.rs` du projet.
> Objectif : ne plus jamais re-dériver ces faits. Toute reprise du sujet part d'ici.
>
> Dernière mise à jour : juin 2026 (révisé après lecture de connect.rs).

## Le symptôme

Molette modèle → device pousse `IN 1d` puis `IN 1f` (lane `f0:03:02:10`), HXLinux
envoie `OUT 1b` (pull) sur la lane ed, le device DEVRAIT dumper (`IN 53/54` + bulk
272o). Chez nous il renvoie `IN 08` (ack nu) + `IN 21` (notif assignation hardware),
le pull échoue. Aucun dump.

## Méthode

Parsing `usb.capdata` (bulk EP `0x01` OUT / `0x81` IN). Indices = position dans la
liste des 249 paquets bulk. Scroll : 191–212. Bootstrap : 0–60.

---

## CAUSE RACINE (résumé)

Le device ne dumpe au scroll que s'il est dans un état « éditeur vivant », signalé
par un **flux continu de `IN 1d` de fond** sur `f0:03:02:10`. HX a ce flux (41 `1d`,
dont 33 ACKés), démarré **dès l'ARM f0 `09:10` au bootstrap** (HX [19] → `1d` [23]).
Notre device n'émet **jamais** ce flux. Le pull, lui, est correct. Donc : pas de flux
`1d` → pas d'état éditeur → pas de dump.

Le **pourquoi** du flux absent n'est pas encore localisé avec certitude (voir
« Frontière »), mais tout le reste est éliminé.

---

## FAIT 1 — Le pull de scroll est (quasi) identique HX ↔ nous. PAS le bug.

HX [195] :  `1b ... 80 10 ed 03 00 1d 00 04 7e 1c 00 00 01 00 06 00 0b 00 00 00 83 66 cd 03 f1 64 2d 65 81 62 01 00`
Nous    :  `1b ... 80 10 ed 03 00 37 00 04 7e 1c 00 00 01 00 06 00 0b 00 00 00 83 66 cd 03 f5 64 2d 65 81 62 01 00`

Seules diffs : cnt (sans objet), double f1 vs f5 (toléré). **Lane identique 7e:1c.**
➜ Hypothèses « pull mal formé / compteur / double » RÉFUTÉES.

## FAIT 2 — La réponse du device diffère.

HX : `OUT 1b` → `IN 53` 92o (dump, écho double f1) → `IN 21` → `19`-pulls → bulk 272o.
Nous : `OUT 1b` → `IN 08` 16o (ack nu) → `IN 21` → rien.
➜ C'est un ÉTAT du device, pas le pull.

## FAIT 3 — L'état « éditeur vivant » = flux `1d` de fond sur f0.

HX : 41 `IN 1d` (lane `f0:03:02:10`), dont 33 ACKés (`OUT 08` f0). Démarre dès le
bootstrap (HX [23], juste après l'ARM f0), puis en continu.
Nous : ~0. Le device n'émet ce flux à AUCUN moment (vérifié trace delta_only=0).
Les seuls `1d` vus sont ceux du geste molette lui-même.

## FAIT 4 — go-live n'est PAS le déclencheur. RÉFUTÉ.

Chronologie HX : trailer [59] → flux 1d démarre [60]/[86] → noms [85] →
**go-live [121]/[123]**. Le flux 1d est déjà actif AVANT go-live. Notre test le
confirme : go-live part, le device répond aux 2 commandes, aucun flux 1d.

## FAIT 5 (CORRIGÉ) — L'abonnement ET l'ARM f0 SONT présents. RÉFUTÉ comme cause.

`connect.rs` abonne les TROIS lanes `0c`/`11` (ef, ed, f0), même ordre et octets
identiques à HX [0–21] :
```
0c ... 02 10 f0 03 00 00 00 02 00 01 00 21 00 10 00 00   (= HX [12], abonnement f0)
```
`amorcage::send_arm_f0` envoie l'ARM f0, identique à HX [19] :
```
08 ... 02 10 f0 03 00 cnt 00 08 09 10 00 00
```
Et le device l'ACK (gate « 3× IN 08/16o ef+ed+f0 » mask=111). Donc subscribe + ARM
f0 sont OK et byte-identiques. **Ce n'est PAS un abonnement/ARM manquant.**

## Contrôle USB — RÉFUTÉ comme cause.

HX Edit ne fait que `GET_DESCRIPTOR` (bReq 6) + `SET_CONFIGURATION` (bReq 9).
Aucun `SET_INTERFACE`, aucune requête vendor/class. Le déclencheur n'est pas au
niveau contrôle.

---

## FRONTIÈRE (non résolu) — la seule divergence structurelle restante

Les octets de l'abonnement + ARM f0 sont identiques, le device les acquitte, mais ne
streame pas. Le déclencheur est donc dans le **contexte / l'ordonnancement**, pas
dans les octets. Différence observée :

- **HX** envoie l'ARM f0 `09:10` (HX [19]) **dans la foulée de l'abonnement f0,
  pendant la phase Connect**, AVANT le 2ᵉ subscribe ef ([20]) et avant la phase 4
  ([30]). Le flux `1d` démarre 4 paquets plus tard ([23]).
- **Nous** : `connect.rs` diffère délibérément l'ARM (« ARM 09:10 reporté à amorcage
  post-ReconfigureX1 »). Il part bien avant la phase 4 (trace +96 ms) et est ACKé,
  mais APRÈS tout le dialogue ReconfigureX1 (2ᵉ subscribe ef, etc.).

Hypothèse à TESTER (non prouvée) : le device n'arme le flux `1d` encodeur que si
l'ARM f0 arrive à ce point précis de la séquence Connect (en miroir HX [19]). Le
report serait la déviation qui casse le streaming. C'est cohérent avec le principe
projet « suivre HX/kempline scrupuleusement ; les déviations introduisent des bugs ».

### Expérience proposée (flag-gardée, 1 seul changement)

Envoyer l'ARM f0 `09:10` **depuis `connect.rs`, juste après l'envoi du `11 f0`**
(donc dans le bloc « Réponse init x2 », en miroir exact de HX [19]), au lieu (ou en
plus) du report dans `amorcage`. Garder l'envoi sous flag d'env (ex. `HX_F0_ARM_EARLY=1`)
pour revert instantané.

Critère de succès en trace : voir apparaître des `IN 1d` `f0:03:02:10` **de fond**
(hors geste molette) dès le bootstrap, puis au scroll un `IN 53/54` au lieu du `IN 08`.

Risque : le report de l'ARM était un choix délibéré (stabilité bootstrap / freeze).
D'où le flag + run témoin/test. Ne PAS toucher au reste (phase 4, PHASE B, noms).

---

## TOUT ce qui est RÉFUTÉ (ne plus explorer)

- Pull de scroll mal formé / lane / double (FAIT 1, lane HX = `7e:1c` aussi).
- go-live comme déclencheur (FAIT 4, chronologie).
- Abonnement f0 manquant (FAIT 5, présent dans connect.rs, identique HX).
- ARM f0 manquant (FAIT 5, présent dans amorcage, ACKé, identique HX).
- Requête de contrôle USB / SET_INTERFACE (HX n'en fait aucune).
- Différence d'endpoint / capture Linux (les `1d` du geste molette SONT captés sur
  0x81 ; si le device streamait, on le verrait).

## Captures de référence

- `stomp_running_start_hxedit_one_notch.json` — HX canonique (boot+PHASE B+scroll dumpe).
- `stomp_running_start_linux_one_notch.json` — notre app (même geste, renvoie 21).