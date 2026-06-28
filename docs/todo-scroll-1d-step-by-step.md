# Debug `1d` Step-by-Step (base `ac6cfb9`)

Objectif: identifier **exactement** la touche qui fait disparaître les `IN 1d` sur le Stomp.

Principe: modifier **une seule chose** à la fois, capturer 15s, mesurer `IN 1d`.

## Base de reference

- Commit de depart: `ac6cfb9`
- Capture courte: `captures/usb-wireshark/stomp_running_start_hxlinux.json`
- Commande d'analyse:

```bash
python3 scripts/analyze_stomp_running_compare.py captures/usb-wireshark/stomp_running_start_hxlinux.json
```

## Precondition importante (decouverte)

- Les `1d` ne sont **pas** garantis juste apres bootstrap.
- Le flux `1d` semble apparaitre quand le Stomp est passe en **etat modification/edition**.
- Observation cle:
  - certaines captures HX Edit recentes montrent aussi `IN 1d = 0`;
  - donc `0 x 1d` n'implique pas forcement un bug code host, si la precondition n'est pas remplie.

## Regle de test

- Procedure identique a chaque fois:
  - Stomp ON
  - lancer app
  - capturer ~15s
- Pour les tests "presence de `1d`":
  - ajouter une action qui met explicitement le Stomp en mode modification/edition
    (commande ou interaction equivalente), puis mesurer.
- Critere:
  - `IN 1d > 0` => dialogue vivant
  - `IN 1d = 0` => dialogue casse
  - **seulement** si la precondition "mode modification" est satisfaite.

## Etapes (micro-touches)

### Etape 0 - Baseline `ac6cfb9`

- Statut: **OK (observe)**
- Observation:
  - capture ~28s: `IN 1d = 232`, `OUT f0/08 = 221`, ratio ~0.953
  - le dialogue `1d` est bien vivant sur cette base.

### Etape 1 - Retirer ARM f0 tot (Connect)

- Fichier: `src-tauri/src/helix/modes/connect.rs`
- Action: enlever `IN 11 x2 -> OUT 08 f0 ... 09:10 (+15ms)`.
- Statut: **KO (observe)**
- Resultat observe:
  - capture ~15.8s: `IN 1d = 0`, `OUT f0/08 = 0`, aucun `1d/1f` fond
  - ARM `09:10` non detecte dans la fenetre test.
- Conclusion: l'ARM f0 tot est un declencheur fort.

### Etape 2 - Remettre ARM f0 tot, garder `19` anticipe retire

- Fichier: `src-tauri/src/helix/modes/connect.rs`
- Action:
  - remettre bloc ARM f0 tot sur `IN 11 x2`;
  - laisser supprime le bloc `IN 08 ... cnt=03 -> OUT 19`.
- But: verifier si les `1d` reviennent **sans** le `19` anticipe.
- Statut: **KO (observe)**
- Resultat observe:
  - capture ~15.6s: `IN 1d = 0`, `OUT f0/08 = 1`, `OUT f0/10 = 12`
  - `ARM_f0` present (ex: #269) mais le dialogue `1d` ne demarre pas.
- Conclusion:
  - l'ARM f0 tot seul ne suffit pas dans cette base;
  - le bloc `19` anticipe (ou son contexte immediat) semble necessaire pour relancer les `1d`.

### Etape 3 - Si `1d` reviennent, tester impact du `19` anticipe

- Reintroduire uniquement le `19` anticipe.
- Comparer:
  - delai 1er `1d`,
  - nombre total `1d`,
  - ratio `OUT f0/08` / `IN 1d`.
- Statut: **KO (observe)**
- Variante testee:
  - ARM f0 tot remis
  - `19` anticipe remis (sur `IN 08 ... cnt=03`)
- Resultat observe:
  - capture ~16.2s: `IN 1d = 0`, `OUT f0/08 = 1`, `OUT f0/10 = 12`
  - phase4 (`19`/`1a`) visible, mais aucun `1d/1f` fond.
- Conclusion:
  - remettre le `19` anticipe ne suffit pas non plus dans l'etat courant.
  - la difference qui maintenait les `1d` est probablement ailleurs que ces 2 blocs seuls
    (contexte d'etat connect/reconfigure/listener a verifier).

### Etape 4 - Introduire progressivement des briques Pipeline

- Une touche par capture, dans cet ordre:
  1. gate post-`ARM_ef`,
  2. enchainement `ReconfigureX1` / `amorcage`,
  3. (optionnel, secondaire) chemin ACK via `firmware_scroll_ack` / `usb_in_pipeline`.
- Stopper des que `IN 1d` tombe a 0.

### Etape 5 - Test minimal sur Pipeline pur

- Base: `80d884d` (Pipeline pur)
- Touche unique:
  - remettre `ARM_f0` tot dans `connect.rs` sur `IN 11 x2` (+15ms),
  - sans toucher le reste du Pipeline.
- Statut: **KO (observe)**
- Resultat observe:
  - capture ~15.6s: `IN 1d = 0`, `OUT f0/08 = 2`
  - ARM observes: `ARM_ed` -> `ARM_f0` -> `ARM_f0` -> `ARM_ef`
- Conclusion:
  - le fix minimal "ARM f0 tot seul" ne suffit pas dans Pipeline;
  - il existe au moins une autre condition bloquante (etat/timing/cinematique).

## Journal de resultats

Renseigner apres chaque run:

- Date/heure:
- Variante:
- Duree capture:
- `IN 1d`:
- `OUT f0/08`:
- Note timing (1er `1d`, ARM observes):

Ce journal permet de revenir rapidement a la derniere variante "vivante".

### Journal rapide (rempli)

- 2026-05-28 - Etape 0 (baseline `ac6cfb9`) - ~28s
  - `IN 1d = 232`, `OUT f0/08 = 221` - vivant
- 2026-05-28 - Etape 1 (sans ARM f0 tot) - ~15.8s
  - `IN 1d = 0`, `OUT f0/08 = 0` - casse
- 2026-05-28 - Etape 2 (ARM f0 tot oui, `19` anticipe non) - ~15.6s
  - `IN 1d = 0`, `OUT f0/08 = 1` - casse
- 2026-05-28 - Etape 3 (ARM f0 tot oui, `19` anticipe oui) - ~16.2s
  - `IN 1d = 0`, `OUT f0/08 = 1`, `OUT f0/10 = 12` - casse
- 2026-05-28 - Etape 5 (Pipeline pur + ARM f0 tot seul) - ~15.6s
  - `IN 1d = 0`, `OUT f0/08 = 2`, ARM `ed -> f0 -> f0 -> ef` - casse
- 2026-05-28 - Capture HX Edit recente (`stomp_running_start_hxedit.json`) - ~31s
  - `IN 1d = 0` aussi
  - interpretation: la session n'etait probablement pas en mode modification.

## Hypotheses (retenues / ecartees)

### Retenues (fortes)

- **Cinematique connect/reconfigure/armement**:
  - les tests montrent que les `1d` disparaissent rapidement des qu'on change la sequence
    d'armement autour de `IN 11 x2` et du bootstrap.
- **Timing d'ouverture du dialogue scroll**:
  - le Stomp semble sensible au moment exact ou le host "ouvre" le canal (`09:10` / sequence associee).
- **Precondition sessionnelle**:
  - les `1d` semblent dependre d'un etat interne "modification/edition" du Stomp.
  - sans cet etat, meme HX Edit peut rester a `0 x 1d`.

### Ecartees (ou secondaires)

- **Pull modele de `slot_model_hw_pull` (`1f` -> `1b/19/272`)**:
  - ce bloc sert surtout au scroll materiel de changement de modele;
  - ce n'est probablement pas la cause directe du `fond 1d` au demarrage.

### Point de vigilance (a ne pas confondre)

- Le chemin d'ACK (`1d`/`1f`) ne cree pas les `1d`:
  - il ne fait que repondre quand un `1d` est deja recu.
- Donc si `IN 1d = 0`, la cause primaire est en amont (cinematique/timing d'armement),
  pas dans le module d'ACK.
- Le chemin ACK (direct listener vs pipeline) reste un sujet de robustesse de reponse,
  mais **pas** l'hypothese principale pour la presence initiale des `1d`.
