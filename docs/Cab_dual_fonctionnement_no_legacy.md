# Cab dual & changement de cab2 — fonctionnement du protocole USB `ed:03`

**HXLinux — HX Stomp XL**  
*Document de référence technique (cabs **IR** Mic IR / WithPan uniquement — pas les baffles **Legacy** hybrid pré-3.50, qui ont un fil `c319` + suffixe `1a 30:00` distinct). Issu de captures usbmon/Wireshark HX Edit comparées aux trames HXLinux.*

> **English version:** [Cab_dual_operation_no_legacy.md](Cab_dual_operation_no_legacy.md)  
> **Legacy (hybrid) :** [Cab_dual_fonctionnement_legacy.md](Cab_dual_fonctionnement_legacy.md) — replace cab2 : `0x23` (hint 1 o) ou `0x25` (`cd02xx`, 48 o)

---

> **Synthèse en une phrase.** Changer le cab2 d'un slot « Cab dual » n'est ni un handshake bloquant ni une affaire de dump : c'est une **séquence tir-et-oublie de trois trames** (`focus → ed:08 → bulk`) sur **une seule lane cohérente** (la lane `live_write`), dont le bulk doit porter **un cab2 de contexte dual** (`cd031c`/`c3 19`, jamais le single `cd031b`/`c2 19`) et un **en-tête nettoyé** (octet 14 à zéro).

---

## 1. Qu'est-ce qu'un « Cab dual » ?

Sur le HX Stomp XL, un bloc « Cab » peut héberger **deux baffles simultanés** (cab1 + cab2), mixés via un panoramique. C'est ce que le firmware appelle un *Cab dual* (sous-catégorie `Dual`), par opposition à un *Cab single* (un seul baffle).

La subtilité qui a coûté le plus de temps : **un baffle « dual » et le baffle « single » du même nom sont deux modèles différents**, avec deux `bulkHex` distincts et deux identifiants distincts. « Soup Pro Ellipse » existe sous deux formes :

| Forme                             | `id`                                  | `chainHexHint`    | `bulkKind`        | Marqueur  | Tête      |
|---                                |---                                    |---                |---                |---        |---        |
| **Dual** (destiné à un slot dual) | `HD2_CabMicIr_SoupProEllipseWithPan`  | `cd031c`          | `assign48_cd0a`   | `c3 19`   | `0x27`    |
| **Single** (un seul baffle)       | `HD2_CabMicIr_SoupProEllipse`         | `cd031b`          | `assign48_cd09`   | `c2 19`   | `0x25`    |

Même nom affiché, mais `cd031c` ≠ `cd031b`. **Dans un slot dual IR, cab1 ET cab2 utilisent la forme dual sur le fil.** C'était la clé du dernier bug (voir §6.2).

---

## 2. Anatomie d'une trame `80:10:ed:03`

Toutes les commandes de modèle passent par des trames USB HID dont les 8 premiers octets sont :

```
[head] 00 00 18 80 10 ed 03
  │              └──────────── opcode « ed:03 » (manipulation de chaîne/modèle)
  └───────────────────────── tête : nature de l'opération
```

Têtes rencontrées :

| Tête                  | Sens                                                                  |
|---                    |---                                                                    |
| `0x1d`                | **focus** — pointe l'élément qu'on s'apprête à modifier               |
| `0x08`                | **ed:08 court** — « arme » le bulk qui suit                           |
| `0x27`                | **bulk replace dual** (slot occupé, 48 o, `cd:0a`→reframe `cd:04`)    |
| `0x31`                | bulk **create dual** (slot vide, 60 o, `cd:03`)                       |
| `0x25`                | bulk **assign single** (`cd:09`)                                      |
| `0x21`                | IN 21 — accusé/handshake émis **par le device**                       |
| `0x19`/`0x93`/`0x38`  | dumps émis par le device                                              |

Au-delà des 8 octets de tête vient l'**en-tête de lane** (octets 8–15), puis le **corps** (bloc modèle).

---

## 3. Les lanes — le cœur du protocole

### 3.1. Le compteur de lane

Les octets **12–13** de chaque trame `ed:03` portent un **compteur sur 16 bits (little-endian)** : la *lane*. On peut le lire comme `[page][offset]` — l'octet de poids fort (octet 13) est la « page », l'octet de poids faible (octet 12) l'offset dans la page.

```
… 80 10 ed 03  00 [seq] 00 [sub]  [ctr_lo] [ctr_hi]  [14] [15] …
                   │         │      └────── lane (u16 LE) ──────┘
                   │         └── sub-opcode (0x04 en live)
                   └── numéro de séquence
```

> **Les octets 14–15 doivent être à `0x00` en opération live.** Ils ne font pas partie du compteur utile ; un octet 14 non nul décale la lane vers une page parasite et le device ignore la trame. (C'est le piège §6.1.)

### 3.2. La règle de cohérence : `focus → ed:08 → bulk`, écart `+0x11`

C'est **la** loi du protocole, prouvée sur les captures jumelles HX Edit :

> **focus, ed:08 et bulk partagent une seule et même lane cohérente.**
> `ed:08 = focus + 0x11` · `bulk = ed:08`

Concrètement, avec `L` = compteur de la lane modèle au moment du tir :

```
focus   →  ctr = L
ed:08   →  ctr = L + 0x11
bulk    →  ctr = L + 0x11
```

Le device **valide strictement** le ed:08 et le bulk par rapport à la lane du focus. Si les trois ne sont pas sur la même page avec le bon écart, il rejette silencieusement.

### 3.3. HX Edit (1 lane) vs HXLinux (4 lanes fragmentées)

C'est ici que le refactor multithread nous a piégés. **HX Edit garde tout sur une seule page** et la fait avancer d'un bloc :

```
HX Edit :  focus 0x6e7d  →  ed:08 0x6e8e  →  bulk 0x6e8e        (page 0x6e, +0x11)
```

**HXLinux, lui, a fragmenté les compteurs sur quatre pages disjointes**, une par « rôle » de thread :

| Lane | Rôle | Exemple (capture réelle) |
|---|---|---|
| `live_write` | écriture live / **bulk modèle** | `0x6cbd` |
| `sq` / auto-ACK | accusés automatiques | `0x1ef4` |
| `editor` | focus éditeur (clic d'onglet) | `0x3255` |
| `keepalive` | battement de cœur (hardcodé `7e1c`) | `0x1c7e` |

Le danger : si on construit le focus sur la lane `editor` et le bulk sur `live_write`, on viole la règle §3.2 — quatre pages au lieu d'une. **La lane modèle correcte est `live_write`.** Focus, ed:08 et bulk doivent tous y être ancrés.

---

## 4. Anatomie d'un bulk Cab dual

Prenons le `bulkHex` dual brut de « Soup Pro Ellipse » (catalogue, capture du 2026-06-12) et décodons-le :

```
27 00 00 18  80 10 ed 03   ← tête 0x27 + opcode ed:03
00 3a 00 04                ← seq=3a, sub=04
99 8b                      ← ctr (lane) = 0x8b99
05 00                      ← octets 14-15  ⚠ byte14=0x05 = RÉSIDU de capture (devrait être 00)
01 00 06 00 17 00 00 00    ← scaffold d'en-tête
83 66 cd 0a                ← début bloc modèle, kind = cd:0a (dual)
75                         ← tag (octet de lane interne au bloc)
64 28 65 82 62 01 64 83 17 ← scaffold fixe ; 82 62 01 = bus du slot (01)
c3 19                      ← MARQUEUR DUAL
cd 03 1c                   ← cab1 = Soup Pro Ellipse (dual) = cd031c
1a                         ← séparateur cab1/cab2
cd 02 d6                   ← cab2 = Jazz Rivet (défaut) = cd02d6
00                         ← fin
```

Structure générale du bloc modèle d'un dual :

```
83 66 cd <0a|04> <tag> 64 28 65 82 62 <bus> 64 83 17  c3 19  <cab1> 1a <cab2> 00
         │                                            │       │        │
         │                                            │       │        └── 2e baffle
         │                                            │       └── 1er baffle
         │                                            └── marqueur dual (single = c2 19)
         └── 0a = template d'assignation ; 04 = reframe « replace » live
```

**`cd:0a` → `cd:04` (reframe).** Le template catalogue est en `cd:0a` (assignation). Pour un *replace* live (changer un cab dans un slot existant), on « reframe » le `0a` en `04`. C'est purement le sub-kind du bloc ; cab1/cab2 ne bougent pas.

À titre de comparaison, le `bulkHex` **single** du même baffle :

```
25 … 8366 cd 09 … c2 19  cd 03 1b  1a ff 00 00 00
        │          │      │        │
        │          │      │        └── ff = pas de 2e baffle
        │          │      └── module = Soup Pro Ellipse (single) = cd031b
        │          └── marqueur SINGLE
        └── kind cd:09 (single)
```

Single = tête `0x25`, kind `cd:09`, marqueur `c2 19`, identifiant `cd031b`, et `1a ff` (aucun cab2). C'est un **autre objet** que le dual.

### 4.1. Création (`head=0x31`) vs remplacement (`head=0x27`)

Ce document détaille surtout le **replace** live (cab1/cab2 dans un slot dual déjà présent). L’**assign initial** sur slot vide suit un autre bulk :

| Opération | Slot | Tête bulk | Taille | Kind bloc | Rôle |
|-----------|------|-----------|--------|-----------|------|
| **create** / `add` | vide | `0x31` | **60 o** | `cd:03` | Pose le dual parent ; cab2 usine = `cd02d6` (Jazz Rivet) |
| **replace** | occupé | `0x27` | **48 o** | `cd:0a` → reframe `cd:04` | Change cab1 ou cab2 (`cab_index` 0 ou 1) |

Flux typique après création : le cab2 usine (`cd02d6`) est modifiable par la séquence §5 (`focus → ed:08 → bulk 0x27`).

Dans HXLinux, `build_cab_dual_create_bulk` (`edit_slot_model.rs`) dérive le bulk 60 o du template catalogue `assign48_cd0a` en recopiant l’identité cab1 (`c319`…`1a`). Activé par défaut via `HX_CAB_DUAL_CREATE_HEAD31` (mettre `=0` pour l’ancien comportement head=27 sans cab2 enregistré).

### 4.2. Cas dégénéré en lecture de captures

Certaines captures de **replace cab2** montrent le **même hint dual deux fois** :

```
… c3 19  cd 03 1c  1a  cd 03 1c  00   ← cab1 et cab2 = Soup Pro (cd031c)
```

Exemple : `captures/usb-wireshark/Save/cab dual change right.json` — re-sélection du **même** Soup Pro Ellipse sur cab2.

> **Ce n’est pas la règle générale.** En création (`captures/usb-wireshark/add_dual_cab.json`) ou après scroll, cab2 est souvent un **autre** hint (ex. `cd02d6` usine, puis `cd0322`, `cd02d1`, etc.). Ne pas inférer « cab2 = copie de cab1 » à partir de ce cas dégénéré.

---

## 5. La séquence de remplacement d’un cab — le « fire »

Le même schéma s’applique au **cab1** ou au **cab2** (`cab_index` 0 ou 1) ; ce document détaille surtout cab2, cas le plus testé.

On a longtemps cru qu'il fallait un handshake bloquant : `focus → attendre dump → ed:08 → attendre IN 21 → finir`. **Faux.** Les captures montrent une **séquence tir-et-oublie** : on émet trois trames sur la lane `live_write`, la session vivante gère le reste. Pas d'attente de dump, pas d'attente d'IN 21 figée, pas d'ACK rejoués.

Avec `L = live_write_ctr` au moment du tir :

```
1.  focus   head=0x1d  lane=live_write  ctr = L          cd=0x04  src=LiveWrite
        ↓  (~93 ms)
2.  ed:08   head=0x08                   ctr = L + 0x11    ← arme le bulk
        ↓  (~400 ms)
3.  bulk    head=0x27                   ctr = L + 0x11    ← le modèle cab2
```

Détails qui comptent :

- **Le tag de lane interne au bloc** (octet juste après `cd:04`) suit aussi une séquence : `focus = tag Y`, `bulk = tag Y+1` (`slot_model_lane_seq`). Dans la capture validée, focus→`Y`, bulk→`0x19`.
- **Les octets 14–15 du bulk sont forcés à `0x00`** avant émission (voir §6.1).
- **Le cab2 du bulk doit être un cab dual** (`cd031c`/`c3 19`), pas le single (voir §6.2).
- Pendant la séquence, le device émet une rafale de notifications `IN head=0x1d` qu'on **ACK normalement** ; c'est attendu, pas une erreur.

Trace réelle d'un fire réussi (lane `live_write`, `L=0x6cbd`) :

```
cab_dual_focus  cd=0x04 src=LiveWrite  → focus ctr = 0x6cbd
FIRE  L=0x6cbd  (focus=L, ed08/bulk=0x6cce)
OUT  head=0x1d                                    ← focus
IN   head=0x21                                    ← device s'engage (IN 21)
IN   head=0x1d  (×N)  → ACK_1d send               ← notifications, ACKées
OUT  head=0x27  bulk len=48 …                     ← bulk modèle
OK   L=0x6cbd  model=0x6cce
```

---

## 6. Les pièges traversés (symptôme / cause / correctif)

### 6.1. L'octet 14 périmé

> **Symptôme.** Le fire s'exécute proprement (IN 21 reçu, aucun crash, lane cohérente), les logs sont parfaits — mais cab2 ne change pas sur le hardware.
>
> **Cause.** Le `bulkHex` catalogue `assign48_cd0a` porte un **octet 14 = `0x05`**, résidu de la session où la capture a été faite. `patch_bulk_header_counters` réécrit le ctr (octets 12–13) mais **laisse l'octet 14 intact**. Sur le fil, ce `0x05` décale la lane vers une page parasite → le device ignore le bulk.
>
> ```
> bulk émis (KO) : 27 … 00 04  ce 6c  05 00  01 …   ← byte14 = 05
> HX Edit  (OK)  : 27 … 00 04  8e 6e  00 00  01 …   ← byte14 = 00
> ```
>
> **Correctif.** Après isolation du pack `head=0x27`, forcer `bulk[14] = 0x00; bulk[15] = 0x00;`. C'est ce que font HX Edit et la création `head=31`.

### 6.2. cab2 single au lieu de cab2 dual — **le bug final**

> **Symptôme.** Le même `bulkHex` figé en dur dans une commande console changeait cab2 parfaitement ; le code UI, à structure identique, échouait. La seule variable : la console mettait cab2 en dur, l'UI le choisissait via le picker.
>
> **Cause.** Le picker servait pour cab2 le `bulkHex` **single** (`cd031b`, `c2 19`). Or, dans un slot dual, **le device n'accepte que des cabs de contexte dual** (`cd031c`, `c3 19`) aux deux emplacements. La console marchait par chance : son cab2 figé était `cd031c` (un cab dual). L'UI envoyait `cd031b` (un single) → rejet silencieux.
>
> ```
> console (OK) : … c3 19  cd 03 1c  1a  cd 03 1c     ← cab2 = cd031c (dual)
> UI     (KO)  : … c3 19  cd 03 1c  1a  cd 03 1b 00  ← cab2 = cd031b (single)
> ```
>
> **Correctif (version propre, juin 2026).** Le picker Cab 2 reste **Single IR** pour l’utilisateur (même nom de baffle qu’ailleurs). Au clic, `resolveCabDualCab2UsbWireFromPicker` mappe l’id single → entrée assign **dual** (`HD2_CabMicIr_FooWithPan`, `variant: dual`) ; `build_cab_dual_replace_cab_bulk` extrait le hint `c319` (ex. `cd031c`, champ cab1 du bulk WithPan) et le patche en cab2 — pas le bulk single `c219` / `cd031b`.

### 6.3. Contexte picker bloqué après un dual

> **Symptôme.** Après avoir changé cab2 avec succès, choisir une **Distortion** (ou tout autre bloc) dans le picker : l’assign échoue et l’UI revient au Cab dual initial.
>
> **Cause.** Le contexte Cab dual (`cabDualPickerSync` / `lastCabDualTabPanesContext`) restait actif : tout clic était routé vers `applyCabDualCabFromPickerListClick` (sous-remplacement cab) au lieu de `probe_slot_model_usb` replace slot entier.
>
> **Correctif.** `isCabDualSubCabPickerPick` : sous-cab **uniquement** si catégorie picker = Cab et variante single/legacy ; sinon `exitCabDualPickerModeForFullSlotReplace()` puis flux assign standard.

### 6.4. Liste picker vide en « Single Legacy » / « Dual Legacy »

> **Symptôme.** Sous-catégories Cab Legacy affichées mais liste de modèles vide.
>
> **Cause.** `usbAssignVariantFromPickerSub` mappait « Single Legacy » / « Dual Legacy » vers `variant: legacy`, alors que `HX_ModelUsbAssign.json` stocke ces cabs avec `variant: single` ou `dual` (hors périmètre IR de ce document, mais même mécanisme picker).
>
> **Correctif.** Cab + « Single Legacy » → `single` ; Cab + « Dual Legacy » → `dual`.

### 6.5. Les lanes fragmentées (rappel §3.3)

> **Cause.** Le refactor multithread a éclaté le compteur unique de HX en quatre lanes (`live_write`, `sq`, `editor`, `keepalive`). Construire le focus sur une lane et le bulk sur une autre viole la règle `+0x11`.
>
> **Correctif.** Ancrer focus + ed:08 + bulk sur la **seule** lane `live_write`, avec focus=`L`, ed:08/bulk=`L+0x11`.

### 6.6. Les ACK figés (crash)

> **Cause.** Une première console rejouait 15 ACK post-bulk **figés** depuis la capture HX Edit, et laissait le ed:08 gelé (compteur incohérent). Ces accusés désynchronisaient la session vivante → freeze du device.
>
> **Correctif.** Compteurs cohérents (focus=`L`, ed:08/bulk=`L+0x11`) **et** suppression des post-ACK figés : la session vivante gère elle-même la suite.

---

## 7. Pourquoi ça marche, maintenant

Le tableau des conditions à réunir simultanément :

| Condition | Valeur correcte | Piège évité |
|---|---|---|
| Lane unique et cohérente | focus=`L`, ed:08/bulk=`L+0x11`, page `live_write` | lanes fragmentées (§6.5) |
| En-tête bulk propre | octets 14–15 = `00 00` | résidu `0x05` (§6.1) |
| Contexte cab2 | cab dual `cd031c` / `c3 19` | single `cd031b` (§6.2) |
| Routage picker UI | sortie contexte dual si autre catégorie | sous-cab par erreur (§6.3) |
| Sub-kind | `cd:0a` reframé en `cd:04` | — |
| Pas de handshake bloquant | tir-et-oublie, session vivante | attente dump/IN 21 inutile |
| Pas d'ACK rejoués | aucun post-ACK figé | crash (§6.6) |

> *L'erreur de fond qu'on traînait n'était pas dans le handshake, ni dans la lane, ni dans le timing — tout ça était juste. Elle était dans une hypothèse de catalogue : croire que cab2 d'un dual était « le même baffle en version single » côté fil USB. Le device ne voit pas un nom : il voit `cd031b` (objet single, `c2 19`) là où il attend un hint dual `cd031c` / `c3 19`. Le picker peut rester Single IR : seul le chemin assign (`WithPan` + `variant=dual`) compte pour le bulk.*

---

## 8. Mémo express

```
SLOT DUAL — CHANGER CAB2
────────────────────────
L = live_write_ctr

focus  : head 1d · lane live_write · ctr = L        · cd 04 · src LiveWrite · tag Y
ed:08  : head 08 ·                   ctr = L + 0x11
bulk   : head 27 ·                   ctr = L + 0x11  · cd 04 (reframe de cd 0a) · tag Y+1
         └─ octets 14-15 = 00 00
         └─ corps : … c3 19  <cab1 dual cd03xx>  1a  <cab2 DUAL cd03xx>  00

RÈGLES D'OR
  • une seule lane (live_write), écart +0x11 entre focus et ed:08/bulk
  • octet 14 du bulk = 0x00 (jamais le résidu de capture)
  • cab1 ET cab2 = cabs DUAL (cd031c / c3 19), jamais single (cd031b / c2 19)
  • tir-et-oublie : pas d'attente de dump, pas d'IN 21 figé, pas d'ACK rejoués
```

---

## 9. Implémentation dans HXLinux

| Fichier | Rôle |
|---------|------|
| [`src-tauri/src/helix/cab_dual_cab2_replace.rs`](../src-tauri/src/helix/cab_dual_cab2_replace.rs) | Séquence fire `focus → ed:08 → bulk` ; force `bulk[14..15]=0` |
| [`src-tauri/src/helix/cab_dual_live_write.rs`](../src-tauri/src/helix/cab_dual_live_write.rs) | Construction du focus `head=0x1d`, lane `live_write` |
| [`src-tauri/src/helix/edit_slot_model.rs`](../src-tauri/src/helix/edit_slot_model.rs) | `build_cab_dual_replace_cab_bulk`, `build_cab_dual_create_bulk`, patch `c319` |
| [`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs) | `probe_slot_model_usb` — route replace cab2 si `cabDualCabIndex == 1` |
| [`src/models.ts`](../src/models.ts) | Picker Cab 1/2, `resolveCabDualCab2UsbWireFromPicker`, routage sortie dual |
| [`src-tauri/resources/HX_ModelUsbAssign.json`](../src-tauri/resources/HX_ModelUsbAssign.json) | `bulkHex` catalogue single vs WithPan / dual |

Captures de référence : `captures/usb-wireshark/Save/cab dual.json`, `captures/usb-wireshark/Save/cab dual change right.json`, `captures/usb-wireshark/add_dual_cab.json`.