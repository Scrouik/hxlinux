# Analyse scroll → dump modèle (HX Stomp XL) — pourquoi ça ne dumpe pas

> Référence **définitive**, sourcée octet par octet sur captures HX Edit + traces HXLinux.
> Toute reprise du sujet part d'ici. Dernière révision : juin 2026 (FAIT 3 corrigé).

## Symptôme

Molette modèle → `IN 1d` puis `IN 1f` (lane `f0:03:02:10`) → HXLinux envoie `OUT 1b`
(pull, lane ed) → HX devrait dumper (`IN 53/54` + bulk). Chez nous : `IN 08` (ack nu)
+ `IN 21` (notif assignation), pull échoué, pas de dump.

---

## CAUSE : se joue AU MOMENT DU PULL, pas avant.

Le pull HX (qui dumpe) et le nôtre (qui ne dumpe pas) sont **identiques au bit près
SAUF l'octet 28 (le double)** :
```
HX  : 1b 00 00 18 80 10 ed 03 00 1d 00 04 7e 1c 00 00 01 00 06 00 0b 00 00 00 83 66 cd 03 f1 64 2d 65 81 62 01 00
nous: 1b 00 00 18 80 10 ed 03 00 38 00 04 7e 1c 00 00 01 00 06 00 0b 00 00 00 83 66 cd 03 f3 64 2d 65 81 62 01 00
                                ^cnt(sans objet)                                          ^^double HX=f1 nous=f3
```
Le device **acquitte** notre pull (`08 ... ed 03 80 10 ... 16 03`) mais ne dumpe pas.
HX, lui, répond directement par le dump `IN 53` qui **écho le double f1** (`cd 03 f1 67`).

➜ **Seule différence de contenu = le double cd:03 (f3 vs f1).** Lead principal restant.
On est +2 vs HX. Vient de l'avance de `editor_ed03_double` pendant PHASE B / lectures
preset. À TESTER (forcer le double scroll à la valeur attendue par le device).
NB : statut « toléré » des doubles = supposition NON vérifiée pour la lane scroll cd:03
(les lectures preset cd:01/02/04 tolèrent, mais cd:03 scroll peut être plus strict).

---

## FAIT 1 — Le pull est byte-identique à HX sauf le double. (cf. ci-dessus)

## FAIT 2 — Réponse device : HX → `IN 53` dump (écho double f1). Nous → `IN 08` ack nu.

## FAIT 3 (CORRIGÉ — l'ancienne version était FAUSSE)

**Ancienne hypothèse (RÉFUTÉE) :** « le device serait dans un état éditeur vivant
qui streame des `IN 1d` de fond en continu, et le scroll ne dumpe que dans cet état. »

**Réfutation décisive :** comptage des `IN 1d` (lane f0) sur deux captures HX :
- `stomp_running_start_hxedit.json` (bootstrap complet, **SANS scroll**) → **0** `1d` de fond.
- `stomp_running_start_hxedit_one_notch.json` (1 cran) → **41** `1d`.

Les `1d` de fond sont donc **produits par le geste de molette**, PAS un abonnement
spontané. Le différentiel « 916 vs 208 paquets » et le « flux continu » étaient un
**artefact du geste utilisateur** dans la capture. Le dump ne dépend d'AUCUN état de
fond — il se joue au pull (cf. CAUSE).

➜ Conséquence : les expériences **go-live** ET **ARM f0 précoce** testaient toutes
deux ce mécanisme inexistant. C'est pourquoi aucune n'a rien produit. Cibles erronées.

---

## TOUT ce qui est RÉFUTÉ / vérifié (ne plus explorer)

- Pull mal formé / lane → FAIT 1 (identique sauf double).
- Abonnement f0 manquant → présent dans connect.rs, identique HX.
- ARM f0 manquant → présent (amorcage), ACKé, identique HX.
- **ARM f0 envoyé trop tard** → testé `HX_F0_ARM_EARLY=1` : ARM précoce part, device
  l'ACK, **aucun changement** (pas de 1d, scroll toujours 21). RÉFUTÉ.
- go-live comme déclencheur → chronologie + FAIT 3. RÉFUTÉ.
- État « 1d de fond » prérequis → FAIT 3 corrigé. RÉFUTÉ.
- Requête de contrôle USB / SET_INTERFACE → HX n'en fait aucune.

---

## Prochaine étape

**Lead unique restant et concret : le double cd:03 du pull (f3 vs f1).**
- Inspecter `scroll_model_pull.rs` : d'où vient `hw_model_pull_ed03_double` (snap de
  `editor_ed03_double`), pourquoi +2 vs HX.
- Tester (flag-gardé) un pull avec le double aligné sur la valeur HX attendue.
- Critère succès : `IN 53/54` dump au lieu de `IN 08` ack nu.

Si ça ne dumpe toujours pas avec le bon double → la cause est un état device non
visible dans le flux bulk → question pour l'auteur kempline (le pull est par ailleurs
byte-identique, donc c'est de la connaissance firmware).

## Captures de référence

- `stomp_running_start_hxedit_one_notch.json` — HX, scroll qui dumpe (pull [195], dump [197]).
- `stomp_running_start_hxedit.json` — HX, bootstrap SANS scroll (0 background 1d → preuve FAIT 3).
- `stomp_running_start_linux_one_notch.json` — notre app (pull byte-identique sauf double, pas de dump).