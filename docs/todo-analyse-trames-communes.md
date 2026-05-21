# TODO — Analyse des trames communes (protocole HX)

**Contexte** : la connexion / keep-alive a été analysée en profondeur (handshake, `sub=08`, `ed→ef→f0`) parce qu’un blocage l’imposait. Le reste du protocole a surtout été couvert au minimum fonctionnel. Hypothèse : il existe encore des trames « sans importance métier » mais nécessaires pour maintenir le HW en état d’écoute / session éditeur — comme le poll `sub=08`.

**Problème mis de côté pour l’instant** : état semi-dysfonctionnel du HW après fermeture de HXLinux (séquence de fin / trames communes à identifier). *Le reset USB à la fermeture a été testé puis retiré (mai 2026) — aggravait l’accès au device au redémarrage de l’app.*

---

## 1. Captures à réaliser (matrice de scénarios)

Fichiers JSON dans `src/Paquets Json/`, un scénario par capture, tag explicite dans le nom :

| # | Scénario | Nom fichier suggéré |
|---|----------|---------------------|
| 1 | Connexion complète | `…_connect.json` (réf. HX Edit + Linux déjà existants) |
| 2 | Idle 1–2 min (rien toucher) | `…_idle.json` |
| 3 | Changement slot hardware | `…_slot_hw.json` |
| 4 | Changement preset UI | `…_preset_ui.json` |
| 5 | Lecture nom preset | `…_preset_name.json` |
| 6 | Live write (un paramètre) | `…_live_write.json` |
| 7 | `probe_slot_model_usb` / assign (si utilisé) | `…_slot_model.json` |
| 8 | Fermeture app **sans** débrancher (HX Edit puis HXLinux, plus tard) | `…_close_app.json` |

Pour chaque capture : noter durée, actions exactes, et si possible **HX Edit** vs **HXLinux** en parallèle (même scénario).

---

## 2. Méthode — trouver la trame commune « invisible »

1. Extraire **tous les OUT** (16 o et autres longueurs fixes) de chaque JSON.
2. **Normaliser** les octets dynamiques connus (compteurs, `session_no`, queue preset) → masque `??` sur ces positions.
3. **Intersection** : motifs OUT présents dans **tous** les scénarios (ou toutes les ~N secondes en idle).
4. **Soustraire** ce qu’on émet déjà (`subscribe`, bootstrap phase 4, `ed→ef→f0`, poll `sub=08`, etc.).
5. Classer le reste par :
   - **périodicité** (~1 s, ~28 ms, événementiel) ;
   - présence en **idle** ;
   - **disparition** à la fermeture app (capture dédiée).

**Critères candidat « maintien / écoute »** :

- présent même quand l’UI ne fait rien ;
- répété régulièrement ;
- souvent un octet **`sub`** / lane (08, 10, 04…) plutôt qu’une opcode exotique ;
- pas une trame unique : parfois un **triplet** ou une **réponse IN** obligatoire.

**Piège** : ne pas chercher une seule trame magique — comparer aussi les **IN** auxquels HX Edit répond et nous non.

---

## 3. Suite technique (quand les captures existent)

- [ ] Script ou procédure sur les JSON : intersection des motifs OUT normalisés.
- [ ] Tableau opcode / `sub` / période / présence par scénario (à intégrer au recap ou doc protocole).
- [ ] Diff HX Edit vs HXLinux sur scénario **idle** en priorité.
- [ ] Reprendre le sujet **fermeture session** avec capture `…_close_app.json` (HX Edit).

---

## 4. Références

- [recap-keep-alive-ed-ef-f0-mai-2026.md](recap-keep-alive-ed-ef-f0-mai-2026.md) — leçon `sub=08`, timing, compteurs.
- [usb-slot-model-fixes.md](usb-slot-model-fixes.md) — chantier slot/modèle (distinct).
- `src/Paquets Json/connect_device_30s_HXEdit.json` — référence connexion.

---

*Créé mai 2026 — plan d’analyse proposé en session chat.*
