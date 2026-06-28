# Captures USB (Wireshark / USBPcap)

Exports JSON Wireshark (`usb.capdata`) pour reverse du protocole Helix — **hors** du crate Tauri.

## Emplacement

Ce dossier remplace l’ancien `src-tauri/paquets JSON/` (qui provoquait des rebuilds
lourds ou des blocages quand on y copiait de gros fichiers pendant `npm run tauri dev`).

## Usage

- Copier ici les nouvelles captures ; elles ne sont **pas** compilées ni bundlées.
- Fichiers en général **non versionnés** (voir `.gitignore`).
- Scripts :
  - `scripts/analyze_ed03_captures.py` — lanes ED03 / preset
  - `scripts/analyze_stomp_running_compare.py` — amorçage ARM + fond scroll (`stomp_running_*`)
  - `scripts/inject_bulk_from_captures.py` — injection `bulkHex` par catégorie + variante (`chainHexHint`)
  - `scripts/inject_eq_bulk_from_captures.py` — raccourci EQ mono/stéréo

## Convention de nommage

| Type de capture | Exemples | But |
|-----------------|----------|-----|
| **Campagne bulkHex** (assign HX Edit) | `eq Mono.json`, `reverb stereo.json`, `modulation legacy.json` | Remplir `bulkHex` dans `HX_ModelUsbAssign.json` |
| **Scroll / session** | `stomp_running_start_hxedit.json`, `3_scroll_HXEdit.json` | Debug dialogue `1d`/`1f`, pull `1b`, dumps `IN 53` |
| **Connect / preset** | `01_connect_HXEdit.json` | Amorçage, lanes, bootstrap preset |

Une capture par famille / variante pour les campagnes bulkHex ; nom explicite (`eq Mono.json`, `modulation stereo.json`, …).

Injection après capture assign :

```bash
python3 scripts/inject_bulk_from_captures.py Modulation \
  mono:"captures/usb-wireshark/modulation mono.json" \
  stereo:"captures/usb-wireshark/modulation stereo.json" \
  legacy:"captures/usb-wireshark/modulation legacy.json"

python3 scripts/inject_bulk_from_captures.py Delay --allow-partial \
  mono:"captures/usb-wireshark/delay mono.json" \
  stereo:"captures/usb-wireshark/delay stereo.json" \
  legacy:"captures/usb-wireshark/delay lecacy.json"

python3 scripts/inject_bulk_from_captures.py Reverb \
  mono:"captures/usb-wireshark/reverb mono.json" \
  stereo:"captures/usb-wireshark/reverb stereo.json" \
  legacy:"captures/usb-wireshark/reverb lecacy.json"

python3 scripts/inject_bulk_from_captures.py "Pitch/Synth" --allow-partial \
  mono:"captures/usb-wireshark/pitch_synth mono.json" \
  stereo:"captures/usb-wireshark/pitch_synth stereo.json" \
  legacy:"captures/usb-wireshark/pitch_synth legacy.json"

python3 scripts/inject_bulk_from_captures.py Filter \
  mono:"captures/usb-wireshark/filter mono.json" \
  stereo:"captures/usb-wireshark/filter stereo.json" \
  legacy:"captures/usb-wireshark/filter legacy.json"

python3 scripts/inject_bulk_from_captures.py Wah \
  mono:"captures/usb-wireshark/wah mono.json" \
  stereo:"captures/usb-wireshark/wah stereo.json"

# Amp seul (sans cab) — clés guitar/bass → variant `amp` + subCategory
python3 scripts/inject_bulk_from_captures.py Amp \
  guitar:"captures/usb-wireshark/amp guitar.json" \
  bass:"captures/usb-wireshark/amp bass.json"

# Amp+Cab IR (un bulk combiné amp+cab, marqueur c319 — pas deux bulkHex)
python3 scripts/inject_bulk_from_captures.py "Amp+Cab" \
  guitar:"captures/usb-wireshark/amp_cab guitar.json" \
  bass:"captures/usb-wireshark/amp_cab bass.json"

python3 scripts/inject_bulk_from_captures.py "Amp+Cab Legacy" \
  guitar:"captures/usb-wireshark/amp_cab legacy guitar.json" \
  bass:"captures/usb-wireshark/amp_cab legacy bass.json"

python3 scripts/inject_bulk_from_captures.py Preamp \
  guitar:"captures/usb-wireshark/preamp guitar.json" \
  bass:"captures/usb-wireshark/preamp bass.json" \
  mic:"captures/usb-wireshark/preamp mic.json"

python3 scripts/inject_bulk_from_captures.py Cab --allow-partial \
  single:"captures/usb-wireshark/cab single.json" \
  dual:"captures/usb-wireshark/cab dual.json"

python3 scripts/inject_bulk_from_captures.py Cab --allow-partial \
  legacy-single:"captures/usb-wireshark/cab single legacy.json" \
  legacy-dual:"captures/usb-wireshark/cab dual legacy.json"

python3 scripts/inject_bulk_from_captures.py IR \
  single:"captures/usb-wireshark/IR Single.json" \
  dual:"captures/usb-wireshark/IR Dual.json"

python3 scripts/inject_bulk_from_captures.py "Volume/Pan" \
  mono:"captures/usb-wireshark/volume_pan mono.json" \
  stereo:"captures/usb-wireshark/volume_pan stereo.json"

python3 scripts/inject_bulk_from_captures.py Looper --allow-partial \
  mono:"captures/usb-wireshark/looper mono.json" \
  stereo:"captures/usb-wireshark/looper stereo.json"

python3 scripts/inject_bulk_from_captures.py "Send/Return" --allow-partial \
  mono:"captures/usb-wireshark/sendreturn mono.json" \
  stereo:"captures/usb-wireshark/sendreturn stereo.json"
```

Utiliser `--allow-partial` si une entrée assign manque encore.

**Scroll vs assign** : une capture scroll (`1b:00` pull, IN `8213` + `c219…`) ne contient en général **pas** de bulk assign (`25:00`…). Ex. Poly Sustain : fil IN `c219cd0265` (frame 909) — le bulk assign est dans `delay mono.json` ; l’entrée cachée `L6SPB_InfSustain` avait un hint catalogue `cd0243` faux.

### Campagnes bulkHex — état (juin 2026)

| Famille | Fichiers | Injection | Notes format |
|---------|----------|-----------|--------------|
| EQ | `eq Mono.json`, `eq stereo.json` | 16/16 | `cd:03`, 48 o |
| Modulation | `modulation mono/stereo/legacy.json` | 79/79 | `cd:03` |
| Delay | `delay mono/stereo/lecacy.json` | 67/68 | `cd:04` moderne, `cd:05` legacy ; hints 2 nibbles (`50`) |
| Reverb | `reverb mono/stereo/lecacy.json` | **38/38** | **`cd:05` partout** ; moderne 48 o ; legacy 44 o, hints **4 nibbles** (`ccf6`) |
| Pitch/Synth | `pitch_synth mono/stereo/legacy.json` | **37/38** | **`cd:05`** ; classiques 44 o (`ccb6`) ; doublons id catalogue (12-String) ; `Poly Bass Wham` non capturé |
| Filter | `filter mono/stereo/legacy.json` | **19/19** | **`cd:05`** ; hints 4 nibbles (`cc89`) + longs (`cd012c`) |
| Wah | `wah mono/stereo.json` | **22/22** | mono **`cd:05`**, stéréo **`cd:06`** ; terminaison hint `1aff` (ex. `cd011a`, pas `cd01`) |
| Amp (sans cab) | `amp guitar.json`, `amp bass.json` | **111/111** | clés **`guitar`** / **`bass`** ; `c219` + hints courts/longs ; Guitar `cd:06`, Bass `cd:06`/`cd:07` |
| Amp+Cab IR | `amp_cab guitar/bass.json` | **111/111** | **`8317c319`** + `<amp> 1a <cab>` dans **un** bulk ; cab IR `cd:03:xx` — doc [Amp_cab_fonctionnement_no_legacy.md](../../docs/Amp_cab_fonctionnement_no_legacy.md) |
| Amp+Cab Legacy | `amp_cab legacy guitar/bass.json` | **111/111** | même `c319` ; cab hybrid **2 nibbles** (`47:00`) ; 44/48 o — doc [Amp_cab_fonctionnement_legacy.md](../../docs/Amp_cab_fonctionnement_legacy.md) |
| Preamp | `preamp guitar/bass/mic.json` | **113/113** | `c219` ; **`cd:08`** ; clés `guitar`/`bass`/`mic` |
| Cab IR single | `cab single.json` | **46/46** | `c219` ; `subCategory: Single` ; `cd:09`/`cd:0a` |
| Cab IR dual | `cab dual.json` | **46/46** | `c319` ; `hint 1a cd02d6` (voie droite défaut Jazz Rivet) ; **≠** bulk single (`27:00`) |
| Cab Legacy single | `cab single legacy.json` | **41/41** | `c219` ; hints courts (`33`…) + 11× `cd02…` ; 44/48 o ; `cd:04` |
| Cab Legacy dual | `cab dual legacy.json` | **41/41** | `c319` ; suffixe **`1a 30:00`** — voie droite défaut **1x12 Lead 80** (`30`) ; même logique pan que IR dual |
| IR | `IR Single.json`, `IR Dual.json` | **3/3** | single `cc95`/`cc96` 44 o ; dual **`c219`** `cd02c4` 48 o (pas `c319`) |
| Volume/Pan | `volume_pan mono/stereo.json` | **7/7** | 48 o ; `cd:04` |
| Looper | `looper mono/stereo.json` | **7/7** | Shuffling `cd0268`/`cd0269` **≠** même bulk ; `VIC_LooperShuffling` = copie `cd0268` |
| Send/Return | `sendreturn mono/stereo.json` | **9/18** | 1–2 + paires stéréo 1/2 capturées ; **3/4** absents (pas de `chainHexHint` catalogue) |

Analyse scroll / fond `1d` :

```bash
python3 scripts/analyze_stomp_running_compare.py captures/usb-wireshark/stomp_running_start_hxedit.json
```

---

## Filtre Wireshark recommandé

Filtre validé terrain (EQ, Modulation, Delay, Reverb, juin 2026) — garde bulk OUT (`0x01`) + IN (`0x81`) et retire le flux interrupt `0x84` qui gonfle les exports (~24× plus léger qu’une capture sans filtre) :

```
usb.addr contains "1.4." && usb.endpoint_address != 0x84 && usb.data_len > 0
```

- **`usb.addr contains "1.4."`** : cible le Helix sur le bus USB courant (ajuster si le device change, ex. `1.3.`).
- **`endpoint_address != 0x84`** : exclut le keepalive interrupt (milliers de paquets vides).
- **`data_len > 0`** : paquets avec payload.

Variantes utiles :

| Objectif | Filtre |
|----------|--------|
| Tout le device Line 6 | `usb.idVendor == 0x0e41` |
| Bulk Helix seulement | `usb.idVendor == 0x0e41 && (usb.endpoint_address == 0x01 \|\| usb.endpoint_address == 0x81)` |
| Assign OUT seulement | `usb.capdata contains "80:10:ed:03"` (plus restrictif ; peut manquer des IN avec le nom modèle) |

**Workflow** : appliquer le filtre avant ou pendant la capture → dans Wireshark, n’exporter en JSON que les paquets **affichés** (filtrés) : *File → Export Packet Dissections → JSON*.

> **Le filtre Wireshark n’est en général pas la cause** des `chainHexHint` courts « manquants » dans `HX_ModelUsbAssign.json`. Les paquets OUT `23:00` + `c219` + `50:1a:ff` et les noms ASCII en IN passent bien le filtre ci-dessus. Si l’injection échoue, vérifier d’abord les critères d’extraction (section suivante), pas la capture.

---

## Deux familles de paquets à ne pas confondre

| | **Assign modèle** (picker HX Edit) | **Scroll molette** (pull slot) |
|---|-----------------------------------|--------------------------------|
| Déclencheur | Clic modèle dans HX Edit | Molette hardware → `IN 1f` → host envoie `OUT 1b` |
| Paquet host typique | `23`/`24`/`25:00` + `80:10:ed:03` | `1b:00` (36 o) sur lane ed — **pas** un bulk assign |
| Corps modèle | `83:66:cd:XX` + `c2:19` + id + `1a:ff` | Id module dans le **dump IN** (`IN 53` / variantes, motif `…19 <id> 1a…`) |
| Rôle dans l’app | `bulkHex` → envoi assign USB (`resolve_usb_assign_bulk`) | `chainHexHint` → nom/catégorie UI (`model_catalog::resolve_chain_hex_entry`) |
| Nom affiché | Souvent paquet **IN** suivant (ASCII, ex. `Simple Delay`) | Extrait du dump scroll, pas du bulk OUT |

**Piège fréquent (Delay)** : des paquets **36 o** (`1b:00`) contiennent `83:66:cd:04` **sans** `c2:19` — ce sont des requêtes lane scroll/pull, **pas** des `bulkHex` à copier dans l’assign. Les vrais bulks assign courts sont des OUT **44 o** (`23:00`).

**Amp+Cab (IR + Legacy)** : un seul bulk par modèle — marqueur **`83:17:c3:19`**, puis `<amp> 1a <cab>…`. Cab **IR** : `cd:03:29` ; cab **Legacy** : octet court `47:00`. L’entrée `amp+cab` / `amp+cab-legacy` réutilise le **`chainHexHint` de l’entrée `amp`** jumelle.

**Cab IR dual** : même idée qu’Amp+Cab (un bulk `c319`), mais la « deuxième voie » est le suffixe fixe **`cd:02:d6`** (Jazz Rivet par défaut) après `1a` — ce n’est **pas** le même paquet que le single `c219` (HX Edit = deux singles logiques, USB = un assign dual distinct ; la voie droite reste la dernière sélection).

**Cab Legacy dual** : même schéma avec suffixe **`30:00`** = **1x12 Lead 80** (Bogner Shiva CL80) en voie droite par défaut — parallèle du **2x12 Jazz Rivet** (`cd02d6`) en IR ; hints longs `cd02…` → terminaison **`1a 30:00:00:00`** (48 o).

---

## Formats `bulkHex` assign (campagnes HX Edit)

Critères d’extraction alignés sur `scripts/inject_bulk_from_captures.py` :

| Champ | Valeurs observées | Notes |
|-------|-------------------|-------|
| Préfixe OUT | `23:00`, `24:00`, `25:00`, `27:00` | 44–48 o ; **`27:00`** = Amp+Cab IR (48 o) ; ne pas ignorer **`23`** (delays classiques) |
| Opcode | `80:10:ed:03` | Assign éditeur |
| Corps lane | `83:66:cd:03` … `cd:08` | … ; **Preamp** → `cd:08` ; Amp Guitar `cd:06` ; Amp Bass `cd:07` |
| Marqueur module | `c2:19` puis id | |
| Fin module | `1a:ff` | |

### `chainHexHint` court vs long

| Type | Exemple dans le bulk | `chainHexHint` |
|------|----------------------|----------------|
| **Court** (2 nibbles) | `…c2:19:50:1a:ff…` | `50` |
| **Moyen** (4 nibbles) | `…c2:19:cc:f6:1a:ff…` | `ccf6` |
| **Long** (`cd01…`) | `…c2:19:cd:01:1d:1a:ff…` | `cd011d` |
| **Long** (delay/reverb moderne) | `…c2:19:cd:02:63:1a:ff…` | `cd0263` |

Règle d’extraction (`chain_hex_hint_from_c219_module`) : si l’id commence par `cd`, prendre jusqu’au suffixe **`1aff`** (pas le premier `1a` — évite `cd011a` → `cd01`) ; sinon si `XX1a` en position 2–3, prendre **2 nibbles** ; sinon tout jusqu’à `1a` (ex. `501aff` → `50`, `ccf61aff` → `ccf6`).

Familles connues à hints mixtes : **Delay** (2 et 6 nibbles), **Reverb legacy** (4 nibbles `cc…`), **Distortion** (ex. Minotaur `70`).

---

## Lien avec le scroll molette

Le scroll et l’assign partagent le **même vocabulaire d’id module** (`chainHexHint`), mais pas le même paquet USB.

### Ce que le scroll utilise

1. **`IN 1d` / `IN 1f`** — fond dialogue lane `f0:03:02:10` (ACK dans `firmware_scroll_ack.rs`).
2. **`OUT 1b`** — pull déclenché sur `IN 1f` (`scroll_model_pull.rs`).
3. **`IN 53`** (et têtes voisines) — dump contenant `19 <module_id> 1a` ; le `chainHex` y est **complet** (mode grab-53, pas besoin du flux `272`).

L’app résout le nom affiché via `HX_ModelUsbAssign.json` → index `chainHexHint` (`model_catalog.rs`). Ex. dump contenant `50` → « Simple Delay » **si** une entrée assign a `chainHexHint: "50"`.

### Impact du « problème chainHex courts » sur le scroll

| Symptôme scroll | Cause liée aux captures / assign | Piste |
|-----------------|----------------------------------|-------|
| Slot vide / pas de nom après scroll | Dump non reconnu ou pull non finalisé | Captures `stomp_running_*`, `HX_SCROLL_CHAINHEX=1`, voir `docs/todo-scroll-hw.md` |
| Nom modèle inconnu malgré dump OK | `chainHexHint` absent ou **mal extrait** de la capture assign (ex. `501aff` au lieu de `50`) | Ré-injecter avec script à jour ; vérifier l’index assign |
| Assign picker OK mais scroll « décalé » | Problème lane / timing pull (`cd_lane` 03↔04), pas `bulkHex` | Comparer OUT `1b` HX Edit vs HXLinux |
| Confusion analyse capture | Traiter un `1b:00` scroll comme un bulk assign | Séparer captures campagne assign vs captures scroll |

**`bulkHex` vide** n’empêche pas à lui seul la résolution scroll (l’index utilise `chainHexHint` + métadonnées). En revanche :

- sans entrée assign correcte pour un id court (`50`, `4f`, …), le scroll **ne peut pas** afficher le bon modèle ;
- une mauvaise lecture des captures (ignorer `23:00`, exiger `cd:03` seul, hint trop long) laisse des trous dans l’assign et donne l’impression que « le scroll ne connaît pas » ces modèles ;
- la **lane `cd`** (`03` / `04` / `05`) dans les bulks assign indique sur quel sous-fil la famille vit — le pull scroll avance le `cd_lane` (octet 27 des OUT `1b`/`19`) ; mélanger les familles lors de l’analyse peut masquer des désync lane.

Après chaque campagne bulkHex, vérifier que les hints courts sont bien présents :

```bash
# Exemple : delays avec hints 2 caractères
rg '"chainHexHint": "(50|4f|4d|4e)"' src-tauri/resources/HX_ModelUsbAssign.json -A1
```

Test Rust utile : `model_catalog::resolves_short_hex_minotaur_stereo` (`70` → Minotaur stéréo).

---

## Repérer un `bulkHex` dans une capture

Checklist paquet **OUT host** (`usb.src == host`, ep `0x01`) :

1. `usb.capdata` commence par `23:00`, `24:00` ou `25:00:00:18:80:10:ed:03`
2. contient `83:66:cd` (03, 04 ou 05 selon la famille)
3. contient `c2:19` puis l’id module jusqu’à `1a:ff`

Exemples :

```
# Delay court (44 o) — hint 50
23:00:…:80:10:ed:03:…:83:66:cd:04:…:c2:19:50:1a:ff:…

# EQ / modulation (48 o) — hint cc84
25:00:…:80:10:ed:03:…:83:66:cd:03:…:c2:19:cc:84:1a:ff:…

# Delay legacy (48 o) — hint cd0199, corps cd:05
25:00:…:80:10:ed:03:…:83:66:cd:05:…:c2:19:cd:01:99:1a:ff:…

# Reverb moderne (48 o) — hint cd0263, corps cd:05
25:00:…:80:10:ed:03:…:83:66:cd:05:…:c2:19:cd:02:63:1a:ff:…

# Reverb legacy (44 o) — hint ccf6, corps cd:05
23:00:…:80:10:ed:03:…:83:66:cd:05:…:c2:19:cc:f6:1a:ff:…
```

Le **nom affiché** du modèle (ASCII, ex. `Simple EQ`) arrive souvent dans le paquet **IN** suivant (`src` device, ep `0x81`), pas dans le bulk OUT.

Repérer un **id scroll** dans une capture session :

1. Filtrer `usb.endpoint_address == 0x81`
2. Chercher têtes `53`, `54`, `4c`, `4e`, `6c`… ou motif `19` + `1a` dans `usb.capdata`
3. Extraire l’id avant `1a` — doit correspondre à un `chainHexHint` dans l’assign

---

## Dépannage rapide

| Problème | Vérifier |
|----------|----------|
| `bulkHex` non injecté alors que la capture semble complète | Préfixe `23` ? corps `cd:04`/`cd:05` ? hint court 2 nibbles ? |
| Nom en IN mais pas de bulk OUT | Normal pour certains modèles — rescroller / ré-assigner dans HX Edit |
| `IN 1d = 0` en capture scroll | Précondition mode édition Stomp (voir `docs/todo-scroll-1d-step-by-step.md`) |
| Scroll sans nom sur delays classiques | Entrées assign `chainHexHint` `50`, `4f`, … + ré-injection Delay |
| Reverb legacy non injecté | Hints **4 nibbles** `ccf6` — ne pas tronquer à 2 ; corps **`cd:05`** sur moderne aussi |
| Export JSON énorme | Filtre `endpoint != 0x84` appliqué **avant** export |

Références code : `scroll_model_pull.rs`, `model_catalog.rs`, `scripts/inject_bulk_from_captures.py`, `docs/todo-scroll-hw.md`.
