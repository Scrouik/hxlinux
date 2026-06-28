# KemplineCorrection

Ce document suit les corrections apportees dans HXLinux par rapport au comportement observe dans Kempline.
Objectif: garder une trace claire, partageable, et incrementale des divergences/problemes et des correctifs.

## Regles de mise a jour

- Ajouter une entree par correction significative.
- Garder les sections courtes: symptome, cause, correction, impact.
- Noter explicitement si le comportement Kempline est identique (bug herite) ou corrige.
- Ajouter les captures/logs de reference quand necessaire.

---

## 2026-04-16 - Liste des presets decalee (cas preset 8)

### Contexte

- Symptomatique observee dans HXLinux: a partir d'un preset manquant (notamment le `8`), les noms suivants etaient decales.
- Exemple: le nom attendu du preset `8` etait absent, et le nom du preset `9` apparaissait a la place.
- Meme comportement observe dans Kempline (verification utilisateur), donc bug non specifique a HXLinux.

### Cause racine identifiee

- Le parseur de `request_preset_names` utilisait une hypothese de longueur fixe de record (`25` octets), inspiree de Kempline.
- Les captures USB (`usb.capdata`) montrent que les records `81 cd 00` sont en pratique a longueur variable.
- Certains noms sont coupes entre deux paquets USB (ex: `Aqua Sledge 2x10`), ce qui casse le decoupage fixe et induit des glissements.

### Corrections appliquees dans HXLinux

1. Parseur `request_preset_names` converti en decoupage "marqueur a marqueur":
   - record commence a `81 cd 00`
   - record se termine au prochain `81 cd 00`
2. Gestion explicite des records incomplets entre paquets:
   - en parse non-final (`finalize=false`), conservation du record partiel pour le paquet suivant
   - en parse final (`finalize=true`), finalisation sur le buffer restant
3. Extraction du nom adaptee au format observe:
   - recherche de `6d`
   - lecture du nom apres `6d xx` jusqu'au premier `00`

### Impact

- La liste des presets est redevenue correcte sur le cas teste (y compris autour du preset `8`).
- Le decalage en cascade (9->8, 10->9, etc.) n'apparait plus avec la correction en place.

### Statut vs Kempline

- Kempline: comportement decale reproduit.
- HXLinux: corrige sur la base des captures USB et du parser a longueur variable.

---

## Prochaine entree (template)

Copier/coller ce bloc pour la prochaine correction:

### YYYY-MM-DD - Titre court

- **Contexte**:
- **Cause racine**:
- **Correction appliquee**:
- **Impact**:
- **Statut vs Kempline**:

