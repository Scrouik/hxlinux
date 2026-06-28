# Analyse scroll → dump modèle (HX Stomp XL) — pourquoi ça ne dumpe pas

> Référence **définitive**, sourcée octet par octet sur captures HX Edit + traces HXLinux.
> Toute reprise du sujet part d'ici. Dernière révision : juin 2026 (double innocenté).

## Symptôme

Molette modèle → `IN 1d` puis `IN 1f` (lane `f0:03:02:10`) → HXLinux envoie `OUT 1b`
(pull, lane ed) → HX devrait dumper (`IN 53/54` + bulk). Chez nous : `IN 08` (ack nu
`16 03`) + `IN 21` (notif assignation), pull échoué, pas de dump.

---

## CONCLUSION (juin 2026) : le pull est byte-identique à HX, le device répond différemment.

Le test `HX_PULL_DOUBLE_DELTA=-2` a aligné notre double sur celui de HX (`f1`). Résultat :
notre `OUT 1b` est désormais **identique au bit près au pull HX qui dumpe**, sauf l'octet 9
(cnt, compteur de séquence sans objet) :
```
HX   : 1b 00 00 18 80 10 ed 03 00 1d 00 04 7e 1c 00 00 01 00 06 00 0b 00 00 00 83 66 cd 03 f1 64 2d 65 81 62 01 00
nous : 1b 00 00 18 80 10 ed 03 00 37 00 04 7e 1c 00 00 01 00 06 00 0b 00 00 00 83 66 cd 03 f1 64 2d 65 81 62 01 00
                                ^cnt (seule diff, sans objet)
```
**Et le device répond toujours `21` (pas de dump).**

➜ Le double est **INNOCENTÉ** (l'ancien lead). Émettre exactement les mêmes octets que HX
ne produit PAS le même comportement device. La cause est donc un **état interne du device**
mis en place par HX Edit mais pas par nous — **invisible dans le flux bulk** (ce n'est pas
*dans* le pull, c'est *dans le device* à l'instant du pull).

C'est la fin de ce qui est diagnosticable par comparaison de paquets. Prochaine action :
**question ciblée à l'auteur kempline** (voir `kempline-question.md`).

---

## FAITS établis (vérifiés sur données — ne pas re-tester)

- **FAIT 1** — Le pull OUT 1b est byte-identique à HX sauf cnt (octet 9) et — avant le test —
  le double (octet 28). Le test a éliminé le double : identité totale sauf cnt.
- **FAIT 2** — Réponse device : HX → `IN 53` dump (écho double). Nous → `IN 08` ack nu `16 03` + `IN 21`.
- **FAIT 3** — Les `IN 1d` de fond sont produits PAR le geste de molette, PAS un abonnement
  spontané. Preuve : capture HX sans scroll = 0 `1d` de fond ; 1 cran = 41. Le dump ne dépend
  d'aucun état de fond visible.
- **FAIT 4 (NOUVEAU)** — Double aligné sur HX (`f1`) → toujours `21`. Le double est innocenté.
  Le comportement diffère à octets identiques → état device non observable.

---

## TOUT ce qui est RÉFUTÉ / vérifié (ne plus explorer)

- Pull mal formé / lane → FAIT 1 (identique sauf cnt).
- Double cd:03 (f3 vs f1) → testé aligné f1, toujours 21. RÉFUTÉ (FAIT 4).
- Abonnement f0 manquant → présent dans connect.rs, identique HX.
- ARM f0 manquant / trop tard → présent, ACKé, testé précoce (`HX_F0_ARM_EARLY`), rien. RÉFUTÉ.
- go-live comme déclencheur → chronologie + FAIT 3. RÉFUTÉ.
- État « 1d de fond » prérequis → FAIT 3. RÉFUTÉ.
- Requête de contrôle USB / SET_INTERFACE → HX n'en fait aucune.

---

## Hypothèses restantes (état device non visible — pour kempline)

Le device distingue notre session de celle de HX par quelque chose qui N'EST PAS dans le pull :
1. Un **dialogue antérieur** (pendant ou après PHASE B) que HX fait et pas nous, qui « arme »
   le device à dumper sur le prochain 1b scroll. À chercher : un échange spécifique HX Edit
   entre le bootstrap et le 1er scroll qu'on n'aurait pas répliqué.
2. Un **registre/handle de session** côté device (lié au `7e:1c`, au `ctr`, ou à un état de
   « focus slot » qu'on n'aurait pas correctement établi) — le device saurait que « ce pull
   vient d'une session qui n'a pas fait X » et répondrait par la notif courte au lieu du dump.
3. Le `21` lui-même (`82 62 01 1a`) est peut-être une **réponse normale** que HX consomme puis
   relance, et le dump viendrait d'un 2e échange qu'on ne fait pas. À vérifier dans la capture
   HX one_notch : y a-t-il un OUT entre le `21` et le `53` ?

## Captures de référence

- `stomp_running_start_hxedit_one_notch.json` — HX, scroll qui dumpe (pull → IN 53).
- `stomp_running_start_hxedit.json` — HX, bootstrap SANS scroll (0 background 1d → FAIT 3).
- `stomp_running_start_linux_one_notch.json` + runs juin 2026 — notre app, pull byte-identique, pas de dump (FAIT 4).