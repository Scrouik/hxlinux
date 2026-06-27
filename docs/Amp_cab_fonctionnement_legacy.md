# Amp+Cab Legacy — protocole USB (hybrid)

**HXLinux — HX Stomp XL**  
*Cabs **Legacy hybrid** dans un couple ampli+cab (`assignVariant: amp+cab-legacy`). Complète [Amp_cab_fonctionnement_no_legacy.md](Amp_cab_fonctionnement_no_legacy.md) (IR).*

> **English:** [Amp_cab_operation_legacy.md](Amp_cab_operation_legacy.md)  
> **Captures clés :**
> - `captures/usb-wireshark/Save/amp_cab legacy guitar.json` — assign, scroll, familles de bulks
> - `captures/usb-wireshark/ampcab_legacy_switch_tab.json` — focus onglets Amp / Cab (`1d`)
> - `captures/usb-wireshark/ampcab_legacy_change_cab_HXEdit.json` — replace cab WhoWatt → Soup Pro (#4401)

---

> **Synthèse (juin 2026, validé HW).** Même marqueur fil **`c3:19`** qu’en IR, cab **hybrid legacy** (hint court ou `cd:02:xx`). Assign et replace modèle passent par le bulk **`HX_ModelUsbAssign.json`** (`variant: amp+cab-legacy`), **pas** par `preset_data`. Lane bloc modèle : **`cd:07`** à l’assign, **`cd:03`** au replace cab et au focus onglet. Focus UI = trame **`1d`** (pas `1b`) + **`ed:08`**. Règle picker : **tout cab legacy** sur **Amp+Cab Legacy** ; **tout cab IR** sur **Amp+Cab** IR.

---

## 1. IR vs Legacy sur le même fil

| | **IR** `amp+cab` | **Legacy** `amp+cab-legacy` |
|--|------------------|------------------------------|
| Cab sur wire | `cd:03:xx` (3 o, MicIr) | **1 octet** (`33`, `47`…) ou **`cd:02:xx`** (3 o) selon l’ampli |
| Bloc param cab (live write) | `85:62` … `1d:c3:1a:01:1c`, PP **`0x03`** | `82:62` … `64:83:17:c3:19`, PP **`0x08`** |
| Focus / replace modèle cab | `1d`, `cd:03`, `1a:01` → `ed:08` → bulk | **idem** (`1d`, pas `1b`) |
| Picker | Cab **Single** (IR) | Cab **Single Legacy** |
| Variante USB | `amp+cab` | `amp+cab-legacy` |

Entrées catalogue : paires Guitar/Bass injectées (`sync_usb_assign_from_catalog.py`), bulk avec **`83:17:c3:19`** + paire amp/cab sur le fil.

---

## 2. Lane `cd:07` (assign) vs `cd:03` (replace / focus)

Le bulk catalogue **`amp+cab-legacy`** embarque un bloc modèle `83:66:cd:**07**:TAG` (ex. tag `fb` sur WhoWatt, frame **#1259**).

| Phase | Lane `cd` | Exemple capture |
|-------|-----------|-----------------|
| **Assign** slot vide (`AddToEmpty`) | **`07`** | `amp_cab legacy guitar.json` #1259 — head `23`, 44 o |
| **Replace cab** seul | **`03`** | `ampcab_legacy_change_cab_HXEdit.json` #4401 — reframe `07→03` avant envoi |
| **Focus onglet** Amp / Cab | **`03`** | `ampcab_legacy_switch_tab.json` — suffixe `1a:00` (Amp) ou `1a:01` (Cab) |

**Piège corrigé :** envoyer le bulk assign tel quel (`cd:07`) lors d’un replace cab → trame acceptée côté app, **ignorée** par le HW.  
**Implémentation :** `reframe_legacy_replace_cd07_to_cd03` dans `amp_cab_cab_replace.rs` ; même reframe si template création `2d` (optionnel, voir §5).

Le tag session (octet après `cd:XX`) est mémorisé par slot :
- **Assign** : `(amp_tag, cab_tag)` depuis le bulk (`cd:07` ou `cd:03`), ex. `fb`
- **Replace cab** : met à jour **`cab_tag`** seulement
- **Focus onglet Amp** : utilise **`amp_tag`** (ne pas réutiliser `live_write_yy` post-replace — sinon mauvais sous-bloc, ex. Soup Pro affiché à la place de WhoWatt)

---

## 3. Focus onglet Amp / Cab — trame `1d`

HX Edit (et HXLinux depuis juin 2026) bascule les onglets Amp/Cab avec **`1d`**, pas `1b`.

### 3.1 Enveloppe (capture `ampcab_legacy_switch_tab.json`)

```text
1d … 80:10:ed:03 … sub=04 … 83:66:cd:03:TAG:64:4e:65:82:62:bus:1a:SUFFIX:00:00:00
                                                      ↑              ↑
                                                   lane focus    00=Amp, 01=Cab
```

Puis **`ed:08`** (~93 ms), puis poke **`f0`** :
- onglet **Cab** : `f0:08`
- onglet **Amp** : `f0:10` puis `f0:08` (frame **#2659**)

Tags observés après assign / bascules : `fb` → `fc` → `fd` → `fe` (progression session ; le focus Amp doit rester sur le tag **ampli** mémorisé à l’assign).

### 3.2 Replace cab — focus cab **obligatoire** avant le bulk

Séquence validée HW (`execute_amp_cab_cab_replace`) :

```text
1d focus cab (cd:03, 1a:01)  →  ed:08  →  ~1100 ms  →  bulk replace (head 23/25/27)
```

**Pourquoi :** sans focus cab préalable, le device n’est pas positionné sur le sous-bloc cab ; le bulk part « OK » côté logs USB mais le HW ne change pas le cab (ou corrompt l’état ampli).

Legacy et IR partagent cette cinématique pour le **replace modèle** ; seul le contenu du bulk diffère.

### 3.3 `1b` / `cd:08` (historique)

D’anciennes captures (`amp_cab legacy guitar.json`) montrent un focus **`1b`** + `cd:08` pour les **params** cab en live write. Le chemin **modèle** (assign / replace / onglets UI) utilise **`1d` + `cd:03`**. Ne pas mélanger les deux.

---

## 4. Assign initial (1er clic slot vide)

### 4.1 Bulk à envoyer

| Tentative | Résultat HW |
|-----------|-------------|
| head **`2d`**, 56 o, `cd:03` (template création) | ❌ 1er clic ignoré ; 2ᵉ clic (replace) seul fonctionnait |
| head **`23`**, 44 o, `cd:07` (bulkHex catalogue assign) | ✅ frame **#1259** |

**Fix :** `HX_AMP_CAB_LEGACY_CREATE_HEAD2D` **OFF** par défaut — le 1er clic envoie le même bulk **`23` / `cd:07`** que le catalogue, pas le template `2d`.

Octets **14–15** du bulk : **`02 00`** sur heads `23` / `25` / `27` — **ne pas écraser** à `00 00`.

### 4.2 UI

- Après probe add : `suppressNextAmpCabFocusUsb` pour éviter un focus `1d` parasite au re-render
- Variante figée : **`amp+cab-legacy`** (ne pas repasser IR au scroll HW)

---

## 5. Replace cab seul (picker onglet Cab)

### 5.1 Construction du bulk

`build_amp_cab_replace_cab_bulk` :
1. Copie le bulk **ampli** parent (`amp+cab-legacy`)
2. Patch **uniquement** le champ cab après `c3:19` / `1a`
3. **Conserve le fil ampli** avant `1a` (ex. WhoWatt `2c`) — **ne jamais** le recopier depuis une autre entrée catalogue ayant le même hint cab

**Piège corrigé (corrélation « noms proches ») :** une recherche catalogue « amp+cab-legacy avec le même cab » remplaçait le fil `2c` par `23` (Soup Pro) → au retour onglet Amp, le HW affichait le mauvais ampli.  
Capture HX Edit #4401 : WhoWatt + Soup Pro = **`2c 1a 33`**, pas `23 1a 33`.

### 5.2 Encodage cab — compact vs long

Deux formes sur le fil `… c3:19 <wire> 1a <cab> …` :

| Famille ampli (ex.) | Head | Fil ampli | Cab défaut | Ex. |
|---------------------|------|-----------|------------|-----|
| **Compact** | `23` (44 o) | 1 o (`2c`, `2b`…) | 1 o (`47`, `34`…) | WhoWatt, US Small Tweed, Tuck’n Go |
| **Long** | `27` (48 o) | 3 o `cd:02:xx` | 3 o `cd:02:xx` | Fullerton Jump, US Princess |
| **Mixte** | `25` (48 o) | 3 o | 1 o | Voltage Queen, US Super |

**Règle picker (produit) :** sur Amp+Cab Legacy, **n’importe quel cab legacy** ; sur Amp+Cab IR, **n’importe quel cab IR**.

**Adaptation fil USB** (`cab_field_bytes_for_amp_cab_replace`) :

| Slot parent | Cab catalogue | Champ envoyé |
|-------------|---------------|--------------|
| 1 o | hint `33` | `33` |
| 3 o | hint `33` | `cd:02:33` |
| 1 o | hint `cd024e` | `4e` (3ᵉ octet de `cd:02:4e`) |
| 3 o | hint `cd024e` | `cd:02:4e` |

Ne **pas** rejeter un cab `cd02xx` sur slot compact — HX Edit autorise la combinaison (ex. US Small Tweed + 1x12 US Princess).

### 5.3 Probe / UI

| Élément | Valeur |
|---------|--------|
| `dualPart` | `amp` / `cab` |
| `assignVariant` ampli | `"amp+cab-legacy"` |
| Probe | `replace` + `catalogModelId` (ampli) + `cabCatalogModelId` + `cabAssignVariant` (`single` / `legacy`) |
| Optimistic UI | merge grace ; pas de relecture `preset_data` |

---

## 6. Tables live write params cab (legacy)

Le routeur reçoit `ampCabAmpParamCount` = params **visibles** du panneau Amp.

| Taille bloc ampli (proxy) | Table | Ex. Level cab |
|---------------------------|-------|---------------|
| **≥ 10** (guitar) | `LEGACY_GUITAR_CAB_ROUTES` | `pSel=0x25`, tag `0x05` |
| **< 10** (compact / bass) | `LEGACY_COMPACT_CAB_ROUTES` | `pSel=0x00`, tag `0xcb` |

Code : `legacy_cab_wire_pair` dans `amp_cab_live_write.rs`.

---

## 7. Récap des bugs rencontrés et causes

| Symptôme | Cause | Fix |
|----------|-------|-----|
| 1er clic assign ignoré HW | bulk `2d` / `cd:03` au lieu de `23` / `cd:07` | head `23` par défaut |
| Replace cab « OK » logs, HW inchangé | pas de focus cab ; ou `cd:07` au lieu de `cd:03` | `1d` → `ed:08` → bulk ; reframe `07→03` |
| Cab accepté si nom proche ampli, ampli bascule au retour Amp | fil ampli écrasé via catalogue (ex. `2c→23`) | conserver fil ampli parent |
| Soup Pro sur onglet Amp après replace | focus Amp avec tag session cab / `live_write_yy` | tag **amp** mémorisé à l’assign |
| Fullerton + Soup Pro : erreur taille cab | hint `33` non expandu en `cd:02:33` | conversion compact ↔ long |
| Small Tweed + Princess : refus « hybrid court » | garde-fou HXLinux (pas HX Edit) | hint `cd024e` → octet `4e` |
| Octets 14–15 à `00 00` | finalize bulk replace trop agressif | laisser `02 00` sur heads connus |

---

## 8. Fichiers code

| Fichier | Rôle |
|---------|------|
| `amp_cab_cab_replace.rs` | Replace cab : focus `1d` → `ed:08` → bulk ; reframe `cd:07→cd:03` |
| `amp_cab_live_write.rs` | Focus onglets `1d`, tags session, tables PP legacy, record assign/replace |
| `edit_slot_model.rs` | `build_amp_cab_replace_cab_bulk`, assign head `23`, encodage cab compact/long |
| `cab_dual/legacy/wire.rs` | `legacy_compact_hint_to_cd02_field`, `legacy_cd02_field_to_compact_hint` |
| `models.ts` | Picker, `applyAmpCabCabFromPicker`, focus onglets, `suppressNextAmpCabFocusUsb` |
| `lib.rs` | `probe_slot_model_usb`, `focus_amp_cab_usb_part`, `record_amp_cab_assign_session` |

---

## 9. Checklist non-régression legacy

- [x] Assign 1er clic slot vide → HW réagit (bulk `23`, `cd:07`)
- [x] Replace cab picker → focus cab puis bulk ; HW change le cab
- [x] Replace cab : fil ampli inchangé (WhoWatt `2c` + Soup Pro `33`)
- [x] Retour onglet Amp après replace → ampli correct (tag amp, pas tag cab)
- [x] Compact + cab `cd02xx` (Princess sur Small Tweed)
- [x] Long + cab compact (Fullerton + Soup Pro → `cd:02:33`)
- [ ] Params cab live write → `pp=08`, sélecteur guitar/compact cohérent
- [ ] Picker reste Legacy après scroll HW
- [ ] Pas de bascule IR (`amp+cab`) sur slot legacy

---

## 10. Relation Cab dual legacy

Les duals legacy partagent `c3:19` et des hints hybrid, mais :

- Cab dual → `dualPart` `cab1`/`cab2`, variante `dual` / `dual-legacy`
- Amp+Cab → `dualPart` `amp`/`cab`, variante `amp+cab` / `amp+cab-legacy`

Ne pas réutiliser les builders replace cab2 dual pour le cab Amp+Cab : passer par **`build_amp_cab_replace_cab_bulk`**.
