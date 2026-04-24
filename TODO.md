# TODO — HXLinux

## Fonctionnalités à implémenter

- [ ] **Déplacement de presets (INSERT réel)** — Drag & drop dans la liste avec décalage réel sur le HX (comportement HX Edit).
      Statut: en pause. Prochaine étape: capturer sous Windows les paquets USB d'un move (ex: 120 -> 123) dans HX Edit, puis implémenter `move_preset_insert(from, to)` côté Rust + appel UI.

- [ ] **Corriger l'instabilité du HX après déconnexion** — Le pédalier se retrouve dans un état instable après une déconnexion USB. À investiguer et corriger.



## Fonctionnalités partiellement implémentées

- [ ] **Sauvegarde preset sur disque** — Exporter un preset en fichier `.hlx` (`à implémenter` dans `src/main.ts`).

- [ ] **Chargement preset depuis disque** — Importer un fichier `.hlx` vers un slot (`à implémenter` dans `src/main.ts`).

## Correction des legacy via doublon dans les noms.

- [ ] Exporter l'ensemble des noms de preset qui sont en doublon pour vérif manuelle.