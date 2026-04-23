# HXLinux — description pour reprendre une session

Ce fichier sert de **mémo locale** quand l’historique de chat ou le contexte IDE est perdu après un redémarrage. Il complète le `README.md` (objectifs produit et commandes de base).

**Dernière mise à jour significative** : **avril 2026** — panneau paramètres : grille **nom | min | cellule (valeur + contrôle) | max** ; **en-tête en grille 2×2** (catégorie en ambre à gauche L1, **`presetMeta.emulationName`** en blanc à droite L1, bloc infos sous-titre à gauche L2, **icône catalogue** à droite L2 + **aperçu au survol** taille quasi native) ; **toggles Off/On** pour paramètres **bool** (`valueType === 2` ou `displayType` **`off_on`**) sans slider ; **`pickEmulationName`** dans **`hxModelCatalogMeta.ts`** ; format **`.models`** étendu côté TS (**`min` / `max`** `number | boolean`, variantes **`displayType_stereo`**, **`min_stereo`**, **`max_stereo`**, **`default_stereo`** appliquées en signal **stéréo** pour l’affichage des bornes / défauts / formatage). **Jointure stricte** **`chainHex` → `id` catalogue → `.models.symbolicID`** (pas de fallback par nom pour la définition du modèle) ; **`hxModelCatalogMeta.ts`** : index séparés **`byHex`**, **`byId`**, **`byCategoryAndName`** pour éviter les collisions (ex. deux **Ping Pong** Delay avec **`chainHex`** différents). **Règles d’affichage des lignes** : liste des paramètres **filtrée par** les clés présentes dans **`HX_ModelCatalog.json`** (`params` du modèle catalogue), **ordre des lignes** = ordre du **`params[]`** du **`.models`** ; alignement des **valeurs** de chaîne sur l’**ordre catalogue** puis projection par **`symbolicID`** ; en **mono**, choix automatique avec/sans entrées **`stereo-only`** dans la séquence source si la longueur de chaîne ne colle pas (évite le décalage type **Bucket Brigade**) ; lignes **`stereo-only`** masquées en mono. Formatage « chaîne » via **`HelixControls.json`** ; table Rust **`HX_CATALOG_MODULE_BY_HEX`** / scripts **`scripts/`** ; **`chainHex` vides** à compléter à la main. **Amp+Cab / `module_hex` grille** : voir section dédiée ci-dessous (inférence **`ampHex` + `1a` + `cabHex`**, faux positifs **`c219` / `c319`**). **Prochaine session (UI matrice)** : **`Icons_line.png`** manquant ou incorrect sur **Path 2** (rangée L3) — voir section matrice.

**21 avril 2026** — alignement **valeurs chaîne ↔ lignes du panneau** pour les modèles dont les **`.models`** portent **`assign`** (ordre DSP ≠ ordre du tableau `params[]`, ex. ampli / préampli **Ch Vol** vs **Master**) : fonction **`alignChainValuesToModelParamOrder`** dans **`src/models.ts`** avant le rendu des lignes. Côté Rust, **`parse_assignable_segment_param_blocks`** accepte désormais un segment assignable en **`0x06` ou `0x08`** (comme la validation de la fenêtre Kempline à 20 segments). Commit local de synthèse : **`de27037`** (*Preset chaîne, catalogue HX et panneau paramètres*).

**22 avril 2026** — **Amp+Cab et `chainHex` pour la grille / le panneau** (`lib.rs`, tests unitaires dans le même fichier) : l’UI affiche *Jointure ID impossible : chainHex manquant pour ce slot* quand **`getCatalogModelIdForHex(slot.moduleHex)`** reçoit une chaîne vide — **`module_hex`** vient de **`extract_first_module_from_assignable_chunk`** (grille 16 via **`try_parse_preset_kempline_grid`**) ou du parseur de secours **`parse_preset_slots_internal`**. Travaux réalisés : (1) **Marqueur Amp+Cab** : **`is_amp_cab_assignable_chunk`** exige un segment **`0x06` ou `0x08`** contenant la fenêtre **`85 18 83 17 c3 19`** (aligné USB / Kempline ; les captures utilisent souvent **`0x08`**). (2) **Faux positif `c319`** : l’octet **`0x19`** final du motif **`83 17 c3 19`** n’est **pas** un début d’ID **`19…1a`** — on l’ignore **uniquement** dans ce contexte (ne **pas** élargir à tout **`c3`/`c2` + `19`**, car **`c3`/`c2`** encodent aussi des booléens dans **`read_params_hex`**). (3) **Faux positif `c219`** : en binaire, l’opcode **`c219`** est **`0xc2` + `0x19`** ; le scan des IDs **`19…1a`** peut donc croire voir un ID dont le préfixe hex coïncide avec le **type** du premier bloc **`c219`** (ex. seul **`cd0217`** au lieu du **`chainHex`** combiné catalogue **`cd02171acd0228`**). **`augmented_module_ids_for_assignable_chunk`** : si le segment est Amp+Cab, **`parse_assignable_segment_param_blocks`** rapporte **au moins deux** blocs **`c219`**, et l’extraction ne donne **qu’un** ID **identique** au type ampli inféré depuis les **`c219`**, on remplace par la **paire** inférée. (4) **Plusieurs `c219`** (ampli + bloc interne + cab, etc.) : **`infer_amp_cab_hex_pair_from_segment_hex_body`** parcourt **tous** les types d’argument **`c219`** dans le corps hex du segment ; paire = premier type classé **AmpLike** puis premier **CabLike** après dans le catalogue (**`HX_CATALOG_MODULE_BY_HEX` / `catalog_slot_kind_for_chain_hex`**), sinon repli **premier / dernier** préfixe 6 hex. **`inferred_amp_cab_hex_keys`** : si le parse à blocs échoue ou est incomplet, repli sur cette inférence **sans** compter sur **`blocks.len() == 2`**. (5) **`amp_cab_combined_chain_hex_for_slot_if_better`** : expose le **`chainHex`** combiné **`ampHex` + littéral `1a` + `cabHex`** quand il existe dans le catalogue et que l’ID extrait est vide ou égal au seul ampli. (6) **Catégorie affichée « Amp+Cab »** : si le catalogue porte **Preamp** / **Amp** / **Amp+Cab** pour l’entrée jointe, l’affichage grille force **Amp+Cab** lorsque le marqueur segment est présent. (7) **Parseur de secours** : si un segment assignable Amp+Cab ne produit **aucun** slot dans la boucle **`19…1a`** classique, on pousse le résultat de **`extract_first_module_from_assignable_chunk`** pour ne pas laisser un slot sans **`module_hex`** en vue **flow** (grille Kempline non reconnue). **État** : certains presets / firmwares peuvent encore échouer (catalogue sans entrée **`chainHex`** combinée, cas **`IR`**, ou bruit binaire) — à poursuivre avec un dump segment réel si besoin. Toujours ouvert : **IR**, longueurs de liste / champs internes vs **`params[]`**.

**23 avril 2026** — **validation terrain Amp+Cab, Amp+Cab+EQ, UI matrice et I/O** (Rust + TS + CSS + catalogue). Correctifs appliqués pendant la session : (1) **Amp+Cab sans perte d’ID** : extraction renforcée des paires ampli/cab via signatures **`c219`** et patrons **`19 ... 1a ... 09`** pour éviter les inversions (*amp seul*, *cab seul* ou mauvais `chainHex` cab). (2) **Amp+Cab avec EQ** : lecture des blocs **`0b/0c`** et saut du padding **`00 + num_params`** pour réaligner les valeurs (cas ampli **`cd0207`** + cab **`cd02f0`**). (3) **Formatage Helix** : prise en charge correcte des formats type printf avec texte littéral (ex. **`%.0f deg`** → **`45 deg`**) pour les paramètres comme **Angle** des cab/mic/IR. (4) **Matrice path 2** : correction de l’affichage de **`Icons_line.png`** entre split/merge, y compris le premier séparateur après l’input, et activation des interactions sur ces séparateurs. (5) **Input / Main L/R cliquables + paramètres** : ajout des pseudo-slots I/O et récupération explicite des valeurs côté Rust (**commande Tauri dédiée**). (6) **Jointure stricte par ID (jamais par nom)** : pour I/O, distinction explicite entre **`chainHex`** et **`slotTypeHex`** (affichage **`chainHex: — (slotType XX)`**), avec résolution modèle par **`catalogModelId`** quand le `chainHex` n’est pas disponible dans le segment.

## À quoi sert l’application

**HXLinux** est un éditeur / explorateur de presets pour **Line 6 HX Stomp XL** (et IDs USB voisins listés dans le code), sur **Linux**, en application **desktop Tauri** (Rust + front web).

Fonctions déjà utiles en pratique :

- Connexion **USB** au boîtier, machine d’états côté Rust pour le protocole (inspiré du travail **Kempline / helix_usb**).
- Lecture des **125 noms de presets**, **activation** d’un preset (Program Change), **renommage** depuis l’UI.
- Chargement du **contenu binaire du preset actif**, parsing partiel en **« slots »** (catégorie + nom) pour l’affichage.
- Mise en page type **grille** (16 blocs + routage), données renforcées par **`stomp_layout`** (split/merge, grille USB quand dispo).
- **Panneau paramètres** (sous la grille dans la vue models) : clic sur un bloc → définitions **`.models`** (noms, min, max) + valeurs **chaîne** lues dans le segment binaire du slot (**pas** de requête USB supplémentaire ; tout vient du dump déjà chargé). Les pastilles de la matrice 16 portent **`data-kempline-slot-index`** (0–7 path 1, 8–15 path 2) pour cette corrélation.

**État réel des valeurs chaîne** : (1) décodage aligné avec `user_slot_reader` Kempline dans **`preset_chain_params.rs`** (pointeur après le délimiteur `09`, même séquence que Python `bytes_read`). (2) **Affichage** : le front aligne la liste brute **`chainValues`** sur l’**ordre des `symbolicID` dérivé du catalogue** (`params` imbriqués dans **`HX_ModelCatalog.json`**), puis n’affiche que les paramètres autorisés par ce catalogue, dans l’**ordre du `.models`**. Répli historique : si le catalogue n’a **pas** de liste `params` pour un modèle, l’alignement retombe sur l’ordre **`assign` + sans `assign`** du `.models` complet. Cas encore sensibles : **IR**, champs internes absents du catalogue ; **Amp+Cab** : logique d’inférence **`module_hex`** documentée au **22 avril 2026** (reste perfectible selon preset / catalogue).

Ce qui reste largement ouvert : **édition** des paramètres vers l’appareil, export/import de fichiers presets (voir `README.md`).

---

## Lecture des paramètres « dans la chaîne » (ce qui a été fait — avril 2026)

Les **valeurs** affichées dans la colonne **chaîne** ne viennent **pas** des fichiers `.models` : elles sont **décodées dans le binaire du preset** déjà reçu en USB (`RequestPreset` → accumulation dans `HelixState.preset_data`). Les `.models` fournissent seulement les **métadonnées** (nom du paramètre, min/max du slider HX Edit, défaut, `displayType`, etc.).

### Chaîne de traitement (Rust)

1. **`split_preset_by_8213`** (`lib.rs`) — découpe le flux en segments (marqueur `82 13` côté octets, équivalent au split hex `8213` chez Kempline).
2. **`kempline_grid_window_start_and_seg_count`** — retrouve la **fenêtre de 20 segments** validée comme grille Kempline (même critères que `try_parse_preset_kempline_grid` : segment `00`, `01`, `02`, `03` aux positions attendues, 16 blocs assignables en `06` ou `08`).
3. **`kempline_assignable_segment_bytes(data, slot_index)`** — pour un index **0…15** (ordre UI : path1 slots 0–7, path2 slots 8–15), renvoie le **segment brut** `&[u8]` correspondant à `KEMPLINE_ASSIG_INDICES[slot_index]`.
4. **`parse_assignable_segment_param_blocks`** (`preset_chain_params.rs`) — segment dont le premier octet est **`0x06` ou `0x08`** ; même recalage **`br`** / **`read_params_hex`** que **`user_slot_reader`** + **`read_params`** Kempline (`simple_filter.py`) après le motif **`85188317`** : un ou plusieurs blocs **`c219`** (cas standard **`c219`** seul ; Amp+Cab **`c319`** puis plusieurs **`c219`**). **`chain_param_values_for_assignable_segment`** dans **`lib.rs`** choisit le bloc **ampli** vs **cab** en classant chaque bloc par **`chainHex`** / catégorie catalogue (`HX_CATALOG_MODULE_BY_HEX`, `MODEL_ID_BY_HEX`, candidats hex avant **`1a`** si besoin). **Grille 16 — même binaire** : **`extract_first_module_from_assignable_chunk`**, **`augmented_module_ids_for_assignable_chunk`**, **`inferred_amp_cab_hex_keys`**, **`infer_amp_cab_hex_pair_from_segment_hex_body`** (voir paragraphe **22 avril 2026** en tête de fichier) alimentent **`module_hex`** pour **`get_active_preset_slots`** et la jointure TS **`getCatalogModelIdForHex`**.

Les valeurs renvoyées au front sont une liste **`ChainParamValue`** (sérialisation JSON **untagged** : booléen, nombre, ou chaîne hex pour les blobs).

### Chaîne de traitement (TypeScript)

1. Chaque pastille de la grille 16 a **`data-kempline-slot-index="0"` … `"15"`** (`gridSlotNode` dans `models.ts`).
2. Au clic, **`loadAndShowModelsParamsForSlot`** appelle **`invoke("get_active_preset_slot_chain_param_values", { slotIndex })`** (si l’index est défini), puis charge le JSON **`.models`** (cache + `read_models_definition_file` / fetch selon l’environnement).
3. **`getCatalogModelIdForHex(slot.moduleHex)`** (`hxModelCatalogMeta.ts`) résout l’**`id`** catalogue depuis **`presetMeta.chainHex`** (lookup **`byHex`**, pas de collision entre deux modèles même nom). Si aucun id : message d’erreur explicite (pas de contournement par nom pour charger le `.models`).
4. **`findModelDefinitionForSlot`** charge le **`.models`** et ne retient que l’entrée dont **`symbolicID`** = **`id`** catalogue (**jointure stricte par id**).
5. **`renderModelsParamsPane`** : extrait l’ordre des **`symbolicID`** depuis **`HX_ModelCatalog.json`** (`getCatalogParamOrderForId`) ; **filtre** le **`params[]`** du `.models` pour ne garder que ces id ; **trie** les lignes dans l’**ordre du `.models`** ; **`alignChainValuesToModelParamOrder`** mappe **`chainValues`** sur la séquence catalogue (avec ou sans entrées **`stereo-only`** en mono selon la longueur reçue), puis projette par **`symbolicID`** sur les lignes affichées. **Masquage mono** : parmi les lignes affichées, celles avec **`"stereo-only": true`** sont omises si le signal est **mono** (`pickSignal` + `moduleHex`).

### Ce qui n’était pas (encore) dans la description avant cette complétion

- Le **fil exact** preset → segment slot → hex → `read_params` → `invoke` → zip avec `.models`.
- Le **rôle distinct** : binaire = valeurs, `.models` = schéma d’UI.
- La **référence explicite** au Python Kempline pour la spec du parseur.

---

## Prochaine étape : fichiers `.models` et affichage (virgule, 0 / 1 / 2 → libellés)

**Oui** : pour que l’affichage colle à l’interface HX (virgule au bon endroit, **0 / 1 / 2** affichés comme **220 Hz / 800 Hz / 3000 Hz**, etc.), il faudra **enrichir** les données — en pratique les **`.models`** (champs **optionnels** pour ne pas casser les outils qui s’attendent au format Line 6 d’origine) **ou** un fichier / table séparée dans le dépôt.

Pistes de champs (à valider ensemble avant implémentation dans `models.ts`) :

- **Échelle linéaire** : par ex. `chainScale`, `chainOffset`, `chainDecimals` (appliqués à la valeur numérique brute renvoyée pour la colonne « chaîne » ou une colonne dérivée « affichage »).
- **Liste discrète** : par ex. `chainEnum` : `[{ "raw": 0, "label": "220 Hz" }, …]` ou tableau de labels indexés par l’entier lu ; si la chaîne est un **u8** `2`, afficher le libellé d’index 2.
- **Registre dans le code** : pour les `displayType` répétitifs, une seule règle dans `models.ts` évite de dupliquer des milliers de lignes dans les JSON ; les cas **spécifiques à un modèle** restent dans le `.models` de ce modèle.

Pour les `displayType` **non** couverts par `HelixControls.json` (voir section suivante), la colonne **chaîne** reste une **vue brute** (ou légèrement formatée : bool on/off, float arrondi côté Rust) — d’où les écarts d’échelle par rapport à l’écran du Helix.

---

## Panneau paramètres — déjà traité dans `src/models.ts` (mémo pour ne pas refaire la demande)

Tout ceci concerne la vue **models** : grille + panneau **nom | min | cellule | max**. La **cellule** centrale contient soit un **slider** (valeur formatée Helix au-dessus + `<input type="range">`), soit une paire de **boutons Off / On** pour les paramètres booléens (voir ci-dessous). La **valeur brute JSON** de la chaîne reste en **infobulle** sur la ligne (et sur le curseur). Les colonnes sont alignées via **`display: grid` sur `ul.models-params-list`** et **`grid-template-columns: subgrid` sur chaque `li.models-params-row`** ; sans support **subgrid**, repli sur une **table** HTML (`display: table`) à colonnes resserrées.

### En-tête du panneau (`#models-params-pane-header`) — grille 2 colonnes × 2 lignes

| Cellule | Contenu | Alignement / style |
|---------|---------|---------------------|
| **(1,1)** | Titre **catégorie** du slot (`#models-params-pane-title`) | Gauche, **`var(--amber)`** |
| **(2,1)** | **`presetMeta.emulationName`** (`#models-params-pane-emulation-name`, via **`pickEmulationName`**) | Droite, **blanc** ; masqué si chaîne vide |
| **(1,2)** | Sous-tête modèle (nom court catalogue, canal/signal, nom USB si différent, etc.) | Gauche |
| **(2,2)** | **Icône** (`icons_models/` ou repli `icons_category/`) | Droite ; **survol** → popover **`position: fixed`** sur `body` avec la même URL, image **`width`/`height: auto`** jusqu’à **`max-width: 90vw`** / **`max-height: 85vh`** (évite les PNG énormes hors écran) ; fermeture avec léger délai + survol de la popover |

Structure HTML : les **quatre** blocs sont des **enfants directs** du `header` (ordre DOM : titre, emulation, subhead, wrap icône) ; le placement repose sur **`grid-column` / `grid-row`** en CSS (`src/styles.css`, classes **`.models-params-pane-*`**).

### Format des paramètres (`.models` ↔ ligne UI)

Chaque entrée **`params[]`** est un objet **`ModelParamDefJson`** côté `models.ts` :

- **`symbolicID`**, **`name`**, **`assign`** (optionnel, entier — ordre DSP côté firmware ; sert surtout de **repli** d’alignement quand le catalogue n’a **pas** de liste `params` pour ce modèle), **`displayType`** (clé vers **`HelixControls.json`** quand elle existe).
- **Variantes stéréo** (affichage uniquement, quand le signal catalogue est **stéréo**) : **`displayType_stereo`**, **`min_stereo`**, **`max_stereo`**, **`default_stereo`** remplacent respectivement **`displayType`**, **`min`**, **`max`**, **`default`** pour bornes, défaut et formatage Helix.
- **`valueType`** (usage Line 6) : **`0`** = entier pas slider / incréments entiers ; **`1`** = float ; **`2`** = **bool** (souvent avec **`displayType`** `off_on` côté Helix).
- **`min` / `max`** : en JSON Line 6 ce sont le plus souvent des **nombres** ; pour les bool **`off_on`** le fichier peut porter **`false` / `true`** — le front les accepte (**`number | boolean`**) pour l’affichage des bornes et la logique slider (**slider** uniquement si min/max sont des nombres avec **`max > min`**).
- **`"stereo-only": true`** : ligne **non affichée** en **mono** parmi les paramètres autorisés par le catalogue ; la valeur peut toutefois être présente dans la chaîne binaire (d’où la logique d’alignement avec/sans ces entrées en mono).
- **`default`** : peut être nombre, chaîne ou bool selon le modèle ; **`default_stereo`** si le défaut diffère en stéréo.

**Valeurs chaîne** (`invoke` → **`ChainParamValueJson`**) : **`boolean`**, **`number`**, ou **`string`** (hex blob). Pour l’**UI bool** : la cellule affiche les **boutons Off/On** si la valeur se lit comme bool (**`true`/`false`**) **ou** entier **`0`/`1`** *et* (**`valueType === 2`** **ou** `displayType` normalisé en **`off_on`**). Libellés des boutons : tableau **`format`** à deux chaînes dans **`HelixControls.json`** pour ce `displayType`, sinon défaut **Off / On**. Les clics mettent à jour **l’aperçu local** (texte chaîne + infobulle) en respectant le type d’origine (**bool** vs **0/1**) ; **aucune** écriture vers le Helix (idem que le slider d’aperçu).

**Formatage affiché** (`formatChainParamValueJson`) : les **bool** passent par **`formatHelixFromControl`** quand le `displayType` a une entrée Helix (ex. index 0/1 sur tableau `format`) ; sinon repli **`on` / `off`**.

### Jointure catalogue ↔ `.models`

- **Stricte** : le **`module_hex`** du slot → **`getCatalogModelIdForHex`** lit **`presetMeta.chainHex`** dans **`HX_ModelCatalog.json`** via une map **`chainHex` → entrée** (**`byHex`**), puis l’**`id`** catalogue. En **Amp+Cab**, le binaire joint souvent une seule chaîne **`ampHex` + `1a` + `cabHex`** (cf. **`cab_info_from_module_id`** dans **`lib.rs`** et commentaires équivalents dans **`hxModelCatalogMeta.ts`** ; candidats de lookup côté TS : chaîne complète puis préfixe avant **`1a`**).
- **`.models`** : une seule entrée dont **`symbolicID`** = cet **`id`** (**pas** de résolution par nom pour cette étape : deux modèles peuvent partager le même **`name`** avec des **`symbolicID`** et paramètres différents).
- Métadonnées **`presetMeta`** (canal, signal, `chainHex` parallèle, `emulationName`) : **`src/hxModelCatalogMeta.ts`** expose aussi **`byId`** (métadonnées + ordre des paramètres catalogue) et **`byCategoryAndName`** (vue historique ; en cas de doublon nom+catégorie, seule la **première** entrée est gardée pour cette clé — la jointure **`chainHex` → id** ne dépend pas de cette map).

### Règles d’affichage : qui apparaît, dans quel ordre

- **Liste affichée** : intersection entre le **`params[]`** du **`.models`** et les **`symbolicID`** listés dans le champ **`params`** du modèle dans **`HX_ModelCatalog.json`** (parcours récursif des objets `{ "SymbolicID": null }` pour produire un ordre de clés). Si le catalogue **ne** définit **pas** de `params` pour ce **`id`**, on affiche **tout** le **`params[]`** du `.models` (comportement de repli).
- **Ordre des lignes** à l’écran : **ordre du `.models`** parmi les paramètres retenus (pas l’ordre du catalogue).
- **Deuxième règle (signal)** : en **mono**, masquer les lignes avec **`"stereo-only": true`** (définition toujours lue dans le `.models`).

### Alignement liste `params` ↔ valeurs chaîne

- **Source d’ordre pour le zip** : quand le catalogue fournit une liste de **`symbolicID`**, **`alignChainValuesToModelParamOrder`** considère cette liste comme l’ordre des valeurs successives dans **`chainValues`** (en **mono**, deux variantes : avec ou sans les id marqués **`stereo-only`** dans le `.models` ; on retient celle dont la longueur est la plus proche de **`chainValues.length`**). Les valeurs sont ensuite assignées aux lignes affichées par **`symbolicID`**.
- **Repli** : catalogue sans `params` → ordre source = **`assign`** croissant puis paramètres **sans** **`assign`** dans l’ordre du **`params[]`** complet du `.models` (même logique mono avec/sans **`stereo-only`**).
- Cela corrige les décalages où l’ordre DSP / JSON **`.models`** ne coïncide pas avec l’ordre réel des valeurs (ex. sync avant **Time** dans le catalogue, **Cosmos Echo**).

### Source des règles d’affichage « chaîne »

- Fichier **`src-tauri/resources/HelixControls.json`** chargé côté front (fetch sur `/src-tauri/resources/HelixControls.json`), cache en mémoire.
- Les clés du JSON correspondent au **`displayType`** du paramètre dans le `.models`.

### Formatage « chaîne » via `HelixControls.json` (pipeline générique)

Pour toute valeur **numérique** dont le **`displayType`** est une clé présente dans `HelixControls.json`, `src/models.ts` applique **`formatHelixFromControl`** :

1. **Exception** optionnelle : objet **`HELIX_DISPLAY_EXCEPTIONS`** (clé = `displayType`) pour court-circuiter le générique si un cas ne colle pas.
2. **`format` = tableau de chaînes** (ex. `off_on`, `sync_note`) → libellé par index **`Math.round(valeur)`** (borné au tableau).
3. **`format` = tableau d’objets** (`lowerBound` / `upperBound`) → choix de la plage **`[lower, upper)`** sur la **valeur brute**, puis fusion des champs `format` / `formatUnits` / `unitsMultiplier` de la plage ; si `format` n’est **pas** un motif `%.…f` → texte littéral (**`Off`**, etc.).
4. Sinon **`format` chaîne** : `valeur × dspToDisplayScale` (si défini), puis `× unitsMultiplier` (si défini), puis **`format`** (`%.…f`) et substitution dans **`formatUnits`** si elle contient un token `%.…f` ; les séquences **`%%`** dans `formatUnits` deviennent un **`%`** littéral (comme sprintf).
5. Sinon entrée **`isDiscrete: true`** sans `format` exploitable → affichage **`Math.round(valeur)`** ; sinon repli numérique brut.

Détails d’implémentation utiles :

- **`alias`** dans `HelixControls.json` (ex. `time_ms_20_1800` → `time_ms`) : résolu **au chargement** ; la map expose la définition complète pour chaque clé.
- **Plages `format[]` + `dspToDisplayScale`** (ex. `time_ms`) : le choix de la plage utilise **`valeur_brute × dspToDisplayScale`** (unité d’affichage, ex. ms), puis on réapplique le même facteur pour le formatage final — les bornes du JSON sont alignées sur l’affichage, pas sur le brut secondes.

Les cas déjà validés manuellement (**`generic_knob`**, **`generic_knob_1to1`**, **`frequency`**, **`eq_low_cut`**, **`eq_high_cut`**) restent couverts par ce même moteur ; tout autre `displayType` présent dans Helix et dans les `.models` est **automatiquement** formaté selon sa définition JSON (sauf exception ajoutée dans `HELIX_DISPLAY_EXCEPTIONS`).

### UI debug

- **Infobulle** sur chaque ligne (et sur le curseur d’aperçu) : même texte que l’ancienne colonne **brute** — valeur reçue de la chaîne (avant format `HelixControls`).
- **Logs jointure ID** : `localStorage.setItem("models_debug_id_join", "1")` → `console.warn` si aucune entrée **`.models`** ne correspond à l’**`id`** catalogue résolu (après essai des fichiers par catégorie).
- **TODO** : mode debug optionnel (longueur `chainValues`, stratégie mono, variante stéréo par param) — voir **`TODO.md`**.

---

## Stack technique

| Couche | Rôle |
|--------|------|
| **Rust / Tauri 2** | USB (`rusb`), threads listener/écriture, état `HelixState`, commandes `invoke` exposées au front. |
| **TypeScript + Vite 6** | UI : `src/main.ts` (liste presets + intégration workspace), `src/models.ts` (vue « chaîne / grille » des blocs du preset + panneau paramètres). |
| **CSS** | `src/styles.css` — styles partagés ; la page `models.html` importe aussi ce fichier via `models.ts`. |

Build front : `npm run build` (`tsc` + `vite build`). App complète : `npm run tauri dev` / `npm run tauri build`.

## Structure des dossiers (utile au quotidien)

```
hxlinux/
├── index.html              # Fenêtre principale : liste + panneau « HX Models » (même document que main.ts)
├── models.html             # Entrée Vite secondaire (build MPA) ; utile si tu ouvres cette page seule en dev
├── description.md          # Ce fichier — mémo de reprise de session
├── src/
│   ├── main.ts             # Liste presets, statut, drag/rename, appels invoke vers Rust
│   ├── models.ts           # Rendu grille / chaîne preset, polling, stomp_layout, panneau params + invoke chaîne
│   └── styles.css          # Tout le look `.models-pane`, matrice `hx-matrix-*`, `.models-params-*`, etc.
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs          # Commandes Tauri, AppState, parse preset, fenêtre Kempline 20 segments, Amp+Cab / `module_hex`, invoke
│   │   ├── preset_chain_params.rs  # parse segment slot 0x06|0x08 : 85188317 / c219 / read_params (serde → UI)
│   │   ├── stomp_layout.rs # Layout stomp + routing (split/merge cols) aligné USB / heuristiques
│   │   └── helix/          # Protocole : modes (connect, request preset(s), standard…), USB, paquets
│   ├── resources/          # Bundlé : HX_ModelCatalog.json, icons_*, models/*.models (gros fichiers)
│   └── tauri.conf.json     # devUrl 1420, ressources bundle
└── README.md               # Statut produit, prérequis, crédits Kempline
```

## Deux surfaces front pour les « models »

1. **Dans la fenêtre principale** (`index.html`) : section `.models-pane` avec `<main class="models-content" id="content">`. **`main.ts` et `models.ts` sont tous les deux chargés** sur cette page ; `models.ts` attache son UI à `#content` / `#status` / `#preset-label` **du panneau droit** (attention aux `id` dupliqués si tu dupliques des fragments HTML).
2. **`models.html`** : page dédiée au build Vite ; `models.ts` y importe `./styles.css`. Le `<main id="content" class="content models-pane">` sert à activer les sélecteurs `.models-pane` / `#content.models-pane` (layout matrice, largeur grille, etc.).

En dev Tauri, ce qui compte le plus est souvent **index + models.ts** dans le même WebView.

## Rust — commandes exposées (`invoke`)

Déclarées dans `lib.rs` (`tauri::generate_handler![...]`), typiquement utilisées par le front :

| Commande | Rôle court |
|----------|------------|
| `get_preset_names` | Liste des noms (125 entrées). |
| `get_active_preset` | Index preset actif (0-based côté app). |
| `get_connected_device_name` / `get_connection_hint_text` | Statut connexion / message utilisateur. |
| `activate_preset` | Program Change USB. |
| `rename_preset` | Renommage sur l’appareil (ASCII, longueur limitée). |
| `request_preset_content` | Lance la lecture du dump preset actif. |
| `get_active_preset_slots` | Slots **`[catégorie, nom, module_hex]`** (triplet JSON) quand le dump est prêt **et** cohérent avec `active_preset` ; **`module_hex`** = chaîne entre **`19…1a`** ou **`ampHex` + `1a` + `cabHex`** inféré pour Amp+Cab (voir **22 avril 2026**). |
| `get_active_preset_slots_debug` | Idem + coords grille debug. |
| `get_active_preset_routing_markers` | Entrées routing type Split/Merge si présentes dans le parse. |
| `get_active_preset_stomp_layout` | Objet `ActivePresetStompLayout` (grille OK, split/merge cols, etc.). |
| **`get_active_preset_slot_chain_param_values`** | **`{ slotIndex: 0..15 }`** → `Vec<ChainParamValue>` ou `null` : valeurs décodées `read_params` pour le segment assignable Kempline du slot (voir `preset_chain_params.rs`). |
| `read_models_definition_file` | Lecture d’un `resources/models/{base}.models` côté bundle (nom de base alphanumérique). |
| `get_preset_data_hex` | Dump brut hex (debug). |
| `request_active_preset_name` | Resync nom preset actif. |

Le flux côté `models.ts` : après changement de preset → `request_preset_content` → boucle d’attente → `get_active_preset_slots` + routing + `get_active_preset_stomp_layout` pour `renderGrid16`. Au clic sur un slot avec modèle → `get_active_preset_slot_chain_param_values` si `data-kempline-slot-index` est défini, fusion avec le JSON `.models` chargé (fetch ou `read_models_definition_file`).

## Fichiers Rust à connaître pour le preset / UI grille

- **`lib.rs`** — `parse_preset_slots`, `split_preset_by_8213`, `kempline_grid_window_start_and_seg_count`, `kempline_assignable_segment_bytes`, `try_parse_preset_kempline_grid`, `KEMPLINE_ASSIG_INDICES`, **`is_amp_cab_assignable_chunk`**, **`extract_first_module_from_assignable_chunk`**, **`augmented_module_ids_for_assignable_chunk`**, **`inferred_amp_cab_hex_keys`**, **`infer_amp_cab_hex_pair_from_segment_hex_body`**, **`amp_cab_combined_chain_hex_for_slot_if_better`**, tests **`assignable_*`** / **`extract_first_module_amp_cab_inference_tests`**, commentaires `[PresetDebug]`.
- **`preset_chain_params.rs`** — `parse_assignable_segment_param_blocks`, `read_params_hex`, enum sérialisable `ChainParamValue` (bool, float IEEE via `ca`, u8, blob `1bda`).
- **`stomp_layout.rs`** — `split_merge_from_usb_preset_body`, `compute_stomp_layout_from_kempline_grid_with_usb`, tests ; colonnes split/merge consommées par `models.ts`. Le helper `merge_after_col_from_usb_preset_body` n’existe qu’en build test (`#[cfg(test)]`) pour éviter un warning `dead_code` en `cargo build` lib.

## Front — matrice stomp 4×20 (`renderGrid16` dans `models.ts`)

Grille **20 colonnes × 4 lignes**, cellules **56×56 px** (`NUM_COLS = 20`, `NUM_ROWS = 4`, `CELL_PX = 56`). Nomenclature des lignes dans le code :

| Ligne | Rôle |
|-------|------|
| **L1** | Path 1 — slots 0–7, I/O Input / Output, traits horizontaux **`Icons_line.png`** entre colonnes paires, pastille `Icons_split_merge.png` aux colonnes **jonction** (split/merge issus du routing). |
| **L2** | Description Path 1 — textes catégorie ; aux colonnes split/merge, petite barre verticale `Icons_vertical_line.png`. |
| **L3** | Path 2 — slots 8–15 si branche B ; aux mêmes colonnes, icônes coin **`Icons_link_split.png`** / **`Icons_link_merge.png`** (alignées sur `stomp_layout`). **À corriger** : réintroduire ou aligner les **traits horizontaux `Icons_line.png`** sur cette rangée (équivalent visuel L1) — actuellement **manquant / incomplet sur Path 2** ; l’asset est dans **`src-tauri/resources/icons_category/Icons_line.png`**. |
| **L4** | Description Path 2 — catégories path B. |

- **Colonne 20** : numéros de ligne grille (debug lisible).
- **Colonnes « jonction »** : dérivées des frontières split/merge (1…8) via `matrixEvenColForRoutingBoundary` (colonnes paires 2…18 côté UI).
- **`ENABLE_MATRIX_VSPAN_ON_PATH2`** (`models.ts`) : par défaut **`false`**. Un overlay `vspan` vertical sur Path 2 partageait la même `grid-area` que les icônes lien ; les deux se superposaient. Laisser à `true` uniquement pour un revert visuel expérimental (commentaires **REVERT** à côté).
- **Ancienne mise en page (5 lignes + rangée 3 « séparateur » 0 px)** : le retour est documenté en blocs commentés **REVERT** dans `models.ts` et `styles.css` (constantes de lignes, hauteurs de rangées, boucle séparateur, classes `row-line-debug-sep`, etc.).

Panneau paramètres : **`ul.models-params-list`** = grille à 4 colonnes partagées ; chaque **`li.models-params-row`** = **subgrid** sur ces colonnes, enfants directs **nom | min | cellule | max** (valeur formatée dans **`.models-params-slider-cell`** : slider **ou** **`.models-params-bool-toggle`** + **`.models-params-bool-btn`** ; classes `.models-params-row-min`, `-chain`, `-max`).

Le CSS associé est sous **`.models-pane .hx-matrix-*`** et **`.models-params-*`** dans `styles.css`. Des régressions visuelles passent souvent par : parent sans `.models-pane`, ou styles inline dupliqués dans `models.html` vs `styles.css`.

## Ressources et métadonnées Line 6

- **`src-tauri/resources/HX_ModelCatalog.json`** — catalogue modèles.
- **`src-tauri/resources/icons_models/`** — icônes par modèle.
- **`src-tauri/resources/icons_category/`** — icônes catégories + assets maison pour la matrice : `Icons_line.png`, `Icons_split_merge.png`, `Icons_vertical_line.png`, `Icons_link_split.png`, `Icons_link_merge.png`, ainsi que les icônes I/O (`icon-input-category.png`, etc.).
- **`src-tauri/resources/models/*.models`** — définitions JSON Line 6 (params, min/max, `displayType`, `valueType`, etc.) ; utilisées pour le panneau paramètres et le matching id catalogue ↔ `symbolicID`.
- **`src-tauri/resources/HelixControls.json`** — données controls (fichier ajouté au bundle ; brancher dans l’app si besoin).

Chemins côté front pour les PNG sous Tauri : souvent `/src-tauri/resources/...` (comme dans `models.ts` pour les I/O).

### Catalogue HX — `presetMeta`, `chainHex`, mono / stéréo (mémo session)

- Chaque modèle du JSON peut porter un objet **`presetMeta`** : notamment **`chainHex`** (une chaîne hex **ou** un tableau **`[mono, stéréo]`**) et **`signal`** en parallèle (`["mono", "stereo"]`) quand le même bloc existe en deux variantes.
- **`src-tauri/src/lib.rs`** : au build, **`HX_CATALOG_MODULE_BY_HEX`** est rempli **uniquement** depuis **`include_str!("../resources/HX_ModelCatalog.json")`** en parcourant tous les `presetMeta.chainHex` (chaîne ou tableau) + nom court du modèle. C’est cette table qui sert à résoudre l’UID hex du segment preset vers **catégorie + nom** affichés.
- **`src/hxModelCatalogMeta.ts`** : `fetch` du catalogue sous `/src-tauri/resources/HX_ModelCatalog.json` (cache au premier chargement ; **recharger l’app** après édition du JSON). Trois index en mémoire : **`byHex`** (`chainHex` → entrée, jointure slot → **`id`**), **`byId`** (**`id`** → `presetMeta`, image, **`catalogParamOrder`** = liste ordonnée des **`symbolicID`** extraits du champ **`params`** du catalogue), **`byCategoryAndName`** (première entrée par paire catégorie+nom, usages historiques). Helpers **`getCatalogModelIdForHex`**, **`getCatalogParamOrderForId`**, **`getPresetMetaForId`**, **`pickSignal` / `pickChannel` / `pickEmulationName`**.
- **`scripts/apply_mono_stereo_pairs_to_catalog.py`** : lit **`HX_ModelCatalog.json`**, détecte les paires mono/stéréo à partir des fiches qui ont déjà un **`chainHex`** et un libellé **`(mono)`** / **`(stereo)`** / **`(stéréo)`**, met à jour la fiche : `chainHex` + `signal` en tableaux.
- **`scripts/enrich_catalog_preset_meta.py`** — complète les champs texte vides de **`presetMeta`** depuis le seul champ **`name`** du modèle (ne remplit pas **`chainHex`**).
- **Travail restant (manuel)** : compléter les entrées **`"chainHex": ""`** dans **`HX_ModelCatalog.json`** (compter avec `rg '"chainHex":\\s*""' src-tauri/resources/HX_ModelCatalog.json`) en lisant l’hex sur le boîtier ou une autre source fiable.

**Note** : une copie **`External files/HX_ModelCatalog.json`** peut exister hors bundle ; les commits UI « légers » peuvent l’**exclure** (diff très volumineux) — resynchroniser à la main si tu t’en sers comme miroir.

### Git — commits sans indexer les gros sous-dossiers de `resources/`

Pour préparer un commit **sans** inclure les changements sous `icons_category/`, `icons_models/`, `models/` (trop lourds ou générés ailleurs), depuis la racine du dépôt :

```bash
git add -A \
  ":(exclude)src-tauri/resources/icons_category" \
  ":(exclude)src-tauri/resources/icons_models" \
  ":(exclude)src-tauri/resources/models"

git status
git commit -m "Ton message"
git push origin refactor/multithread
```

Les fichiers **à la racine** de `src-tauri/resources/` (ex. `HX_ModelCatalog.json`) restent éligibles au staging s’ils sont modifiés. Ajoute d’autres `:(exclude)…` si tu dois aussi ignorer `External files/` ou autre.

**Commits / contexte** : sur la branche **`refactor/multithread`**, le commit **`f79be40`** reste une référence pour `preset_chain_params` + première itération UI min | chaîne | max. Un commit local ultérieur (**`dd9ee9f`**, message **`feat(models): panneau paramètres et en-tête catalogue`**) regroupe notamment **`index.html`**, **`models.html`**, **`src/models.ts`**, **`src/styles.css`**, **`src/hxModelCatalogMeta.ts`** (en-tête 2×2, `emulationName`, toggles bool, aperçu survol icône, etc.). Le **21 avril 2026**, commit **`de27037`** (*Preset chaîne, catalogue HX et panneau paramètres*) : alignement **`assign`** côté TS, segments **`0x06|0x08`**, évolutions **`lib.rs`** / catalogue / scripts / styles, suppression **`modules_by_id.json`**. Les gros diffs **`HX_ModelCatalog.json`** / **`TODO.md`** / **`description.md`** peuvent rester hors commit jusqu’à message dédié — voir **`git log`** / **`git status`**.

## Reprise rapide après redémarrage

1. Lire **`README.md`** + ce **`description.md`**.
2. Lancer **`npm run tauri dev`** (ou `npm run dev` pour le front seul sur `http://localhost:1420`).
3. Pour toute modification UI models : **`src/models.ts`** + **`src/styles.css`** ; vérifier que **`models.ts` importe bien `./styles.css`** si tu travailles sur `models.html`.
4. Pour protocole / parsing preset / valeurs chaîne : **`src-tauri/src/lib.rs`** + **`preset_chain_params.rs`** + **`stomp_layout.rs`** + modules **`helix/`**.

Bon courage pour la suite.
