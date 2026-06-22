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

## 4. UI HXLinux

| Élément | Valeur |
|---------|--------|
| `dualPart` onglet Amp | `amp` |
| `dualPart` onglet Cab | `cab` |
| `ampCabAssignVariant` | `"amp+cab-legacy"` |
| `ampCabAmpParamCount` | Longueur params visibles Amp (route guitar vs compact) |
| Replace cab picker | `assignVariant` ampli **legacy** + cab `single` / single legacy |

---

## 5. Fichiers code

| Fichier | Rôle |
|---------|------|
| `amp_cab_live_write.rs` | `build_amp_cab_legacy_param_model_block`, tables, focus `1b` |
| `edit_slot_model.rs` | `build_amp_cab_replace_cab_bulk`, `amp_cab_cab_field_range_in_bulk` |
| `models.ts` | `usbAssignVariantForAmpCabSlot`, onglets, `applyAmpCabCabFromPicker` |

---

## 6. Checklist non-régression legacy

- [ ] Focus onglet Cab → HW sur cab
- [ ] Params cab (Level, …) → `pp=08`, sélecteur table guitar/compact cohérent
- [ ] Changement cab picker → bulk `amp+cab-legacy` patché, slot reste Amp+Cab Legacy
- [ ] Pas de fallback IR (`pp=03`) quand la variante est legacy

---

## 7. Relation Cab dual legacy

Les **duals** legacy (deux cabs) partagent le marqueur `c319` et des familles hybrid, mais :

- Cab dual → `dualPart` `cab1`/`cab2`, variante catalogue `dual` / `dual-legacy`
- Amp+Cab → `dualPart` `amp`/`cab`, variante `amp+cab` / `amp+cab-legacy`

Ne pas réutiliser les builders replace cab2 dual pour le cab Amp+Cab sans passer par `build_amp_cab_replace_cab_bulk`.
