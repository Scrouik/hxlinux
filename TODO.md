# TODO — HXLinux

## Fonctionnalités à implémenter

- [ ] **Déplacement de presets** — Drag & drop dans la liste avec décalage réel sur le HX.
      Nécessite la lecture et l'écriture complète des données d'un preset (pas encore implémenté).

- [ ] **Corriger l'instabilité du HX après déconnexion** — Le pédalier se retrouve dans un état instable après une déconnexion USB. À investiguer et corriger.

- [ ] **Détection du modèle HX connecté** — Identifier automatiquement le modèle branché (Stomp, Stomp XL, Floor, LT…) et adapter le comportement en conséquence.
      const SUPPORTED_DEVICES: &[(&str, u16, u16)] = &[
      ("HX Stomp XL",  0x0e41, 0x4253),
      ("HX Stomp",     0x0e41, 0x4246),
      ("Helix Floor",  0x0e41, 0x5055),
      // à compléter
      ];


## Fonctionnalités partiellement implémentées

- [ ] **Double-clic sur preset** — Envoyer un MIDI Program Change pour charger le preset sur le HX (`à implémenter` dans `src/main.ts`).

- [ ] **Sauvegarde preset sur disque** — Exporter un preset en fichier `.hlx` (`à implémenter` dans `src/main.ts`).

- [ ] **Chargement preset depuis disque** — Importer un fichier `.hlx` vers un slot (`à implémenter` dans `src/main.ts`).

## Améliorations futures

- [ ] Lecture des paramètres complets d'un preset (chaîne de signal, effets, valeurs).
- [ ] Édition en temps réel des paramètres.
- [ ] Visualisation de la chaîne de signal.
- [ ] Génération de presets via IA.
- [ ] Export / import de fichiers `.hlx`.
