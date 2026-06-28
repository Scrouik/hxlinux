# HXLinux — Inventaire des fonctionnalités (v1 utilisable)

> **Périmètre :** HX Stomp XL sur Linux · état post-commit `First_usable_version` (juin 2026).  
> Ce document sert de base pour le README GitHub et la communication autour de la première version utilisable.

---

## Synthèse

HXLinux est un éditeur open source pour Line 6 Helix sur Linux (Tauri + Rust + TypeScript). Sur **HX Stomp XL**, il permet de :

- parcourir et renommer les **125 presets** ;
- visualiser et éditer la **chaîne de signal** (matrice stomp + panneau paramètres) ;
- **assigner, remplacer, copier, coller et déplacer** des blocs FX en USB live ;
- gérer les modèles **Amp+Cab** et **Cab Dual** (onglets + replace cab) ;
- piloter **Input** et **Split** Path 1 (live write + scroll hardware) ;

… le tout en **USB natif Linux**, sans HX Edit.

---

## 1. Plateforme & connexion

| Capacité | Détail |
|----------|--------|
| App desktop Linux | Tauri 2 (Rust + TypeScript/Vite), 2 fenêtres : presets + models |
| USB natif | Détection HX Stomp XL, handshake, machine à états protocole |
| Session stable | Fermeture propre sans figer le hardware en mode éditeur |
| Géométrie fenêtres | Sauvegarde/restauration position et taille |

---

## 2. Fenêtre principale — presets (125 slots)

| Capacité | Détail |
|----------|--------|
| Liste des noms | Lecture des 125 presets depuis le device |
| Activation preset | Clic → envoi sur le HX |
| Renommage | Clic droit → Rename, édition inline, envoi USB + mise à jour UI optimiste |
| Sauvegarde sur hardware | Clic droit → Save (preset actif uniquement, slot non vide) |
| Sélection visuelle | Preset actif mis en évidence |
| Drag & drop liste | Réordonnancement **UI seulement** — pas encore envoyé au HX |
| Load from disk | Menu présent, **non implémenté** |

---

## 3. Fenêtre Models — vue d'ensemble

| Capacité | Détail |
|----------|--------|
| Grille matrice stomp | 16 slots FX + blocs structurels (Input, Output, Split, Merge) |
| Routing visuel | Colonnes split/merge, paths 1 et 2 |
| Chargement preset | Dump USB one-shot → hydratation grille + cache params session |
| Overlay chargement | Verrou UI pendant le load preset |
| Bannière preset | Nom + indicateur « modifié » |
| Sync slot actif HW | Molette / boutons sur le Stomp → sélection UI alignée |

---

## 4. Matrice — édition des slots FX

| Capacité | Détail |
|----------|--------|
| Sélection slot | Clic → focus hardware + panneau paramètres |
| Copier | Clic droit sur slot rempli → snapshot modèle + params |
| Coller | Sur toute case FX vide (même path, autre path, **autre preset**) |
| Déplacer (DnD) | Pointer Events (pas HTML5 DnD) : copier → coller → vider source |
| Contrainte move v1 | Même path uniquement (0–7 ↔ 0–7, 8–15 ↔ 8–15), destination vide |
| Supprimer slot | Vider le slot sur le hardware |
| Cache session params | `preset_data` lu **une fois** au load ; ensuite cache + overrides live |
| Move Split/Merge | Déplacement des marqueurs routing (partiellement en place) |

**Pas encore :** DnD inter-path auto split/merge, drag Split/Merge libre, budget DSP.

---

## 5. Picker — changement de modèle

| Capacité | Détail |
|----------|--------|
| Liste scrollable | Modèles FX assignables (`HX_ModelUsbAssign.json` + catalogue) |
| Assign USB | Trames `bulkHex` capturées depuis HX Edit |
| Remove slot | Même mécanisme (vider le slot) |
| Scroll modèle HW | Molette sur slot actif → pull USB, sans re-dump systématique |
| Picker verrouillé | Input, Output, Split, Merge : catégorie figée selon le bus structurel |
| Exclusions | Catégories non assignables (ex. Split dans la liste FX) |

---

## 6. Panneau paramètres — édition live

### Types de contrôles

| Type | Exemples |
|------|----------|
| Slider numérique | Gain, Level, Time, etc. (float et entier) |
| Slider discret à crans | Ratio, Clipping, Wave shape, Type Compress/Limit |
| Toggle booléen | Bright, Fuzz, EQ on/off, polarity |
| Combo micro | Sélection micro (displayType dédié) |
| EQ graphique | Bandes masquées quand master EQ off |
| Formatage valeurs | `HelixControls.json` (unités, labels, paliers) |

### Règles d'affichage (alignées hardware)

- Masquage `stereo-only` en mono
- Masquage booléens internes (`@enabled`, `@stereo` — `valueType: 2` sans `displayType`)
- Échelles spéciales : pan 0…1 → −100…+100, split A/B, etc.
- Ordre wire : `assign` croissant puis ordre JSON (live-write correct)

### Écriture USB

- `write_live_param` (float / bool / discret)
- `write_live_param_midi_cc` si besoin

---

## 7. Modèles doubles — Amp+Cab & Cab Dual

| Capacité | Amp+Cab | Cab Dual |
|----------|---------|----------|
| Onglets | Amp \| Cab | Cab 1 \| Cab 2 |
| Params par onglet | Oui | Oui |
| Picker onglet secondaire | Cab verrouillé Single IR | Cab 2 verrouillé Single IR |
| Replace cab seul | Bulk modern ou séquence legacy | Bulk dual hint `c319` |
| Focus USB partie | Oui | Oui |
| Scroll HW | Détection amp + cab lié | Détection cab1 + cab2 |

---

## 8. Path 1 — blocs structurels

| Bloc | Picker | Live write | Scroll HW |
|------|--------|------------|-----------|
| **Input** | Verrouillé | Oui | Oui |
| **Split** | Verrouillé | Oui | Oui (encodage Y/A/B inversé scroll vs select) |
| **Output** | Verrouillé + focus | Non | Non |
| **Merge** | Verrouillé + focus | Non | Non |

Params I/O et flow (Split/Merge) : lecture depuis `preset_data` + affichage panneau.

---

## 9. Sync hardware ↔ UI

| Event | Déclencheur |
|-------|-------------|
| `models:hardware-slot-changed` | Changement slot actif sur le HX |
| `models:slot-model-changed` | Scroll / changement modèle |
| `models:slot-param-changed` | Twist knob sur le hardware |
| `models:slot-content-changed` | Watch contenu slot |
| `models:path1-input-source-changed` | Scroll / echo Input |
| `models:path1-split-type-changed` | Scroll / echo Split |
| `models:preset-saved` | Après save preset |

Soft-sync : pas de re-parse complet entre deux dumps si inutile.

---

## 10. Infrastructure USB (sous le capot)

- Lecture preset ed:03 (bugs compteurs 16 bits, double éditeur corrigés)
- Handshake Phase B (commit éditeur)
- Keep-alive et lanes couplées
- Scroll multi-cran sans freeze ED03
- Récupération lecteur preset
- Parse valeurs chaîne (Amp+Cab, Cab Dual, `c319`/`c219`)
- Layout stomp

---

## 11. Non implémenté ou partiel

| Domaine | État |
|---------|------|
| Helix LT / Floor | Non supporté (topologie 4 paths, 2 DSP) |
| Budget DSP (`load` dans `.models`) | Non calculé |
| Output / Merge live write + scroll | Partiel |
| DnD inter-path auto split/merge | Planifié |
| Réordonnancement presets sur HX | UI seulement |
| Import/export preset fichier | Non fait |
| Load preset depuis disque | Stub |
| Campagne `bulkHex` | Couverture partielle du catalogue |

---

## Prérequis

- Linux (testé famille Ubuntu/Debian)
- Line 6 **HX Stomp XL** connecté en USB
- **HX Edit** installé (pour fournir les fichiers métadonnées modèles)

## Lancer l'application

```bash
npm run tauri dev    # développement
npm run tauri build  # build production
```

## Installation (testeurs)

Binaires pré-compilés : [Releases GitHub](https://github.com/Scrouik/hxlinux/releases) — voir [install.md](install.md).

## Crédits

Reverse engineering USB inspiré de [kempline/helix_usb](https://github.com/kempline/helix_usb).

## Documentation technique

| Document | Contenu |
|----------|---------|
| [`description.md`](../description.md) | Mémo reprise de session |
| [`TODO.md`](../TODO.md) | Backlog priorisé |
| [`matrix-edit-handoff.md`](matrix-edit-handoff.md) | Matrice : copier/coller, DnD, cache session |
| [`models-hardware-sync.md`](models-hardware-sync.md) | Sync UI ↔ hardware |
