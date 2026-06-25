# HXLinux — Écriture de paramètres et changement de cab : les 4 familles

> **English:** [cab_family_live_write.en.md](cab_family_live_write.en.md)

> **Portée.** Ce document décrit le protocole d'écriture live (`write_live_param`) et le
> changement de cab pour les **quatre familles de cab en slot Cab seul / Cab dual** du HX
> Stomp XL : **single modern**, **dual modern**, **single legacy**, **dual legacy**.
> Tout est issu de captures Wireshark/usbmon (méthode *capture-first*, jamais de
> spéculation). Chaque comportement est gardé derrière un **flag env-var** (défaut ON ;
> `=0` restaure l'ancien comportement / témoin).

> **Hors scope — Amp+Cab legacy hybrid.** Les couples **Amp+Cab** (`assignVariant:
> amp+cab-legacy`) suivent un autre chemin : focus cab **`1b`**, live write **PP `0x08`**,
> sélecteurs **`0x25+`** / tables compact, pas le builder IR `23`/`27` décrit ici. Voir
> [Amp_cab_fonctionnement_legacy.md](Amp_cab_fonctionnement_legacy.md) /
> [Amp_cab_operation_legacy.md](Amp_cab_operation_legacy.md). Les **Amp+Cab IR** (cab modern
> dans un couple) relèvent de [Amp_cab_fonctionnement_no_legacy.md](Amp_cab_fonctionnement_no_legacy.md).

### Carte du code (Rust + UI)

| Zone | Fichiers |
|------|----------|
| Routage live write | `src-tauri/src/helix/live_write.rs`, `src-tauri/src/lib.rs` (`write_live_param`) |
| Cab single / hints legacy | `src-tauri/src/helix/amp_cab_live_write.rs` |
| Cab dual IR + legacy route | `src-tauri/src/helix/cab_dual_live_write.rs` |
| Replace cab2 dual legacy | `src-tauri/src/helix/cab_dual/legacy/wire.rs`, `cab_dual_cab2_replace.rs` |
| Handshake legacy (évité) | `src-tauri/src/helix/legacy_cab_param_commit.rs` |
| UI params + picker cab | `src/models.ts` (`appendModelsParamRows`, `renderModelsParamsDualTabs`, `applyCabDualCabFromPickerListClick`) |

### Index des captures (référence rapide)

| Famille | Captures principales (`captures/usb-wireshark/Save/`) |
|---------|--------------------------------------------------------|
| Single modern | `cab single.json` |
| Dual modern | `IR Dual.json`, `add_dual_cab_modif_param_cab2.json`, `add_dual_cab_soup_pro_2x12bluebell_HXEdit.json` |
| Single legacy | `cab single legacy.json` |
| Dual legacy | `cab dual legacy.json`, `add_dual_legacy_change_cab2.json`, `add_dual_legacy_change_cab2_&_dual.json` |

Voir aussi l'inventaire : [captures/usb-wireshark/README.md](../captures/usb-wireshark/README.md).

---

## 0. Vocabulaire et repères de trame

Une trame d'écriture de paramètre (`23` discret/bool, `27` float) se termine par un bloc
modèle de la forme :

```
83 66 cd <KK> <tag> 64 <op> 65 <85|82> 62 <bus> 1d <VT> 1a <CAB> 1c <pSel> 77 <val> 00
            └─KK─┘                                     │      │       │
            03 = single / cab1                         │      │       └─ pSel : sélecteur de paramètre
            04 = dual / cab2                           │      └─ CAB : 00 = cab1, 01 = cab2 (dual)
                                                       └─ VT : marqueur type de valeur
                                                              c2 = discret legacy
                                                              c3 = float (et discret modern par replay statique)
```

Trois octets concentrent toute la logique de différenciation entre familles :

| Octet | Rôle | Valeurs |
|-------|------|---------|
| `cd <KK>` | bloc modèle | `cd 03` = single / cab1 · `cd 04` = dual / cab2 |
| `<VT>` (juste après `1d`) | type de valeur sur le fil | `c2` = discret legacy · `c3` = float / discret modern |
| `<CAB>` (juste après `1a`) | index du cab dans un dual | `00` = cab1 · `01` = cab2 |

**Marqueur d'assignation `c2:19` vs marqueur legacy.** Attention au piège : le `c2:19`
présent dans le **bulk d'assignation** est un marqueur « **single cab** » — il est porté
*aussi* par les MicIr modern (`c2 19 cd03xx`). Le vrai discriminant *legacy* n'est pas ce
`c2:19`, mais la **forme du champ cab** : un cab legacy a un **hint d'1 octet**, un cab
modern a un **bloc `cd 03 xx` de 3 octets**.

---

## 1. Cab single modern

> **État : RÉSOLU.** Paramètres + `@mic` fonctionnels.

**Code :** `live_write.rs` (`force_discrete_c2_marker`, `standalone_legacy_assign_is_one_byte_hint`),
`amp_cab_live_write.rs` (blocs IR single).

**Captures :** `captures/usb-wireshark/Save/cab single.json` (assign + params MicIr, bloc `cd 03 xx`).

### Particularité

Le single modern écrit ses discrets avec le marqueur `c3` (issu du **replay statique** du
gabarit), *pas* `c2`. C'est l'inverse exact du single legacy.

### Le problème rencontré

Pendant la mise au point du single legacy, on a forcé un marqueur `c2` sur les discrets via
`force_discrete_c2_marker()`. Ce forçage, appliqué sans condition, **cassait le single
modern** : son `@mic` discret sortait en `c2` au lieu du `c3` attendu, et le device
l'ignorait.

Le test décisif a été `HX_CAB_DISCRETE_C2=0` (désactivation du forçage) :

> Avec le forçage OFF, le single **modern** remarchait, le single **legacy** retombait en
> panne. Preuve directe que le `c2` est **juste pour le legacy** et **faux pour le modern**.

### La solution

Conditionner la pose du `c2` à la signature legacy, c'est-à-dire au **hint cab d'1 octet**.
Helper `standalone_legacy_assign_is_one_byte_hint()` : le drapeau `c2` n'est posé **que** si
le champ cab fait 1 octet (legacy). Le modern (bloc `cd 03 xx`, 3 octets) garde son `c3`.

*Le single modern n'a donc aucun code spécifique : il est « ce qui reste » quand on ne pose
pas le `c2`. La leçon : un correctif legacy ne doit jamais s'appliquer inconditionnellement.*

---

## 2. Dual cab single modern (dual modern)

> **État : RÉSOLU** (sessions antérieures). Tous params + `@mic`, cab1 et cab2.

**Code :** `cab_dual_live_write.rs` (`resolve_cab_dual_live_write_route`,
`build_cab_dual_minimal_param_packets_from_state`, armement `ed:08`),
`cab_dual_cab2_replace.rs` (replace cab2 IR).

**Captures :** `IR Dual.json`, `add_dual_cab_modif_param_cab2.json`,
`add_dual_cab_soup_pro_2x12bluebell_HXEdit.json`, `cab dual change right.json` (focus / replace IR).

### Particularités

Le dual modern introduit deux mécanismes absents du single :

1. **Distinction cab1 / cab2 par le bloc modèle** : cab1 = `cd 03`, cab2 = `cd 04`.
2. **Index du cab dans l'octet après `1a`** : `00` pour cab1, `01` pour cab2.

### Les points qui ont dû être résolus

| Point | Exigence device |
|-------|-----------------|
| Modèles cab2 | contexte dual `cd031c` / `c3 19` (mélanger avec le single échoue) |
| Armement | **`ed:08` obligatoire avant chaque `23`/`27`** (sinon write silencieusement ignoré) |
| Bloc modèle | `cd:03` (cab1) / `cd:04` (cab2), pas `cd:04` partout |
| Index cab | dans l'octet après `1a` (`00`/`01`) |
| Index param | **local (0-based) par cab**, pas un index global continu |

> **`ed:08` est la règle d'or du dual.** Le device n'émet aucune erreur si un `23`/`27`
> arrive sans armement préalable — il l'ignore en silence. Tout write dual doit être précédé
> de son `ed:08`.

*Le dual modern est la « forme de référence » : les deux autres familles (legacy) sont
ramenées à son builder une fois leurs spécificités gérées.*

---

## 3. Cab single legacy

> **État : RÉSOLU.** Paramètres + `@mic` fonctionnels.

**Code :** `live_write.rs`, `amp_cab_live_write.rs` (burst `23`/`27`/`57` legacy),
`legacy_cab_param_commit.rs` (handshake async — **désactivé** si `HX_LEGACY_SINGLE_IR_PARAM`),
`models.ts` (`wireLocal` via `cabAssignVariant === "legacy"`).

**Captures :** `captures/usb-wireshark/Save/cab single legacy.json` (scroll, `@mic`, discrets `c2`, floats `57`).

### Particularité

Un single legacy ne s'écrit **pas** avec un burst minimal : il utilise la **trame IR
standard** `23` (discret) / `27` (float), à `cd:03`, mais avec deux singularités — le
marqueur discret `c2`, et un sélecteur de paramètre **wire-local**.

Forme discrète observée :

```
83 66 cd 03 <tag> 64 1e 65 85 62 <bus> 1d c2 1a 00 1c <pSel> 77 <payload>
                                          └c2┘        └pSel wire-local┘
```

### Les trois bugs empilés (et leurs correctifs)

**Bug 1 — Mauvais aiguillage (handshake async parasite).**
Un test `standalone_legacy_assign_uses_cd03ff` se déclenchait à tort et routait le single
legacy vers un chemin de handshake asynchrone incorrect.
**Correctif :** flag `HX_LEGACY_SINGLE_IR_PARAM` prioritaire — tous les single legacy passent
par le chemin IR standard (`route_override = None`).

**Bug 2 — Marqueur `c3` figé au lieu de `c2`.**
`assemble_23_bool_write` codait en dur `c3` pour les discrets, alors que le device legacy
attend `c2`.
**Correctif :** `force_discrete_c2_marker()` réécrit l'octet `start+12` du bloc `83 66 cd…`
en `c2`, en **post-finalisation**. Flag `HX_CAB_DISCRETE_C2`.
**Condition essentielle** (voir §1) : le `c2` n'est posé **que** si le hint cab fait 1 octet
(`standalone_legacy_assign_is_one_byte_hint()`), pour ne pas casser le single modern.

**Bug 3 — Sélecteur de paramètre global au lieu de wire-local.**
HXLinux envoyait un `pSel` = index global « à plat » (mic compté dans le total). Le device
numérote `pSel` **0-based, séparément par groupe de type d'onde** (discret vs float).
**Symptôme :** le 1er paramètre s'injectait dans le 2ᵉ, et ainsi de suite — un décalage `+1`.
**Correctif :** sélecteur **wire-local** dans `liveWriteParamIndexForRow` (param `wireLocal`),
flag localStorage `models_wire_local_param_selector` (`=0` désactive le wire-local).
Le wire-local ne s'applique **qu'au single legacy** : `renderModelsParamsPane` passe
`cabAssignVariant === "legacy"` comme 13ᵉ argument à `appendModelsParamRows`.

### Tableau de synthèse single legacy

| Symptôme | Cause | Correctif | Flag |
|----------|-------|-----------|------|
| Handshake async parasite | `..._uses_cd03ff` faux positif | chemin IR standard prioritaire | `HX_LEGACY_SINGLE_IR_PARAM` |
| `@mic` muet (sort en `c3`) | `assemble_23_bool_write` code `c3` | `force_discrete_c2_marker()` post-finalize | `HX_CAB_DISCRETE_C2` |
| 1er param → 2ᵉ (décalage +1) | `pSel` global au lieu de local | sélecteur wire-local | `models_wire_local_param_selector` |

*Le single legacy a servi de laboratoire : c'est là qu'on a appris le marqueur `c2` discret
et le principe wire-local, deux acquis réutilisés ensuite pour le dual legacy.*

---

## 4. Cab dual legacy

> **État : RÉSOLU.** Changement cab2 + tous params + `@mic` (cab1 et cab2). Affichage UI
> du changement de cab2 corrigé.

**Code :** `cab_dual/legacy/wire.rs` (`build_legacy_cab2_replace_bulk`,
`CAB_DUAL_LEGACY_CAB2_REPLACE_23_TEMPLATE`), `cab_dual_live_write.rs` (`dual_legacy_standard_param_enabled`,
`discrete_wants_c2`), `live_write.rs`, `models.ts` (`applyCabDualCabFromPickerListClick`,
`renderModelsParamsDualTabs`).

**Captures :** `cab dual legacy.json` (ADD, params, floats `71`),
`add_dual_legacy_change_cab2.json` (replace cab2 compact `23` 44 o),
`add_dual_legacy_change_cab2_&_dual.json` (session complète change + params).

C'est la famille la plus complexe : elle cumule les contraintes du dual modern (cab1/cab2,
`ed:08`, index cab) **et** du legacy (`c2` discret, wire-local), plus une forme de trame
propre pour le changement de cab2.

### 4.1 Vérité-fil (captures)

**ADD** (`2d`, 56 octets) — marqueur dual `c3 19`, deux hints 1 octet :

```
… 83 66 cd 04 <tag> 64 27 65 82 62 <bus> 63 82 13 06 14 83 18 83 17 c3 19 <cab1=33> 1a <cab2=30> 09 10 0a c3 …
                                                                      └c3 19┘ (marqueur dual legacy)
```

**FOCUS cab2** (`1d`, 40 octets) :

```
… 83 66 cd 04 <tag> 64 4e 65 82 62 <bus> 1a 01 00 00 00
```

**CHANGEMENT CAB2** (`23`, 44 octets — trame **compacte**, *pas* un bulk) :

```
83 66 cd 04 <tag> 64 28 65 82 62 <bus> 64 83 17 c3 19 <cab1> 1a <cab2new> 00
                                                 └c3 19┘     └1a┘└cab2 nouveau┘
```

**PARAM** (`23`, 44 octets — discret) — **identique au dual modern**, avec `c2` discret,
index cab après `1a`, `pSel` wire-local :

```
83 66 cd 04 <tag> 64 1e 65 85 62 <bus> 1d c2 1a <00|01> 1c <pSel> 77 <val> 00
                                          └c2┘   └cab┘      └wire-local┘
```

> La forme float `27` du dual legacy n'a **pas** été observée en capture ; elle est inférée
> par symétrie avec le discret. À valider si un cas float pose problème.

### 4.2 Changement de cab2

Branche 1 octet de `build_legacy_cab2_replace_bulk` (wire.rs) remplacée par un template
*capture-grounded* `CAB_DUAL_LEGACY_CAB2_REPLACE_23_TEMPLATE` (44 o) :

- cab1 à l'index 40, cab2 à l'index 42, bus à l'index 34 (patché par le wrapper
  `build_slot_model_probe_packets`).
- Flag `HX_DUAL_LEGACY_CAB2_23_TEMPLATE`. **→ Validé hardware.**

### 4.3 Paramètres : routage via le builder dual modern

Plutôt qu'un burst hybride spécifique, le dual legacy emprunte le builder du dual modern
(`build_cab_dual_minimal_param_packets_from_state`).

- `write_live_param` : garde `route_is_dual_legacy_cab(&route) && !dual_legacy_standard_param_enabled()`,
  flag `HX_DUAL_LEGACY_STD_PARAM`.
- `resolve_cab_dual_live_write_route` : en `standard_legacy`, utilise le bloc IR modern
  (`build_cab_dual_cab1/cab2_ir_param_model_block`, `cd 03`), `param_selector = param_index`
  local, court-circuite le cache echo.

> **État actuel accepté par le HW (`cd 03` vs `cd 04`).** La capture §4.1 montre les params
> discrets dual legacy en **`cd 04`**. Le chemin `standard_legacy` (flag ON) émet parfois
> **`cd 03`** sur le bloc modèle — notamment le `@mic` (§4.5). Le device l'accepte aujourd'hui ;
> ne pas « corriger » sans capture d'échec. Si un write est ignoré, tester un override
> `cd 04` sur le `model_block` avant tout autre changement.

### 4.4 Sélecteur wire-local (décalage +1)

Même bug que le single legacy : le `pSel` partait en index global → le 1er param s'injectait
dans le 2ᵉ.

**Correctif :** `renderModelsParamsDualTabs` passe un 13ᵉ argument
`cabDualLegacyWireLocal = dualSlotKind === "cab_dual" && cabDualAssignVariant === "dual-legacy"`
à `appendModelsParamRows`. Le dual **modern** garde `"dual"` → wire-local OFF → index global
inchangé.

```typescript
const cabDualLegacyWireLocal =
  dualSlotKind === "cab_dual" &&
  (cabDualAssignVariant ?? "").trim().toLowerCase() === "dual-legacy";
```

### 4.5 Le `@mic` : drapeau d'état fragile → contexte porté par la route

C'est le dernier — et le plus instructif — des bugs. Symptôme : *« tout marche sauf le
micro »*, sur cab1 **et** cab2.

**Diagnostic au byte près.** Le `@mic` sortait bien en `opcode=23`, `pSel=00`, `1a 00`/`1a 01`
corrects — **mais avec `c3` au lieu de `c2`** :

```
cab1: … 83 66 cd 03 19 … 1d c3 1a 00 1c 00 77 08 00
cab2: … 83 66 cd 03 1a … 1d c3 1a 01 1c 00 77 0b 00
                            └c3┘ ← devrait être c2
```

**Cause racine.** Le `c2` du builder dual dépendait du drapeau d'état partagé
`force_discrete_c2_for_legacy_single`. Or ce drapeau est **consommé** (remis à `false`) par
**tout** write — y compris un write **float** qui précède. Quand on bouge un slider float
puis le micro, le float consomme le drapeau → le `@mic` discret qui suit ne le voit plus →
sort en `c3`. Les autres params (floats) marchent quand même car ils n'ont pas besoin du
`c2` : seul le `@mic` discret en pâtit. D'où *« tout marche sauf le micro »*.

**Solution.** Ne plus dépendre d'un état partagé fragile : **porter le contexte legacy dans
la route**.

1. Champ `discrete_wants_c2: bool` ajouté à `LiveWriteRouteOverride` (init `false` partout).
2. `resolve_cab_dual_live_write_route` : `discrete_wants_c2: standard_legacy` dans le retour
   non-echo, `false` dans la branche echo-cache.
3. `build_cab_dual_minimal_param_packets_from_state` :
   ```rust
   let force_c2 = route.discrete_wants_c2 || state.force_discrete_c2_for_legacy_single;
   ```

Ainsi le `c2` est décidé par la route du `@mic` lui-même, à chaque appel, sans dépendre de
l'ordre des writes. **→ Validé hardware** (cab1 + cab2).

> **Note `cd 03` vs `cd 04`.** Le `frame_b` du `@mic` sort en `cd 03` (et non `cd 04` comme
> la capture du dual legacy). Le device l'accepte — non corrigé car fonctionnel. Si un futur
> cas échoue, forcer `cd 04` via override du model_block.

### 4.6 Affichage UI au changement de cab2

> **Principe directeur (à ne jamais régresser) : un changement de cab ne lit RIEN du
> hardware.** Les paramètres du nouveau cab sont les **défauts du `.models`** (JSON) du cab
> choisi. Le fil (`moduleHex`) ne doit jamais piloter cette résolution.

**Symptôme.** Au changement de cab2, l'UI gardait les paramètres de l'**ancien** cab2 (le
hardware, lui, changeait bien).

**Cause.** Depuis que `cabDualWireParts` parse aussi le legacy, le rebuild repartait du
**fil optimiste** (`optimisticSlot.moduleHex`), qui porte encore l'ancien cab2 tant qu'il
n'y a pas de re-dump. La chaîne `resolveCabDualTabPanes` → `buildDualTabPanesFromCabDualWire`
reconstruisait alors le pane2 sur l'ancien cab2.

**Solution (deux volets).**

1. Dériver le hex du cab **réellement choisi** (`cab2PickerCatalogId`) via
   `moduleHexForUsbVariant(...)`, au lieu du fil optimiste, dans
   `applyCabDualCabFromPickerListClick` (branche `tab === 1`).
2. Forcer un **rebuild complet** du panneau après le `finally`, en invalidant le chemin
   « patch valeurs seules » :
   ```typescript
   if (tab === 1 && selectedParamsKemplineSlotIndex === ki &&
       lastProbePickerAssignContext?.ki === ki) {
     selectedParamsInPlaceUpdater = null;   // invalide canPatchValuesOnly
     selectedParamsInPlaceSlotKey = null;
     selectedParamsValuesSig = null;
     const slotNow = lastHwSyncNormalizedSlots?.[ki] ?? optimisticSlot;
     await loadAndShowModelsParamsForSlot(slotNow, ki);
   }
   ```
   Les trois `= null` sont essentiels : sans eux, `loadAndShowModelsParamsForSlot` repart
   dans `canPatchValuesOnly` qui ne met à jour que le cab1.

*Le bon point d'ancrage du rebuild est l'**ID du cab2 fraîchement choisi**
(`lastProbePickerAssignContext.cabDualCab2ModelId` → `probeCab2Hint`), pas le fil — fidèle au
principe « pas de lecture HW pour un changement de cab ».*

---

## 5. Tableau récapitulatif inter-familles

| | Single modern | Dual modern | Single legacy | Dual legacy |
|---|---|---|---|---|
| Bloc modèle | `cd 03` | `cd 03`/`cd 04` | `cd 03` | `cd 04` (params : `cd 03` toléré) |
| Marqueur discret | `c3` (replay) | `c3` | **`c2`** | **`c2`** |
| Marqueur float | `c3` | `c3` | `c3` | `c3` |
| Index cab (`1a …`) | — | `00`/`01` | `00` | `00`/`01` |
| Sélecteur `pSel` | global | global | **wire-local** | **wire-local** |
| `ed:08` arming | — | **requis** | — | **requis** |
| Hint cab | bloc `cd 03 xx` (3 o) | bloc (3 o) | **1 octet** | **1 octet** ×2 |
| Builder paramètres | frames single | dual minimal | frames single (IR) | dual minimal (route `discrete_wants_c2`) |

---

## 6. Référence des flags

| Flag | Défaut | Effet (`=0` = témoin / ancien comportement) |
|------|--------|---------------------------------------------|
| `HX_LEGACY_SINGLE_IR_PARAM` | ON | Single legacy via chemin IR standard (pas de handshake async parasite) |
| `HX_CAB_DISCRETE_C2` | ON | Force `c2` sur les discrets legacy (`force_discrete_c2_marker`) |
| `models_wire_local_param_selector` (localStorage) | ON | Sélecteur `pSel` wire-local (discret/float 0-based séparés) |
| `HX_DUAL_LEGACY_CAB2_23_TEMPLATE` | ON | Changement cab2 dual legacy via template `23` 44 o capture-grounded |
| `HX_DUAL_LEGACY_STD_PARAM` | ON | Paramètres dual legacy routés via le builder dual modern |

---

## 7. Principes clés (transversaux)

- **Sélecteurs `pSel` wire-type-local, 0-based** : le device numérote séparément les
  discrets et les floats. Un index global « à plat » provoque un décalage silencieux, sans
  erreur device.
- **`c2` = discret legacy · `c3` = float** (et discret modern via replay statique). Le
  marqueur de type de valeur est l'octet **juste après `1d`**, avant `1a`.
- **`cd 04` + index cab après `1a`** pour le dual ; **`cd 03`** pour le single. (Le `@mic`
  dual legacy fonctionne aussi en `cd 03`.)
- **Le `c2:19` du bulk d'assign = « single cab »**, *pas* legacy. Le legacy se reconnaît au
  **hint d'1 octet** vs le bloc `cd 03 xx` de 3 octets du modern.
- **`ed:08` mandatoire** avant chaque `23`/`27` en dual (sinon write ignoré en silence).
- **Leçon `@mic`** : un drapeau d'état **partagé** entre writes
  (`force_discrete_c2_for_legacy_single`) est fragile — un write précédent le consomme.
  Préférer porter le contexte dans la **route** (`discrete_wants_c2`).
- **Changement de cab = aucune lecture HW** : les params viennent des défauts `.models` du
  cab choisi. Ne jamais piloter la résolution par le fil optimiste.
- **Méthode capture-first** : ne jamais coder à l'aveugle. Exiger le `[LiveWrite][sent]` ou
  le `[SlotModelProbe]` pour trancher au byte près.