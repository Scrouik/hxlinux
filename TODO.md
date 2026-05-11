# TODO HXLinux

## Refactor nommage « Kempline »

- [ ] Renommer progressivement les identifiants **`kempline_*`**, types **`KemplineCell`**, attributs **`data-kempline-slot-index`**, commande Tauri **`get_active_preset_kempline_flow_chain_param_values`**, etc., vers un vocabulaire **produit** (ex. grille 16 slots, `preset_slot_index`, `grid16_*`, `flow_segment_*`).
- [ ] Ajouter une courte section dans **`README.md`** : l’app s’inspire du reverse **helix_usb / Kempline** ; le code actuel **n’est plus** une traduction ligne à ligne — les comparaisons avec les analyses Kempline ne suffisent pas à juger « bon / faux » sans contexte HXLinux.
- [ ] Conserver une **table de correspondance** (ancien nom → nouveau) dans le premier commit du refactor, pour les recherches git et les discussions issues.

_Raison : éviter que des développeurs optimisant ou modifiant le dépôt comparent avec Kempline et concluent à tort à une erreur d’implémentation._
