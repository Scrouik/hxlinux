# Amp+Cab Legacy — protocole USB (hybrid)

**HXLinux — HX Stomp XL**  
*Cabs **Legacy hybrid** dans un couple ampli+cab (`assignVariant: amp+cab-legacy`). Complète [Amp_cab_fonctionnement_no_legacy.md](Amp_cab_fonctionnement_no_legacy.md) (IR).*

> **English:** [Amp_cab_operation_legacy.md](Amp_cab_operation_legacy.md)  
> **Captures:** `captures/usb-wireshark/Save/amp_cab legacy guitar.json`, `amp_cab legacy bass.json`

---

> **Synthèse.** Même slot **`c319`** qu’en IR, mais le cab est un **hybrid legacy** (suffixe modèle `64:83:17:c3:19`, cab court sur le fil). Focus cab = trame **`1b`** (pas `1d`). Params cab : **PP `0x08`**, sélecteurs **`0x25+`** (guitar) ou **`0x00+`** (compact) selon la taille du bloc ampli.

---

## 1. IR vs Legacy sur le même fil

| | **IR** `amp+cab` | **Legacy** `amp+cab-legacy` |
|--|------------------|------------------------------|
| Cab sur wire | `cd:03:xx` (3 octets) | Souvent **2 nibbles** (`47:00`, …) |
| Bloc param cab | `85:62` … `1d:c3:1a:01:1c` | `82:62` … `64:83:17:c3:19` |
| Focus onglet Cab | `1d`, `cd:03`, `1a:01` | `1b`, `cd:08` |
| PP live write | `0x03` | `0x08` |
| Picker catégorie | Amp+Cab | Amp+Cab **Legacy** / sous-cat legacy |

Entrées catalogue : paires **Guitar/Bass** injectées (`sync_usb_assign_from_catalog.py`), bulk unique **`8317c319`** + paire amp/cab.

---

## 2. Tables de sélecteurs cab (legacy)

Le routeur ne lit **pas** `preset_data` : il reçoit `ampCabAmpParamCount` = nombre de params **visibles** du panneau Amp (catalogue).

| Taille bloc ampli (proxy) | Table | Ex. 1er param cab (Level) |
|---------------------------|-------|---------------------------|
| **≥ 10** params (guitar) | `LEGACY_GUITAR_CAB_ROUTES` | `pSel=0x25`, tag `0x05` |
| **< 10** (compact / bass) | `LEGACY_COMPACT_CAB_ROUTES` | `pSel=0x00`, tag `0xcb` |

Code : `legacy_cab_wire_pair` dans `amp_cab_live_write.rs`.

Logs : `ppSource=amp_cab:legacy_table`, `pSelSource=amp_cab:legacy_guitar_sel` ou `legacy_compact_sel`.

---

## 3. Focus cab legacy (`1b`)

```text
1b … 80:10:ed:03 … 83:66:cd:08:04:64:21:65:81:66:bus:08:00:00:00
```

Capture : `amp_cab legacy guitar.json`. Envoyé au clic onglet Cab et en secours avant le premier `write_live_param` si pas encore focalisé.

---

## 4. Remplacement du **cab** seul (picker) — validé HW juin 2026

Changer le cab depuis l’onglet Cab **ne remplace pas** tout le slot. Le bulk est construit depuis **`HX_ModelUsbAssign.json`** (`build_amp_cab_replace_cab_bulk`) : seul le champ cab après `c319` / `1a` est patché. **`preset_data` n’intervient pas** sur ce chemin (il peut rester en retard en RAM ; l’UI utilise merge grace optimiste).

### 4.1 Cinématique USB legacy (≠ IR, ≠ Cab dual)

| Étape | Legacy `amp+cab-legacy` | IR `amp+cab` (référence) |
|-------|-------------------------|---------------------------|
| Préambule | **`ef` → `f0`** (16 o chacun) | `1d` focus cab → **`ed:08`** (16 o) |
| Bulk | head **`0x23`** (44 o) ou **`0x25`** (48 o) | head **`0x27`** / `0x25` selon entrée catalogue |
| Octets **14–15** du bulk | **`02 00`** — **ne pas écraser** | head `0x27` : zéros autorisés (chemin Cab dual) ; `0x25` : conserver `02 00` |

**Piège corrigé (juin 2026) :** réutiliser `focus → ed:08 → bulk` (comme Cab dual IR) ou forcer les octets 14–15 à `00 00` envoyait un bulk « OK » côté app mais **ignoré par le device**. Le legacy doit reprendre la **même séquence que l’assign initial** (`AddToEmpty` : `ef/f0/bulk`).

Capture assign de référence : `amp_cab legacy bass.json` frame **1357** (bulk `23…c319061a32`, octets 14–15 = `02 00`).

Implémentation : `execute_amp_cab_cab_replace` dans `amp_cab_cab_replace.rs` (branche `legacy=true`).

### 4.2 Champ cab sur le fil (hybrid compact)

Pour les slots legacy **1 octet** (ex. TucknGo bass), le token cab vient du **`chainHexHint`** de l’entrée cab catalogue (`33`, `37`, …), pas du bulk IR `cd02xx`. Un cab trop long (`cd024d` sur slot compact) est **refusé** avant envoi (`cab_field_bytes_for_amp_cab_replace`).

### 4.3 UI / picker

| Élément | Valeur |
|---------|--------|
| `dualPart` onglet Amp / Cab | `amp` / `cab` |
| `ampCabAssignVariant` | `"amp+cab-legacy"` (ne pas basculer vers `amp+cab` IR au scroll ampli) |
| `ampCabAmpParamCount` | Params visibles Amp → tables guitar vs compact |
| Picker onglet Cab | Catégorie **Cab** / sous-cat **Single Legacy** (pas forcer `Single` IR) |
| Probe | `probe_slot_model_usb` `replace` + `cabCatalogModelId` + variante ampli **legacy** |

Focus **`1b`** (§3) : onglet Cab et **params** live write — **pas** la cinématique du replace modèle cab.

---

## 5. Fichiers code

| Fichier | Rôle |
|---------|------|
| `amp_cab_cab_replace.rs` | Fire replace cab : `ef/f0/bulk` (legacy) vs `focus/ed:08/bulk` (IR) |
| `amp_cab_live_write.rs` | `build_amp_cab_legacy_param_model_block`, tables, focus `1b` |
| `edit_slot_model.rs` | `build_amp_cab_replace_cab_bulk`, `chainHexHint`, champ cab 1 octet |
| `models.ts` | Variante picker, `applyAmpCabCabFromPicker`, `isAmpCabSlotLegacy` |
| `hxModelCatalogMeta.ts` | `usbAssignVariantForAmpFamilyScroll` respecte Amp+Cab Legacy |

---

## 6. Checklist non-régression legacy

- [ ] Focus onglet Cab → HW sur cab (`1b`)
- [ ] Params cab (Level, …) → `pp=08`, sélecteur table guitar/compact cohérent
- [x] Changement cab picker → `ef/f0/bulk`, cab patché, octets 14–15 = `02 00`, HW réagit
- [ ] Picker reste **Amp+Cab Legacy** / **Single Legacy** après assign ampli
- [ ] Pas de fallback IR (`pp=03`, variante `amp+cab`) quand la variante est legacy

---

## 7. Relation Cab dual legacy

Les **duals** legacy (deux cabs) partagent le marqueur `c319` et des familles hybrid, mais :

- Cab dual → `dualPart` `cab1`/`cab2`, variante catalogue `dual` / `dual-legacy`
- Amp+Cab → `dualPart` `amp`/`cab`, variante `amp+cab` / `amp+cab-legacy`

Ne pas réutiliser les builders replace cab2 dual pour le cab Amp+Cab sans passer par `build_amp_cab_replace_cab_bulk`.
