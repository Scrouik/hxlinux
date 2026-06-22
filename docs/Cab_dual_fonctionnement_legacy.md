# Cab dual Legacy — protocole USB `ed:03`

**HXLinux — HX Stomp XL**  
*Cabs **Legacy** hybrid (pré-3.50) — complément de [Cab_dual_fonctionnement_no_legacy.md](Cab_dual_fonctionnement_no_legacy.md) (IR / WithPan).*

> **English version:** [Cab_dual_operation_legacy.md](Cab_dual_operation_legacy.md)

**Captures (2026-06-22) :**
- `captures/usb-wireshark/add_dual_legacy_change_cab2.json` — replace cab2 isolé
- `captures/usb-wireshark/add_dual_legacy_change_cab2_&_dual.json` — scénario complet (11 trames modèle exploitables)

---

> **Synthèse.** Les duals legacy partagent **exactement le même transport** que les duals IR (lane unique `live_write`, écart `+0x11`, octet 14 à zéro, tir-et-oublie) — les §3/§5 du [doc IR](Cab_dual_fonctionnement_no_legacy.md) se réutilisent tels quels. La divergence est **entièrement dans le corps du bulk** : têtes différentes, scaffold plus long, sélecteurs cab **1 octet** (majorité) ou **`cd02xx` sur 3 octets** (sous-famille hybrid longue).

---

## 1. Transport : identique à l’IR

Aucune différence sur la lane. Chaque écriture respecte `focus + 0x11` = ed:08 = bulk, octet 14 à zéro :

```
focus 0xd350 → bulk   0xd361   (+0x11)   replace cab2
focus 0xd49e → bulk   0xd4af   (+0x11)   replace dual (head 0x25)
focus 0xd51a → bulk   0xd52b   (+0x11)   replace cab2 (1er d’une série)
```

La logique de fire (`focus → ed:08 → bulk` sur `live_write`) se réutilise **sans modification** (`cab_dual/replace_fire.rs`).

---

## 2. Carte des têtes legacy

| Tête | Opération | Encodage cab2 / remarque |
|------|-----------|--------------------------|
| `0x2d` | Assign / create dual (slot initial) | descripteur complet `30 09 10 0a c3` |
| `0x25` | Changer le **dual parent** (picker slot entier) | défaut `cd 01 63` (3 o) |
| `0x25` | Changer **cab2** si hint `cd02xx` / `cd01xx` (3 o) | `c3 19 <cab1:1o> 1a cd02xx 00 00 00` — **48 o** |
| `0x23` | Changer **cab2** si hint **1 octet** | `c3 19 <cab1> 1a <cab2> 00` — **44 o** |
| `0x1d` | Focus | — |

*Rappel IR : `0x31` create, `0x27` replace cab2, `0x25` = assign **single** (pas dual).*

> **Piège documenté (juin 2026, validé HW).** Ne pas envoyer un bulk **`0x23` de 46 o** (cab2 `cd02xx` patché dans un template 44 o) : le Stomp ignore le bulk. HX Edit n’utilise jamais cette forme dans les captures.

---

## 3. Champ cab : deux familles d’encodage

Divergence structurelle par rapport à l’IR :

```
IR      :  c3 19  cd 03 1c   1a  cd 03 1b   00     (cab = 3 octets)
Legacy  :  c3 19  33         1a  33         00     (cab = 1 octet, head 0x23)
Legacy  :  c3 19  30         1a  cd 02 4e  00 00 00   (cab2 = 3 o, head 0x25)
```

### 3.1 Replace cab2 — sélecteur 1 octet (`head 0x23`, 44 o)

`chainHexHint` sur **2 caractères hex** (ex. `2e`, `31`, `30`) :

```
c3 19  <cab1 : 1 octet>  1a  <cab2 : 1 octet>  00
```

Exemples validés HW (Lead 80 dual, cab1 = `30`) : Celest 12H (`2e`), US Deluxe (`31`), Field Coil (`2f`).

### 3.2 Replace cab2 — hint `cd02xx` (`head 0x25`, 48 o)

`chainHexHint` sur **6 caractères** (`cd024e`, `cd0228`, `cd0227`, …) — entrée catalogue `variant: dual` avec `bulkKind: assign48` :

```
c3 19  <cab1 : 1 o>  1a  cd 02 xx  00 00 00
```

Exemples validés HW : Princess Blue (`cd024e`), Grammatico (`cd0228`), Fullerton (`cd0227`).

**Construction HXLinux** (`build_legacy_cab2_replace_bulk` dans `cab_dual/legacy/wire.rs`) :

1. Partir du bulk **dual** `0x25` du cab pické (squelette 48 o).
2. Remplacer **cab1** par celui du dual parent (ex. `30` = Lead 80).
3. Remplacer **cab2** par le hint 3 o (`cd02xx`) — swap de longueur cab1/cab2, **total inchangé** (48 o).

Ne **pas** patcher le hint 3 o dans un template parent `0x23` (44 o) : cela produit 46 o et échoue silencieusement sur le matériel.

### 3.3 Règle catalogue rapide

| `chainHexHint` | `bulkKind` dual (catalogue) | Tête replace cab2 |
|----------------|----------------------------|-------------------|
| `2e`, `31`, `33`, … (≤ 2 hex) | `assign44_cd04_…` | **`0x23`** (44 o) |
| `cd024e`, `cd0227`, … (6 hex) | `assign48_cd04_…` | **`0x25`** (48 o) |

Sélecteurs 1 o observés sur les captures (`add_dual_legacy_change_cab2_&_dual.json`) :

| Emplacement | Valeurs | Modèle (`chainHexHint`) |
|-------------|---------|-------------------------|
| cab1 | `0x33`, `0x30` | Soup Pro Ellipse, 1x12 Lead 80 |
| cab2 | `0x33`, `0x2e`, `0x38`, `0x47` | Soup Pro, Celest 12H, Jazz Rivet, WhoWatt 100 |

Le piège `cd031b` / `cd031c` **ne s’applique pas** : ici c’est **`c219` + hint single** vs **`c319` + hint dual**. Le device **développe** le sélecteur en descripteur complet dans les échos IN :

```
OUT bulk  :  c3 19  33  1a  2e  00
IN echo   :  c3 19  33  1a  2e  09 10 0a c3 …
```

**Mapping catalogue :** les sélecteurs 1 o = `chainHexHint` dans `HX_ModelUsbAssign.json` (entrée `variant: dual` pour le wire). Pas de table ad hoc à inventer — ne jamais patcher depuis le bulk **single** `c219`.

---

## 4. Défaut `cd0163` au changement de dual (`head 0x25`)

Quand l’utilisateur choisit un **autre** dual legacy dans le picker (replace slot entier) :

```
head 0x25 :  c3 19  <cab1 : 1 o>  1a  cd 01 63  00 00 00
```

- **cab1** = sélecteur du nouveau dual parent (ex. `0x30` = Lead 80).
- **cab2** = défaut **3 octets** `cd0163` — équivalent legacy du Jazz Rivet `cd02d6` en IR.

D’où une **longueur de champ variable** (1 o en replace cab2, 3 o en défaut dual) : rôle de `amp_cab_cab_field_range_in_bulk` dans `edit_slot_model.rs` (terminaison `0x09` ou 1/3 octets selon contexte).

> **Point ouvert (§9.8).** Confirmer par une capture « change dual → lecture sans toucher cab2 » que `cd0163` est bien le défaut systématique du `0x25`, et non un artefact de la transition (#10837).

---

## 5. Focus mutualisé

Un **seul focus** peut couvrir **plusieurs** replace cab2 consécutifs sur le même slot. Après le dual Lead 80 :

```
#17059  focus  1d  L=0xd51a  1a:01     ← un seul focus
#18223  bulk   23  ctr=0xd52b  30 1a 2e
#19153  bulk   23  ctr=0xd594  30 1a 38   ← lane avance entre bulks
#20045  bulk   23  ctr=0xd5fd  30 1a 47
```

Le focus persiste tant qu’on reste sur le même emplacement ; la **lane avance** à chaque bulk (`d52b` → `d594` → `d5fd`). Implémentation : ne pas supposer un focus systématique avant chaque `0x23` si le contexte UI n’a pas changé de slot/onglet.

---

## 6. Timeline décodée (`add_dual_legacy_change_cab2_&_dual.json`)

```
ASSIGN dual legacy
 5361  2d  d2e7   c3 19 33 1a [30 09 10 0a c3]   create, cab2 descripteur complet

CHANGER cab2 ×2
 6643  1d  d350   focus (cab2)
 8707  23  d361   c3 19 33 1a 33                 cab2 → Soup Pro
10027  23  d3ca   c3 19 33 1a 2e                 cab2 → Celest 12H (cab1 ≠ cab2)

CHANGER le dual legacy
10837  25  d433   c3 19 33 1a cd0163             bulk intermédiaire (picker)
13055  1d  d49e   focus (cab1 / slot)
15387  25  d4af   c3 19 30 1a cd0163             cab1 → Lead 80 (0x30)

CHANGER cab2 ×3
17059  1d  d51a   focus                          ← focus mutualisé
18223  23  d52b   c3 19 30 1a 2e
19153  23  d594   c3 19 30 1a 38
20045  23  d5fd   c3 19 30 1a 47
```

---

## 7. IR vs Legacy — tableau de synthèse

| Aspect | Dual IR (WithPan) | Dual legacy |
|--------|-------------------|-------------|
| Lane / `+0x11` / octet 14 | identique | identique |
| Tête bulk **cab2** (1 o) | `0x27` (48 o) | **`0x23`** (44 o) |
| Tête bulk **cab2** (`cd02xx`) | — | **`0x25`** (48 o) |
| Tête **changement dual** | `0x27` / create `0x31` | **`0x25`** / assign **`0x2d`** |
| Marqueur dual | `c3 19` | `c3 19` |
| Identifiant cab | `cd03xx` (3 o) | **1 o** ou **`cd02xx`** (3 o) |
| Longueur champ cab | fixe | **variable** selon tête / opération |
| Défaut cab2 (create / dual) | `cd02d6` | **`30`** (create) / **`cd0163`** (`0x25` parent) |
| Piège single vs dual | `cd031b` vs `cd031c` | `c219` vs `c319` + hint |
| Piège replace cab2 | — | **jamais** `0x23` + cab2 3 o (46 o) |

---

## 8. Mémo express

```
SLOT DUAL LEGACY — CHANGER CAB2
───────────────────────────────
Transport : IDENTIQUE IR (focus → ed:08 → bulk, live_write, +0x11, byte14=0)
            + un focus peut couvrir plusieurs bulks consécutifs

CAB2 — hint 1 octet (chainHexHint ≤ 2 hex)
  bulk : head 0x23, 44 o
  corps : c3 19 <cab1:1o> 1a <cab2:1o> 00

CAB2 — hint cd02xx (6 hex, ex. cd024e)
  bulk : head 0x25, 48 o
  corps : c3 19 <cab1:1o> 1a cd02xx 00 00 00
  build : template dual 0x25 du cab pické + swap cab1 parent / cab2 hint

INTERDIT : head 0x23 + cab2 sur 3 o → 46 o → HW ignore

DUAL PARENT (picker slot entier) : head 0x25, cab2 défaut cd0163
CREATE slot vide : head 0x2d
```

---

## 9. Implémentation HXLinux (`cab_dual/`)

| Fichier | Rôle |
|---------|------|
| `cab_dual/replace_fire.rs` | Fire agnostique ; bulk `0x23` / `0x25` (legacy) ou `0x27` (IR) |
| `cab_dual/legacy/wire.rs` | `build_legacy_cab2_replace_bulk` — routage 1 o → `0x23`, `cd02xx` → `0x25` |
| `cab_dual/ir/` | Focus / live_write IR (`cab_dual_live_write.rs`) |
| `edit_slot_model.rs` | `build_cab_dual_replace_cab_bulk` délègue au wire legacy si besoin |
| `src/models.ts` | `resolveCabDualCab2UsbWireFromPicker` → `variant: dual` (sans WithPan) |

---

## 10. Checklist

- [x] Transport `+0x11`
- [x] cab1 ≠ cab2 (`33 1a 2e`)
- [x] Têtes `0x23` / `0x25` / `0x2d`
- [x] Mapping `chainHexHint` catalogue
- [x] Focus mutualisé (3 bulks / 1 focus)
- [x] Replace cab2 : routage `0x23` (1 o) vs `0x25` (`cd02xx`) — **validé HW juin 2026**
- [ ] Confirmer défaut `cd0163` sur `0x25` (lecture post-change dual)
- [ ] Replace **cab1** seul
- [ ] Create dual legacy (`0x2d`) vs IR (`0x31`)

---

*Contenu fusionné depuis `dual_legacy_part.md` (juin 2026).*
