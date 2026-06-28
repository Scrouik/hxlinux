# USB Slot Model Fixes (4-5 mai 2026)

**Voir aussi** : [Récap keep-alive `ed → ef → f0`](recap-keep-alive-ed-ef-f0-mai-2026.md) (polling éditeur, handler Kempline, silence `f0` sur Linux) — sujet distinct de l’assignation slot/modèle décrite ici.

## Contexte

L'objectif était de stabiliser l'assignation de modèles dans un slot via `probe_slot_model_usb` (HXLinux), en particulier le cas **Distortion mono** qui échouait alors que d'autres familles passaient.

Symptôme critique observé pendant l'investigation : certains essais Distortion mono "polluaient" l'état de session (state poisoning), puis les changements suivants pouvaient échouer même sur des modèles auparavant stables.

## Découvertes clés

1. Le `bulkHex` envoyé dépend bien du modèle cliqué (pas un template unique), mais cela ne suffit pas.
2. Le transport ED03 n'est pas uniforme :
   - profils valides observés en `80:10:ed:03`,
   - profils rencontrés en `03:10:ed:03`,
   - le profil réellement stable dépend de la capture de référence HX Edit.
3. Plusieurs octets sont dynamiques de session (compteurs / lane) et ne doivent pas toujours être rejoués figés.
4. Le framing (préambule/context packets) est aussi important que le payload modèle.
5. Le vrai test de réussite est : changement OK + changements suivants OK, sans dégrader l'état global.

## Correctifs appliqués

### Backend Rust

#### `src-tauri/src/helix/edit_slot_model.rs`

- Unification du chemin `usb_json` pour réduire les heuristiques fragiles accumulées.
- Patch dynamique des compteurs ED03 (`seq`/`ctr`) dans les chemins concernés.
- Gestion du byte lane après `83 66 cd 03|04` avec progression de session (`slot_model_lane_seq`) au lieu de valeurs figées de capture.
- Ajout d'un préambule court stable (`ef` puis `f0`) avant le bulk JSON.
- Nettoyage des variantes expérimentales pré/post devenues obsolètes.

#### `src-tauri/src/helix/mod.rs`

- Ajout de l'état `slot_model_lane_seq` dans `HelixState` pour piloter les octets dynamiques entre transactions successives.

#### `src-tauri/src/lib.rs`

- Simplification du flux `probe_slot_model_usb` et suppression de timings conditionnels devenus inutiles après l'unification.

### Ressources JSON

#### `src-tauri/resources/HX_ModelUsbAssign.json`

- Ajout et usage d'un champ structuré `edOpcode` (au lieu de dépendre de `notes`).
- Alignement des entrées Distortion mono sur le profil ED03 stable observé en comparaison HX Edit/HXLinux.
- Mise à jour cohérente de `bulkHex`, `bulkKind`, `edOpcode` pour les entrées concernées.

#### `src-tauri/resources/HX_ModelCatalog.json`

- Mise à jour associée du catalogue (fichier critique conservé dans les commits de correction).

## Résultat

- Distortion mono : changement de modèle à nouveau fonctionnel.
- Pas de régression visible sur les familles déjà stables.
- Disparition du state poisoning observé pendant les essais intermédiaires.

## Commits de cette phase

- `184423c` - `fix(usb): align distortion mono assign profiles with stable ED03 path`
- `66911bf` - `fix(app): include remaining runtime USB changes and resource cleanup`

Ces commits regroupent les fichiers runtime et ressources nécessaires au fonctionnement stable.
