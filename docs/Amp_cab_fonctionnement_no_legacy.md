# Amp+Cab IR — fonctionnement USB (params & focus cab)

**HXLinux — HX Stomp XL**  
*Document de référence pour les couples **Ampli + Cab IR** (`assignVariant: amp+cab`). Complément legacy : [Amp_cab_fonctionnement_legacy.md](Amp_cab_fonctionnement_legacy.md).*

> **English:** [Amp_cab_operation_no_legacy.md](Amp_cab_operation_no_legacy.md)  
> **Captures:** `captures/usb-wireshark/Save/amp_cab guitar.json`, `amp_cab bass.json` (voir [captures/usb-wireshark/README.md](../captures/usb-wireshark/README.md))

---

> **Synthèse.** Un slot Amp+Cab = **deux modèles distincts** (ampli + cab) dans **un seul bulk** (`…85188317c319…` + `<amp> 1a <cab>`). L’UI cible le sous-bloc via `dualPart` (`amp` / `cab`) et un **index param local** (0 = premier param **de ce** sous-modèle). Le fil USB cab IR utilise **PP `0x03`**, suffixe modèle **`1d:c3:1a:01:1c`**, focus onglet Cab = trame **`1d` `cd:03` `1a:01`** (comme Cab dual cab2 onglet).

---

## 1. Qu’est-ce qu’un « Amp+Cab » ?

| Concept | Détail |
|--------|--------|
| Slot matrice | **Une** case (un `slot_bus`) |
| Contenu | **Ampli** + **cab IR** liés (pas deux slots) |
| Catalogue | Entrée `HX_ModelUsbAssign` variante **`amp+cab`** (clone de l’entrée `amp` — `chainHexHint` souvent partagé) |
| UI | Onglets **Amp** / **Cab** ; picker onglet Cab verrouillé **Cab / Single** |
| Wire preset | Marqueur **`c319`** puis paire **`<amp_hex> 1a <cab_hex>`** (souvent terminaison `09`) |

**Ne pas confondre** avec **Cab dual** : deux cabs dans un slot Cab, `dualPart` = `cab1` / `cab2`, variante `dual`.

---

## 2. Modèle logiciel HXLinux (sans `preset_data` en session)

| Couche | Règle |
|--------|--------|
| Ciblage sous-modèle | `dualPart: "amp"` ou `"cab"` sur `write_live_param` |
| Index param | **Toujours local** au panneau actif (`paramIndexBase = 0`) — pas d’aplatissement « offset ampli » |
| Variante assign | `ampCabAssignVariant: "amp+cab"` (IR) ou `"amp+cab-legacy"` (hybrid — voir doc legacy) |
| Route live write cab | `resolve_cab_live_write_route` dans `amp_cab_live_write.rs` — **catalogue + compteurs UI**, pas le dump preset |
| Valeurs affichées | Cache session `slotChainSessionByKey` + overrides live ; hydratation **une fois** au changement de preset |

`preset_data` sert au **chargement** du preset (grille initiale), pas à router chaque slider cab en session.

---

## 3. Focus onglet **Cab** (hardware)

Quand l’utilisateur clique l’onglet **Cab** (ou ouvre le slot déjà sur Cab) :

| Variante | Tête | Bloc modèle (extrait) | Commande Tauri |
|----------|------|------------------------|----------------|
| **IR** `amp+cab` | `0x1d` | `83:66:cd:03` … `82:62:bus:1a:01` | `focus_amp_cab_usb_part` → `build_amp_cab_ir_cab_focus_packet` |
| **Legacy** `amp+cab-legacy` | `0x1b` | `83:66:cd:08` … | même commande → `build_amp_cab_cab_focus_packet` |

Implémentation IR : réutilise `build_cab_dual_cab2_tab_focus_packet` (`cd:03`, `1a:01`) — le suffixe cab Amp+Cab IR coincide avec le focus cab2 dual onglet.

**Effet attendu :** l’écran / encoders du Stomp se positionnent sur le **cab**, pas l’ampli.

---

## 4. Écriture live d’un paramètre **cab** (IR)

Capture de référence : `add_amp_cab_modif_param_cab.json` (mention dans `amp_cab_live_write.rs`).

| Champ | Valeur IR |
|-------|-----------|
| `dualPart` | `cab` |
| `param_index` | Local (ex. Mic = `0`) |
| PP (`pp` dans logs) | **`0x03`** |
| `param_selector` | = index local (`00` pour Mic) |
| Bloc modèle 16 o | `83 66 cd 03 YY 64 1e 65 **85** 62 bus **1d c3 1a 01 1c**` |

Chemin code : `write_live_param` → `resolve_live_write_route_override` (branche `cab`) → `build_live_write_frames_from_state` (préambule `08`/`f0`/`23`/`27` générique, **pas** le chemin minimal Cab dual).

Logs typiques : `ppSource=amp_cab:ir_capture`, `pSelSource=amp_cab:ir_local_index`.

---

## 5. Remplacement du **cab** seul (picker)

Changer le cab depuis l’onglet Cab **ne doit pas** remplacer tout le slot par un Cab single :

| Étape | Comportement |
|-------|----------------|
| UI | `probe_slot_model_usb` `replace` : `catalogModelId` = ampli, `cabCatalogModelId` = nouveau cab, `assignVariant` = **`amp+cab`** |
| Bulk | `build_amp_cab_replace_cab_bulk` depuis **`HX_ModelUsbAssign.json`** — patch **uniquement** le champ cab après `c319` / `1a`. **`preset_data` n’alimente pas ce bulk.** |
| Cinématique **IR** | **`1d` focus cab** → **`ed:08`** (16 o) → **bulk** (head souvent **`0x27`** ou **`0x25`** 48 o pour certains amplis guitar) |
| Cinématique **legacy** | **`ef` → `f0` → bulk`** — voir [Amp_cab_fonctionnement_legacy.md](Amp_cab_fonctionnement_legacy.md) §4 (≠ IR) |

Implémentation fire IR : `execute_amp_cab_cab_replace` (`legacy=false`) dans `amp_cab_cab_replace.rs` — lane `live_write` alignée sur Cab dual cab2 (`focus → ed:08 → bulk`). Octets bulk **14–15** = `02 00` conservés sur tous les heads assign (`0x23` / `0x25` / `0x27`).

Fichiers : `amp_cab_cab_replace.rs`, `edit_slot_model.rs`, `applyAmpCabCabFromPicker` dans `models.ts`.

---

## 6. Fichiers code

| Fichier | Rôle |
|---------|------|
| `src-tauri/src/helix/amp_cab_cab_replace.rs` | Fire replace cab : `focus/ed:08/bulk` (IR) vs `ef/f0/bulk` (legacy) |
| `src-tauri/src/helix/amp_cab_live_write.rs` | Blocs modèle IR/legacy, focus, `resolve_cab_live_write_route` |
| `src-tauri/src/lib.rs` | `probe_slot_model_usb` (branche `amp_cab_cab_replace`), `focus_amp_cab_usb_part`, `write_live_param` |
| `src/models.ts` | Onglets Amp/Cab, `ampCabAssignVariant`, `ampCabAmpParamCount`, picker cab |
| `src/hxModelCatalogMeta.ts` | Variantes `amp+cab` / détection Amp+Cab |

---

## 7. Checklist non-régression

- [ ] Clic onglet **Cab** → focus HW (IR et legacy)
- [ ] Param cab (Mic, Level, …) → `pSel` = index local, `pp=03` (IR)
- [x] Changer cab dans picker → `focus → ed:08 → bulk`, slot reste **Amp+Cab**, HW réagit (juin 2026)
- [ ] Pas de lecture `preset_data` pour router un slider cab en session

---

## 8. Pièges connus

1. **Envoyer `assignVariant: single`** depuis l’onglet Cab → le device traite un **Cab seul** (slot entier remplacé) — toujours `amp+cab` + `cabCatalogModelId`.
2. **Index global** (offset nombre de params ampli) → le device ignore ou modifie le mauvais bloc — `dualPart=cab` + index local.
3. **Oublier le focus cab** avant édition **params** → risque d’écrire sur l’ampli ; le **replace modèle** cab IR utilise `focus → ed:08 → bulk`, le legacy **`ef → f0 → bulk`** (pas le même chemin).
4. **Confondre replace legacy et IR** — `focus → ed:08 → bulk` sur legacy loggue « OK » mais le HW ignore ; voir [Amp_cab_fonctionnement_legacy.md](Amp_cab_fonctionnement_legacy.md) §4.
