# Changelog

Toutes les modifications notables de HXLinux sont documentées dans ce fichier.

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
