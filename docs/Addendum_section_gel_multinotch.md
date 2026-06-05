## 10. Gel en scroll multi-cran rapide : saturation ED03 (MITIGÉ par throttle)

> **Statut : MITIGÉ, pas guéri.** En scroll multi-cran *rapide*, l'UI se figeait après
> 3-4 balayages. Ce n'est ni le bug parseur (§9) ni un plafond de `ctr` : c'est une
> **saturation de l'ED03** par accumulation de transactions de pull jamais fermées (grab-53
> n'envoie aucun `19`). Le déclencheur est le **rythme**, pas le nombre de crans. Mitigation
> 100 % host-side : **coalescing vers le dernier cran + throttle** du rythme des pulls.
> Validé : 24 pulls / 10 balayages, wrap `ff→00` franchi, **0 gel**. Le vrai remède (fermer
> la transaction comme HX Edit) reste bloqué sur la règle du `ctr` des `19` (cf. §5, §11).
>
> **English:** [Addendum_section_gel_multinotch.en.md](./Addendum_section_gel_multinotch.en.md)
>
> **Remplace** le point 2 de
> [scroll_model_pull_handoff_addendum.md](./scroll_model_pull_handoff_addendum.md) §6
> (« coalescing — conçu, non implémenté »).
>
> **Captures :** `stomp_running_start_linux_multi_notch.json` (runs Linux), 
> `stomp_running_start_hxedit_2_multi_notch.json` (référence Windows).
> **Code :** `scroll_model_pull.rs` (`coalesce_last_enabled`, `post_pull_settling_ms`,
> `tick_hw_model_pull`), `usb_listener.rs` (appel du tick à chaque IN).

---

### 10.1. Symptôme et preuve que le device est vivant

L'UI se fige après quelques balayages rapides. Dans la capture, **après le dernier pull la
pédale continue de répondre aux keep-alives** sur toutes les lanes (ed/ef/f0) pendant
plusieurs secondes, mais n'émet plus aucun `1d`/`1f` de scroll. C'est la signature exacte du
gel documenté au handoff §6 : boucle keep-alive vivante, sous-système scroll mort (qu'un
appui bouton sur la pédale débloque). Le gel est donc **interne au device**, déclenché par
notre trafic — pas un plantage hôte.

### 10.2. Cause : transactions ED03 non fermées qui s'accumulent

grab-53 lit le dump puis **s'arrête** : aucun `19` de fermeture. Chaque pull laisse donc une
transaction ED03 ouverte côté device, qui ne se draine que lentement (lag d'écho ~2,3 s,
handoff §6). En scroll *lent* (~1 pull/s) chaque transaction a le temps de se vider → la
session tient (38 crans, §9). En scroll *rapide*, les pulls tombent toutes les ~80-110 ms :
les transactions ouvertes s'empilent plus vite qu'elles ne se drainent, la fenêtre interne
sature, et le scroll se fige.

➜ **Le déclencheur est le débit de pulls, pas leur nombre.** Mesuré sur le run qui gèle :
jusqu'à **8 pulls dans une fenêtre glissante de 2,3 s** (intervalles `103, 93, 93, 110, 77,
97` ms). Le run lent qui survivait à 38 crans était à ~1 pull/s.

### 10.3. Ce que fait HX Edit (et pourquoi lui ne sature jamais)

Référence Windows, par cran : `1b → dump modèle → (21) → 19#1 → écho 68 o (head 39) → 19#2`.
Deux `19` **légers** + échos de 68 o, **aucun `272`** pendant le scroll. HX **ferme** donc
chaque transaction, à 1:1 et sans blackout de settling → sa lane ED03 ne s'accumule jamais,
quel que soit le rythme.

Le vrai correctif (proposition A) serait de répliquer cette fermeture. Il reste **bloqué** :
le `ctr` des `19` est un compteur de position **lié au contenu du dump** (mesuré : pas
`1b→19#1` = `0x46/0x44/0x4c/0x64/0x4b/0x4d` pour des dumps de 88/84/92/116/92/96 o, et le
device ne le ré-encode pas dans le dump). Impossible à reconstruire à ±0 sans specs Line 6 —
c'est le mur du §5 appliqué à la fermeture. Envoyer un `19` au `ctr` deviné rejouerait le gel.

### 10.4. La mitigation : coalescing + throttle (host-side pur)

Sans toucher au device (aucun paquet nouveau), on **plafonne le rythme** des pulls :

- **Coalescing** : un `1f` reçu pendant la fenêtre de settling n'est plus jeté mais mémorisé
  (le DERNIER gagne, `hw_model_pull_pending_slot_bus`). Un seul pull différé part en fin de
  settling, via `tick_hw_model_pull` appelé depuis `usb_listener` **à chaque IN** (seul point
  qui « bat » hors capture). On lit toujours le modèle FINAL du balayage. Le tick **simule
  l'avance lane** du `1f` manquant (`advance_firmware_scroll_lane(0x1f)`) avant d'émettre le
  `1b` différé — même contrat que le pull immédiat dans `handle_in_layer_trigger`.
- **Throttle** : la fenêtre de settling sert de pas de cadence. À 500 ms, on ne dépasse plus
  ~2-3 transactions en vol → sous le seuil de saturation.

### 10.5. Seuil device mesuré

En balayant `HX_PULL_SETTLING_MS` sous scroll rapide soutenu :

| `HX_PULL_SETTLING_MS` | Résultat |
|----------------------:|----------|
| 50 (défaut historique) | gèle (3-4 balayages) |
| 300 | gèle encore |
| 500 | **stable** |

➜ La fenêtre de drainage ED03 du device est **~300-400 ms**. On fige `500 ms` par défaut
(~1,3-1,6× de marge). C'est le compromis réactivité/stabilité : l'UI se cale sur le modèle
final ~0,5 s après l'arrêt de la molette.

### 10.6. Résultat

| Run | Throttle | Pulls | Wrap `ff→00` | Gel |
|-----|---------:|------:|:------------:|:---:|
| multi-cran rapide | 50 ms | ~12 (gel au 3-4ᵉ balayage) | non atteint | **oui** |
| multi-cran rapide | 500 ms | **24** sur 10 balayages | franchi | **non** |

### 10.7. État du code

- `coalesce_last_enabled()` : **défaut ON** (`HX_PULL_COALESCE_LAST=0` pour l'ancien comportement).
- `post_pull_settling_ms()` : **500 ms** par défaut si coalescing actif, 50 ms sinon ;
  `HX_PULL_SETTLING_MS=<n>` override prioritaire (tuning / mesure du seuil).
- `tick_hw_model_pull(state)` : tire le pull différé en fin de settling, appelé depuis
  `usb_listener` à chaque IN, indépendamment de la capture en cours.
- Le tout reste sous `HX_PULL_COUPLE_LANE` (la fonctionnalité pull scroll elle-même).

---

*Synthèse : le gel multi-cran n'était pas un nouveau mystère mais le gel device connu (§6),
atteint plus vite parce qu'en scroll rapide on empile des transactions ED03 jamais fermées.
Faute de pouvoir les fermer (ctr des `19` hors de portée, §5), on plafonne leur rythme :
coalescing + throttle 500 ms, host-side, zéro risque device. Scroll multi-cran stable. La
fermeture façon HX Edit (proposition A) reste la cible si le `ctr` se débloque un jour.*