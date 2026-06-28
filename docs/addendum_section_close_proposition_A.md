## 11. Proposition A (fermeture façon HX Edit) — TESTÉE, échec sur lane synthétique

> **Statut : ABANDONNÉE (matériel), code testé localement puis retiré / non commité
> (cf. §11.6 ; flag prévu `HX_PULL_CLOSE_LIKE_HXEDIT`, OFF par défaut).** On a implémenté et testé sur la pédale la fermeture de transaction
> à la HX Edit (`19#1` au `ctr` calculé → écho 68 o → `19#2`). Résultat sans appel :
> **l'écho n'arrive jamais** (le device refuse notre `19` sur lane synthétique) et le
> `19#1` orphelin **accélère le gel**. C'est le **mur du §5 qui revient** : tant que le `1b`
> n'est pas sur la lane vivante du device, le `19` est irrecevable. On **reste sur grab-53 +
> throttle** (§10), config validée stable. La formule de fermeture est juste et reste
> documentée ci-dessous pour un repreneur qui cracquerait le §5.
>
> **English:** [addendum_section_close_proposition_A.en.md](./addendum_section_close_proposition_A.en.md)
>
> **Complète :** §6 (gel post-dump), §10 (mitigation throttle). **Captures :**
> `stomp_running_start_hxedit_2_multi_notch.json` (référence HX), §10.6
> `stomp_running_start_linux_multi_notch.json` (grab-53 stable). **Run close-on :** logs
> terminal session dev (mai 2026), flags ci-dessous — pas de capture JSON archivée.

---

### 11.1. Ce qui a été tenté

Fermer chaque notch comme HX, pour supprimer la **cause racine** du gel (transactions ED03
jamais closes, §6/§10) au lieu de seulement plafonner leur rythme. FSM :

```
dump → lire dump[20] → 19#1 au ctr = (ctr du 1b) + dump[20] + 8
     → attendre écho 68 o (head 0x39, lane ed:03:80:10)
     → 19#2 au ctr +0x31 → finalize
```

Le modèle est émis **avant** le `19#1` (donc aucune régression d'affichage même si la
fermeture échoue). Abort propre si l'écho manque dans la fenêtre.

### 11.2. La formule du `ctr` des `19` — RÉSOLUE et vérifiée (acquis)

Contrairement au handoff §5/§11 qui la croyait hors de portée, le `ctr` des `19` **se
dérive** : le device place dans **l'octet 20 du dump** la longueur que l'host doit
acquitter.

```
delta(1b → 19#1) = dump[20] + 8        (vérifié 10/10 sur les deux captures HX)
delta(19#1 → 19#2) = 0x31              (constant)
double : +1 par OUT
```

Test déterminisme : même contenu de dump → même delta (modèle `cd0246` ×4 → toujours
`0x4d`). La variation apparente venait de modèles **différents** (deux dumps de 92 o →
`0x4c` vs `0x4b`). **La formule n'est PAS le point de blocage** — les logs du run close-on
le confirment, l'arithmétique tombe juste à chaque cran. Test unitaire :
`close_19_1_ctr_matches_hx_formula`.

### 11.3. Pourquoi ça échoue quand même : l'asymétrie `1b` / `19`

Le device traite les deux paquets **différemment** :

| Paquet | Rôle | Validation du `ctr` | Sur lane synthétique (`0x6cbd`) |
|--------|------|---------------------|---------------------------------|
| `1b` | *demande* de dump | **lâche** | dumpe ~92 % (grab-53 marche) |
| `19` | *continuation* de transaction | **stricte** (vraie lane interne) | **jamais d'écho** |

Le `1b` est servi même hors de la vraie lane ; le `19`, lui, est confronté à l'état de
transaction réel du device → sur notre lane synthétique, **pas d'écho 68 o**. C'est le
**mur du §5** appliqué à la fermeture : HX ferme parce que TOUTE sa session (`1b` + `19`)
est sur la lane vivante que le device suit ; notre `1b` synthétique nous interdit le `19`.

### 11.4. Trace matériel (run close-on)

```
model → "64"; "Minotaur"
[close] 19#1 ctr=6dc8 (=1b 6d84 + dump[20] 0x3c + 8) double=f7:64
[close] écho 68o absent → abort propre (pas de 19#2)        ← refus du 19
  …1b ctr=6df9 → pull échoué (pas de bulk assignable)        ← cran « Teemah » : reject §4
  …1b ctr=6e44
model → "cd0223"; "Heir Apparent"
[close] 19#1 ctr=6e92 (=1b 6e44 + dump[20] 0x46 + 8) double=fa:64
[close] écho 68o absent → abort propre
```

Deux symptômes, deux causes distinctes :

- **« Teemah » sauté** = reject ordinaire (§4, ~8 % bénin) : le `1b` au `ctr=6df9` n'a pas
  dumpé, l'UI saute un cran et se réaligne au suivant. **Pas un bug de code** — le mode
  close ne « voit » simplement pas ce cran.
- **Gel** = close **strictement pire** que grab-53. Chaque abort laisse une transaction
  **à moitié ouverte** (`1b` + `19#1` non échoué, jamais fermé) → empile *plus* d'état non
  clos que grab-53 (qui ne laisse que le `1b`) → sature et fige **plus vite**. Corrélation
  mesurée en §11.4.1.

#### 11.4.1. Corrélation mesurée (même throttle, close ON vs OFF)

Même protocole multi-cran rapide que §10.6 ; seul le flag close change. Repro close-on :

```text
HX_PULL_COUPLE_LANE=1 HX_PULL_CLOSE_LIKE_HXEDIT=1 HX_PULL_COALESCE_LAST=1
HX_PULL_SETTLING_MS=500 HX_SCROLL_PULL_DEBUG=1
```

| Run | `CLOSE` | Throttle | Pulls avant gel | Wrap `ff→00` | Gel |
|-----|:-------:|---------:|----------------:|:------------:|:---:|
| grab-53 (§10.6) | OFF | 500 ms | **24** (10 balayages) | franchi | **non** |
| proposition A | ON | 500 ms | ~8–12 (3–4ᵉ balayage) | non atteint | **oui** |

La trace §11.4 provient du run close-on (session dev, mai 2026) : l'arithmétique `ctr` tombe
juste à chaque cran tenté, mais chaque `[close] écho 68o absent → abort propre` laisse une
transaction de plus à moitié ouverte qu'en grab-53 pur.

### 11.5. Décision

- **`HX_PULL_CLOSE_LIKE_HXEDIT` reste OFF** (défaut). On retourne à grab-53 + throttle
  500 ms (§10), stable et sans gel.
- Le code de A (FSM, formule, détection d'écho, tests) a été **validé localement** mais
  **n'est pas sur la branche courante du dépôt** (cf. §11.6). Pour le réactiver utilement il
  faut d'abord **cracker le §5** (poser le `1b` sur la lane vivante du device) — alors la
  recette de fermeture du §11.2 s'applique telle quelle.
- **Consigne utilisateur (README)** : ne pas manipuler les commandes de la pédale pendant
  que l'éditeur est connecté ; faire les changements **depuis l'éditeur**. HX Edit autorise
  l'usage simultané, mais pour HX Linux ça n'a pas de sens au vu de la contrainte : la
  lecture live du modèle au scroll matériel n'est ni fiable ni sûre, et l'éviter supprime
  d'emblée le risque de gel.

### 11.6. État du code (reprise)

Le flag `HX_PULL_CLOSE_LIKE_HXEDIT`, la FSM de fermeture (§11.1) et le test unitaire
`close_19_1_ctr_matches_hx_formula` **ne sont pas présents sur la branche courante du
dépôt** (`fix/none-sur-3894283` au moment de la rédaction — `grep` à vide dans
`scroll_model_pull.rs`). L'implémentation a été **testée localement** puis **retirée /
non commitée** pour éviter d'activer par inadvertance une voie qui empire le gel. Un
repreneur qui veut rejouer A doit réintroduire le patch depuis son historique de session, ou
le réécrire à partir de la recette §11.1–11.2. **Ne pas merger** sans d'abord résoudre §5
(lane vivante).

---

*Synthèse : la fermeture façon HX était la bonne idée (et la formule du `ctr`, longtemps
réputée impossible, a fini par tomber : `dump[20] + 8`). Mais elle bute sur le même mur que
tout le chantier — sans le `1b` sur la lane vivante, le device refuse nos `19`, et les
tenter empire le gel. On s'arrête à un état suffisant et stable (grab-53 + throttle,
scroll matériel découragé côté utilisateur), et on parque A, prêt à servir si le §5 cède un
jour.*