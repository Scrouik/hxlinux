# Scroll → dump modèle (HX Stomp XL) — ADDENDUM au handoff (juin 2026)

> Cet addendum **met à jour**
> [Scroll_model_pull_handoff.md](./Scroll_model_pull_handoff.md). Il fige la compréhension
> validée sur matériel après le passage en **mode grab-53**. Il **corrige** les sections
> §0 (statut), §5 (règle du `ctr`), §6 (freeze) et §12 (décision opérationnelle) du
> handoff. Tout ce qui n'est pas contredit ici reste valable.
>
> **Handoff d'origine (historique) :**
> [Scroll_model_pull_handoff.md](./Scroll_model_pull_handoff.md) ·
> [Scroll_model_pull_handoff.en.md](./Scroll_model_pull_handoff.en.md)
>
> **English:** [scroll_model_pull_handoff_addendum.en.md](./scroll_model_pull_handoff_addendum.en.md)
>
> **Statut révisé : 1-notch RÉSOLU et stable.** Multi-notch rapide : **MITIGÉ** (coalescing +
> throttle 500 ms — cf.
> [Addendum_section_gel_multinotch.md](./Addendum_section_gel_multinotch.md) §10).
>
> **Compléments :**
> [§9 décrochage parseur](./addendum_section_decrochage_38.md) ·
> [§9 EN](./addendum_section_decrochage_38.en.md) ·
> [§10 gel multi-cran](./Addendum_section_gel_multinotch.md) ·
> [§10 EN](./Addendum_section_gel_multinotch.en.md)

---

## 1. Ce qui a débloqué le problème : on ne lit QUE le `IN 53`

On n'a besoin que du **chainHex** du modèle, et il est **entièrement dans le `IN 53`**
(la trame de dump, ~84-116 o, motif `… 19 <id> 1a …`). Le bulk `272` ne porte que les
**paramètres** du modèle — inutiles ici.

Le mode **grab-53** :
```
IN 1f → OUT 1b (+ OUT 08 interstitiel f0) → IN <dump> → extraire chainHex → FINI.
        On n'envoie JAMAIS les 19, on ne poursuit JAMAIS le 272.
```

**Effet, vérifié sur capture + runs :** supprimer la traîne `19/272` a réglé **les deux**
murs d'un coup :
- **plus de freeze** (le hardware reste vivant sur des dizaines de crans) ;
- **les rejects intermittents s'effondrent** (de ~50 % à ~3-8 %).

La traîne `19/272` était donc bien la cause commune : elle « empoisonnait » la session
ED03 (transaction incomplète côté host + retard ~2,3 s du device → collision du cran
suivant). En s'arrêtant au `53`, le device est libre à chaque `1f`.

---

## 2. CORRECTION du §5 — c'est une CONTINUATION, pas une « page »

Le handoff §5 concluait « le device ne dumpe qu'en page `0x6c`, jamais en page `0x1c` ».
**C'est faux / trop étroit.** Run 1-notch (~38 crans) : le `ctr` du pull est monté

```
0x6cbd → 0x6d08 → … → 0x7794      (+0x4b par 1b émis, UN seul 1b par notch en grab-53)
```

soit **8 pages traversées** (`6c, 6d, 6e, 6f, 70, 71, … 77`), **en dumpant tout du long**.

➜ Règle corrigée : le device accepte le pull tant que son `ctr` est une **continuation
monotone à partir d'une graine valide** (`0x6cbd` posée une fois par session). Ce n'est PAS
une valeur de page précise. La valeur `0x1c7e`/lane vivante de HX rejetait chez nous parce
qu'elle n'était pas une continuation de NOTRE session, pas à cause de la page.

- **double** `cd:03` : `+1` par OUT, `hi` figé `0x64`, wrap `cd 03→04` au passage du `lo`
  au-dessus de `0xff` (observé : log `double wrap bas → cd lane 04` au pull `ctr=0x708c`).
- Pas de plafond de page observé jusqu'à `0x77`.

---

## 3. Anatomie 1-notch validée

**Cycle sain** (≈92 % des crans) :
```
IN 1d pré-scroll (lane scroll avancée, SANS ACK)
  → IN 1f trigger
  → OUT 1b + OUT 08 (interstitiel f0)
  → IN <dump> (head variable 53/54/56/4c/4e/6c, len 84-116) → chainHex extrait immédiatement
  → IN 21 (notif d'assignation POST-dump, ignorée)
  → IN 1d (ACKé)
```

> **Point clé sur le `21`** : sur un cran réussi, le `IN 21` arrive **APRÈS** le dump — ce
> n'est PAS un reject, c'est la notif d'assignation hardware. Ce n'est un *reject* que
> lorsqu'il arrive **AVANT tout dump** (étape 1). Ne pas confondre les deux (le handoff et
> les vieilles analyses appelaient « 21 » les deux cas).

**Cycle reject** (≈8 %, bénin) :
```
IN 1f → OUT 1b → IN 21 (avant tout dump) → abort propre (aucune transaction pendante)
   → ce cran n'est pas lu → l'UI garde le modèle précédent → le cran SUIVANT réaligne
```

---

## 4. Caractérisation du reject 1-notch (vérifiée sur logs)

- **Sporadique, ~8 %** (3 rejects sur 38 pulls dans le run de référence).
- **Indépendant de la cadence** : les 3 rejects sont survenus à rythme normal
  (~900-1100 ms entre crans), **pas** en scroll rapide. C'est l'intermittence device
  résiduelle (état ED03 interne non observable), pas un problème de timing host.
- **Aucun dump récupérable après le `21`** : seuls des `1d` suivent. Le device n'a
  simplement pas dumpé CE `1b`. Pour récupérer le cran, il faudrait **réémettre un `1b`
  frais** (cf. §6, piste « retry-on-reject » — non encore implémentée).
- **Conséquence UI** : un cran de retard, auto-corrigé au cran suivant. C'est le « décalage
  qui se réaligne » observé autour du notch 33.

---

## 5. Chiffres de référence

| Run | Pulls | Dumps | Rejects | Freeze | Notes |
|-----|-------|-------|---------|--------|-------|
| 1-notch (cadence lente, ~52 s) | 38 | 35 (~92 %) | 3 (propres) | 0 | `ctr` 0x6cbd→0x7794 |
| multi-notch (pbUi) | 31 | 29 (~94 %) | 1 | 0 | + ~5 `1f` perdus en scroll rapide |

---

## 6. Ce qui reste / compléments

1. **(1-notch) retry-once-on-reject** — sur `IN 21` à l'étape 1, réémettre **un seul** `1b`
   frais (continuation normale `ctr`/double). Le device dumpe très probablement au 2ᵉ essai
   → cran récupéré, plus de décalage visible. Peu risqué en grab-53 (pulls dos-à-dos OK).
   *Statut : proposé, non implémenté.* (Note : depuis les captures juin 2026, le `IN 21` **avant**
   dump n'est plus traité comme reject — cette piste est probablement **obsolète** ; à revalider
   avant implémentation.)
2. **(multi-notch) coalescing du `1f` pendant le settling** — *anciennement « conçu, non
   implémenté » ici.* **→ Remplacé et détaillé dans
   [Addendum_section_gel_multinotch.md](./Addendum_section_gel_multinotch.md) §10** :
   coalescing défaut ON (`HX_PULL_COALESCE_LAST=0` pour l'ancien comportement), throttle
   500 ms, `tick_hw_model_pull` depuis `usb_listener`. *Statut : **implémenté et validé**
   (24 pulls / 10 balayages, 0 gel ; gel device mitigé, pas guéri).*

Le point 1 reste une amélioration **host-side** optionnelle. Le point 2 est couvert par le
complément §10 (sans traîne `19/272`).

---

## 7. État du code (rappel)

- `helix/scroll_model_pull.rs` — mode grab-53 derrière `HX_PULL_COUPLE_LANE=1`.
  - Graine : `double = editor_ed03_double` vivant, `ctr = 0x6cbd` ; puis `+0x4b`/1b.
  - `ingest_pull_capture` : finalise sur le premier dump (extraction chainHex), **sans `19`**.
  - `send_pull_both_19s` conservée `#[allow(dead_code)]` (réf. handshake complet si un jour
    on veut les paramètres `272`).
  - Abort propre sur `IN 21` étape 1.
- Debug : `HX_SCROLL_PULL_DEBUG=1`, `HX_INIT_TRACE=1`.

## 8. Captures de référence

- `stomp_running_start_hxedit_one_notch.json` — HX (référence protocole).
- `stomp_running_start_linux_multi_notch_pbUi.json` — grab-53 multi-notch (29/31 dumps, 0 gel).
- Run 1-notch (logs) — 35/38 dumps, 3 rejects propres, 0 gel, `ctr` 0x6cbd→0x7794.

---

*Synthèse : la lecture du modèle au scroll est PASSÉE de « suspendue / instable / gel » à
« 1-notch stable, zéro gel » (§9 : parseur) et « multi-cran rapide mitigé » (§10 :
coalescing + throttle). La traîne 19/272 n'était pas nécessaire au chainHex. Reste surtout
le retry-on-reject (§6.1, probablement obsolète) et la fermeture façon HX (§10.3, bloquée
sur le `ctr` des `19`).*