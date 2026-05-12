# TODO HXLinux

## Refactor nommage « Kempline »

- [ ] Renommer progressivement les identifiants **`kempline_*`**, types **`KemplineCell`**, attributs **`data-kempline-slot-index`**, commande Tauri **`get_active_preset_kempline_flow_chain_param_values`**, etc., vers un vocabulaire **produit** (ex. grille 16 slots, `preset_slot_index`, `grid16_*`, `flow_segment_*`).
- [ ] Ajouter une courte section dans **`README.md`** : l’app s’inspire du reverse **helix_usb / Kempline** ; le code actuel **n’est plus** une traduction ligne à ligne — les comparaisons avec les analyses Kempline ne suffisent pas à juger « bon / faux » sans contexte HXLinux.
- [ ] Conserver une **table de correspondance** (ancien nom → nouveau) dans le premier commit du refactor, pour les recherches git et les discussions issues.

_Raison : éviter que des développeurs optimisant ou modifiant le dépôt comparent avec Kempline et concluent à tort à une erreur d’implémentation._

## `HX_ModelUsbAssign.json` — complétude, schéma, alignement catalogue

- [ ] **Campagne hardware** : vérifier les **autres familles de modèles** (au-delà des distorsions / ce qui est déjà capturé), captures USB si besoin, et **ajouter / valider** les entrées dans **`src-tauri/resources/HX_ModelUsbAssign.json`** (une ligne `id` + `variant` + `bulkHex` valide par cas testé).
- [ ] **Audit de structure** : aujourd’hui le Rust (`helix/edit_slot_model.rs`, `load_usb_assign_entries`) ne lit que **`id`**, **`variant`**, **`bulkHex`**. Le picker (`hxModelCatalogMeta.ts`, `loadUsbAssignPickerDataFromJson`) lit en plus **`name`**, **`category`**, **`subCategory`**. Les champs **`edOpcode`**, **`bulkKind`**, **`chainHexHint`** (et **`notes`**) ne sont **pas** consommés par le code — redondants ou purement doc par rapport au bulk. Décider : les retirer, les garder comme doc seulement (mettre à jour la description du fichier + schéma `schemaVersion`), ou les **dériver / valider** par script à partir de `bulkHex` pour éviter la dérive.
- [ ] **Alignement `HX_ModelCatalog.json`** : pour chaque entrée (ou via script), **importer `presetMeta.basedOn`** (et sa valeur) depuis le catalogue **pour la même `id`**, afin d’afficher / filtrer côté UI de façon cohérente avec HX Edit sans dupliquer à la main. Vérifier les cas mono/stéréo / `chainHex` tableau.
- [ ] **`chainHexHint` vs catalogue** : intention produit = s’affranchir des **`chainHex` / params erronés** du catalogue pour l’USB. Or **`patch_catalog_chain_into_bulk`** utilise encore **`resolve_catalog_model_chain_bytes`** (`HX_ModelCatalog.json`) quand la chaîne catalogue est assez longue. **`chainHexHint`** dans le JSON d’assign n’est **pas** lu — à exploiter (ou un champ **`chainHexUsb`** dédié) comme **source prioritaire** pour le patch quand présent, avec repli catalogue seulement si absent.
- [ ] **Ordre d’affichage picker vs ordre hardware** : l’ordre des modèles dans le picker suit aujourd’hui **l’ordre des lignes** dans **`HX_ModelUsbAssign.json`**. Une insertion au milieu **décale** l’ordre d’énumération côté fichier sans que ce soit l’ordre « mémoire hardware ». Réfléchir à un champ explicite (**`hardwareOrder`**, **`programIndex`**, etc.) stable, ou une convention « ne trier que par ce champ », documentée dans le schéma du fichier.

_Voir aussi le bloc **Todo** dans **`description.md`**._
