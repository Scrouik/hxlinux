# Changelog

Toutes les modifications notables de HXLinux sont documentées dans ce fichier.

## [0.2.0] — 2026-08-12

### Command Center (nouveau)
- Nouvel espace **Command Center** avec onglets **Edit** et **Controllers** pour gérer les assignations de contrôleurs.
- Lecture et écriture live des assignations : **Source, Type, Couleur, Nom**, et état de **bypass** par slot.
- Création d'une assignation directement depuis un **footswitch**, affichage **multi-contrôles**, édition live des bornes **Min/Max** (écrites sur le device).
- Suppression d'assignations via une **colonne corbeille** (unitaire) et action **« tout supprimer »** avec fenêtre de confirmation intégrée.
- Effacement automatique des contrôles d'un slot avant un changement de modèle.

### Snapshots (nouveau)
- Bascule du **snapshot actif** (Snap 1 à 4) depuis les boutons du bandeau (caméra + numéro).
- Affichage des **valeurs de paramètres par snapshot** et des paramètres pilotés par snapshot dans la table des contrôles.
- **« Snapshot »** disponible comme source de contrôleur dans l'onglet Controllers, avec icône caméra sur les paramètres concernés.
- **Renommage** des snapshots, détection fiable du snapshot réellement actif, et cohérence de session USB.
- Corrections : un preset sans paramètre de snapshot s'affiche toujours sur **Snap 1** ; le snapshot actif ne reprend plus par erreur celui du preset précédent.

### Grille & édition
- **Drag & drop** dans la grille : déplacement de blocs et **création d'un split** par glisser-déposer.
- **Saisie clavier** de la valeur directement sur les sliders non-crantés (en plus du glissement).

### Stabilité lecture / sauvegarde (majeur)
- **Fin des gels de lecture de preset** : acquittement du trailer de fin de dump (clôture de transaction façon HX Edit) et contrainte device documentée.
- **Sauvegarde réparée** : le raccourci Ctrl+S n'est plus intercepté par un slider, la sauvegarde ne part plus sur une lane USB étrangère, et le padding des paquets de save/rename est aligné (multiple de 4) — c'était la vraie cause des gels au save et au renommage.
- **Identité des presets par index** : fin des presets « intriqués » (homonymes « New Preset ») lors des clics/éditions.
- **Source Input relue correctement** au chargement (Main / Return / USB), au lieu de retomber sur Main L/R par défaut.
- Numérotation **banque + lettre** (01A à 32D) affichée partout.
- Suppression d'une **caméra fantôme** lors d'un changement de modèle.

### Interne
- Chantier d'assainissement de l'affichage : introduction d'un modèle d'état unique **CurrentPreset** reconstruit depuis le dump (paliers 1 et 2a).
- Nettoyage de code mort et d'instrumentation de diagnostic.

## [0.1.1] — 2026-07-07

### Input / Output / Split / Merge (I/O spéciaux)
- Synchronisation complète lecture **et** écriture pour les slots spéciaux Input, Output, Split (tous types : Y, A/B, Crossover, Dynamique) et Merge — jusqu'ici seule la lecture partielle était disponible.
- Correction d'une confusion Split A/B ↔ Split Y : le paquet d'écriture corrompait la signature du bus dans le bloc modèle, et l'encodage fil de ces deux types était inversé par rapport à la convention catalogue (écriture, écho de lecture, écho de scroll matériel).
- Le panneau de paramètres reste maintenant correctement synchronisé quand on change de type de Split depuis l'UI ou depuis le device.
- Ajout d'une commande d'écriture dédiée pour les paramètres génériques des slots spéciaux, isolée du chemin d'écriture des blocs FX normaux.
- Masquage de doublons d'affichage : "Input From" et "Output To" (déjà représentés par leurs sélecteurs dédiés), et "Bypass" sur les blocs Split.
- Threshold/Decay grisés quand le Gate d'entrée est désactivé.

### Stabilité de la communication USB
- Réduction des accès directs au device : au changement de type Split, l'application affiche désormais les valeurs par défaut du catalogue plutôt que de relire l'état en direct — cette sollicitation excessive du device fragilisait la communication après un usage intensif.
- Correction d'un flash visuel affichant brièvement le mauvais modèle lors d'un changement rapide de preset.

### Autres corrections
- Correction de l'affichage de paramètres signés (ex. Pitch Wham) et des formats `%+d`/`%d`.
- Masquage des lignes de paramètres "Tempo Sync" absentes de l'écran du device sur ~107 modèles.
- Correction de l'écriture de paramètres booléens mal classés comme discrets (ex. compresseurs, distortions).
- Correction d'un blocage de lecture de preset et des valeurs par défaut des cabs legacy.
- Ajout d'une action "vider tous les blocs" sur un preset.
- Nettoyage de code mort et de warnings de compilation.

## [0.1.0] — Première version testable

Premier build utilisable pour les testeurs (HX Stomp XL sous Linux).
