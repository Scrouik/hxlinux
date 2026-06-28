# Analyse scroll → dump modèle (HX Stomp XL) — RÉSOLU

> Référence **v5 — juin 2026**. Remplace les v1 à v4, **toutes erronées** (voir « Historique des
> conclusions fausses » en fin de document). Le scroll dump **fonctionne** ; cause trouvée et
> corrigée, vérifiée sur trace runtime. Toute reprise du sujet part d'ici.

---

## Résultat

Tourner la molette modèle sur le HX Stomp XL → HXLinux émet un `OUT 1b` de pull (lane ed) → le
device **dumpe** le modèle (`IN 53` + bulk 272) → HXLinux extrait l'id et résout le nom. Trafic
léger (pas de relecture du preset entier). Validé cold-boot, un cran :

```
[couple] pull slot_bus=01 — editor_ed03_double vivant=64f2 ctr=6cbd
OUT 1b+f0 double=f2:64 ctr=6cbd
OUT 19 #1 double=f2:64
[ScrollModelPull] model → "cd01fe"; "Deranged Master"
```

---

## Cause racine (vérifiée sur données)

Le pull qui dumpe doit porter **DEUX compteurs issus de la session ed03 vivante**, pas figés :

1. **double cd:03** (octets 28–29 du `1b`/`19`) = `editor_ed03_double` **lu en direct** au moment
   du pull. Réutilisé tel quel par le `1b` ET les deux `19` (capture témoin d6eb2b1 : `eb/eb/eb`).
   Avance de `+3` **entre** deux pulls (eb → ee → f1 → f4).
2. **ctr** (octets 12–13 du `1b`/`19`) = part de **`0x6cbd`** (init `HelixState::new`, hérité
   d6eb2b1), avance `+0x4b` après le `1b`, `+0x31` après chaque `19`. Jamais réécrasé.

Le refactor « pipeline réactif » avait **découplé les deux** :
- double snappé une seule fois dans un champ dédié `hw_model_pull_ed03_double` (puis +1 figé),
- ctr cloué à une constante morte `0x1c7e`.

Résultat : le device voyait un pull incohérent avec l'état de SA session → réponse `IN 08` ack nu
(`16 03`) + `IN 21`, **jamais le dump**.

### Preuve décisive (run témoin d6eb2b1 + runs de correction, juin 2026)

| Run | double | ctr | Résultat |
|-----|--------|-----|----------|
| d6eb2b1 (témoin cold boot) | `eb:64` (vivant) | `0x6cbd` (vivant) | **DUMPE** (cd01fe, Minotaur, Teemah!, Heir Apparent sur 4 crans) |
| Actuel, sans flag | `f3:64` (figé snap+1) | `0x1c7e` (figé) | `21`, pas de dump |
| Actuel, HX_PULL_DOUBLE_DELTA=-2 | `f1:64` (=HX) | `0x1c7e` (figé) | `21` — **double aligné mais ctr faux** |
| HX_PULL_COUPLE_LANE=1, ctr non init mod.rs | `f2:64` (vivant) | `0x0000` (non init) | `21` — **double bon mais ctr faux** |
| HX_PULL_COUPLE_LANE=1 + mod.rs ctr=0x6cbd | `f2:64` (vivant) | `0x6cbd` (vivant) | **DUMPE** (cd01fe / Deranged Master) |

Les trois échecs intermédiaires montrent qu'aligner **une seule** des deux variables ne suffit
jamais. Il faut les deux, vivantes, simultanément. C'est ce qui a fait croire (v4) à une « cause
device invisible » : chaque test n'isolait qu'une variable.

### Détail important
`f2:64` a dumpé alors que d6eb2b1 utilisait `eb:64`. Le device n'exige **pas** une valeur absolue
précise — seulement une valeur **cohérente avec la session ed03 vivante**. Le correctif est donc
robuste face aux variations de bootstrap (PHASE B laisse le double à `0x64f2` ; d6eb2b1 à `0x64eb` ;
les deux marchent).

---

## Correctif (mode couplé)

Fichier `helix/scroll_model_pull.rs`, derrière le flag `HX_PULL_COUPLE_LANE=1` :

- `build_pull_1b` / `build_pull_19` : en mode couplé, lisent `editor_ed03_double` vivant et le
  réutilisent (pas d'incrément intra-pull) ; n'écrivent jamais le champ figé.
- `finalize_pull_capture` : avance `editor_ed03_double` de `+3` pour le pull suivant.
- `handle_in_layer_trigger` : ne snappe rien, ne force pas le ctr.

Fichier `helix/mod.rs`, `HelixState::new()` :
- `hw_model_pull_ctr: 0x6cbd` (base ctr du pull scroll, brute au 1er pull, comme d6eb2b1).

Sans le flag : comportement figé hérité conservé (témoin).

---

## FAITS établis (vérifiés sur données)

- **FAIT A** — Le device dumpe sur le pull si double + ctr sont des continuations de la session
  ed03 vivante. Vérifié : d6eb2b1 dumpe cold-boot ; correctif couplé dumpe cold-boot.
- **FAIT B** — Aligner une seule des deux variables ne dumpe jamais (trois runs d'échec ci-dessus).
- **FAIT C** — Le `21` (44 o) ne contient JAMAIS le model-id (tronque à `83 17`). Seul le dump
  (`IN 53` puis bulk 272) porte l'id (`19 <id> 1a`, ex. `cd01fe`). Vérifié sur toutes les captures.
- **FAIT D** — Recharger le preset entier sur scroll est rejeté (≈2932 o/cran, flood bus). Le pull
  ciblé est la bonne primitive (quelques centaines d'o).

---

## RÉFUTÉ définitivement

- **FAIT 4 (v4) « double innocenté, cause device invisible, escalade kempline »** → FAUX. La cause
  était accessible : double ET ctr découplés. Le test v4 (double seul aligné) échouait car le ctr
  restait figé.
- Abonnement f0 manquant (v1), timing ARM f0 (v2), double seul (v3) — tous des demi-vérités qui
  n'isolaient qu'une partie du mécanisme.

---

## À VÉRIFIER ENCORE (durabilité)

- **Multi-crans** : d6eb2b1 « devenait silencieux après quelques crans ». Tester 5–6 crans
  d'affilée cold-boot. Suspect si décrochage : la dérive du `+3` entre pulls (`editor_ed03_double`)
  ou le wrap `cd 03 → 04`. À diagnostiquer sur trace si ça se reproduit.
- Plusieurs slots / plusieurs presets, pour confirmer la robustesse avant de retirer le flag.

---

## Captures de référence

- `stomp_running_start_linux_d6eb2b1.json` — **témoin qui dumpe** (4 crans, modèles lus).
- Runs juin 2026 HEAD : sans flag (échec), DELTA=-2 (échec), couplé sans ctr (échec), couplé+ctr
  (DUMPE). Profil pull qui dumpe : `1b … ctr=bd:6c … 83 66 cd 03 f2 64 …`.
- `stomp_running_start_hxedit_one_notch.json` — HX, scroll qui dumpe (référence protocole).

---

## Historique des conclusions fausses (pour ne pas reboucler)

v1 : « abonnement f0 manquant » → réfuté v2 (présent, identique HX).
v2 : « timing ARM f0 » → réfuté v3 (HX_F0_ARM_EARLY testé, rien).
v3 : « double cd:03 f3 vs f1 » → réfuté v4 (aligné f1, toujours 21).
v4 : « cause invisible, inaccessible » → réfuté v5 (d6eb2b1 dumpe ; cause = double + ctr découplés).

**Leçon** : vérifier le témoin (run d6eb2b1 cold-boot) AVANT de théoriser. Et corriger les
variables couplées ENSEMBLE, jamais une à la fois — sinon chaque test échoue et fait croire à tort
à une impasse.