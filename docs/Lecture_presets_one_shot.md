# HX Linux — Particularité de la lecture des presets

> **En une phrase** : la lecture complète des presets est un **amorçage one‑shot**. Le dump intégral (272×N) et la liste des 125 noms ne se lisent **qu'une seule fois par connexion USB**, dans une séquence figée. Ensuite l'éditeur vit en mode `Standard` et ne fait plus que des lectures **ciblées** (corps du preset actif au changement). Conséquence directe : un échec dans cette unique séquence est **fatal pour toute la session** — il n'y a pas de re‑dump automatique.

## 1. La séquence, et ce qui n'arrive qu'une fois

Chaîne d'amorçage (une exécution par branchement) :

```
Connect → ReconfigureX1 → amorcage (phase 4 + settle)
        → RequestPresetNames → RequestPresetName → RequestPreset → Standard
```

| Étape | Rôle | Fréquence |
|---|---|---|
| phase 4 (dump 272×N + trailer + PHASE B) | amener l'éditeur « vivant », vider l'état initial | **une fois / connexion** |
| RequestPresetNames | lire les 125 noms | **une fois / connexion** |
| RequestPresetName / RequestPreset | nom + corps du preset actif | une fois à l'amorçage, puis **à chaque changement de preset** |
| Standard | runtime : ACK, événements HW/UI, lectures ciblées | en continu |

Donc « la lecture ne se fait qu'une fois » = **le dump bootstrap + la liste des noms**. Le **corps** d'un preset, lui, se relit à chaque sélection — mais en ciblé, pas un re‑dump complet.

## 2. Le dump phase 4 et son trailer

Le device pousse le dump en rafale de chunks de 272 o, **clos par un chunk partiel** (le « trailer ») :

```
préambule : 92o(54) → 40o(1f) → 68o(39)        ctr 1a:02 … 3f:02   (handshake pré-dump)
dump      : 11 × 272o   head=08                 ctr 50:02
trailer   : 1 × 140o    head=84  sub=04  len<272 ctr 50:02          ← FIN DE DUMP
```

**La taille du trailer dépend du preset actif** (= taille totale modulo la frontière de chunk). C'est `140/84` sur ce run, mais ailleurs ce sera `132/7a`, `116/6a`, … Il faut donc le reconnaître **par sa nature** — un chunk de données (`sub=0x04`) plus court que 272 — jamais par une longueur en dur, sous peine d'intermittence par preset.

## 3. Le « go‑live » : pourquoi le trailer est critique

Le trailer déclenche la **PHASE B** (dialogue éditeur post‑dump : `1b 76:0e`, `1c 76:cc`, `1a`, `19 ed/ef`…). Cette PHASE B **réveille** le device en mode éditeur. Tant qu'elle n'a pas eu lieu :

- le device reste **vivant** (il ping `50:02` / `09:02`),
- mais il **ignore** les requêtes de lecture (`1d` noms, `19` corps).

Donc : **trailer reconnu → PHASE B → go‑live → lectures servies.** Trailer manqué → device muet aux lectures, alors qu'il a l'air vivant.

## 4. Pourquoi un échec est fatal (et paraissait « intermittent »)

Comme tout est **one‑shot**, il n'y a pas de seconde tentative dans la session : si le trailer du preset actif n'est pas reconnu, l'amorçage timeoute (gate phase 4, 3500 ms), le settle est forcé, l'éditeur n'est jamais « vivant », les noms reviennent **vides** et les lectures de corps tournent en watchdog.

D'où le symptôme « marche N fois puis subitement plus rien » : chaque connexion n'a **qu'un seul essai**, et le résultat dépend du **preset actif** au moment du branchement. Un preset dont le dump finit sur une taille reconnue passe ; un autre gèle toute la session. Le reboot du Stomp n'y change rien — c'était une lacune de reconnaissance **côté hôte**.

## 5. Les compteurs (lane ED03)

Tout le dialogue éditeur — requêtes **et** ACK des chunks — doit rouler sur **une seule lane** :

| Lane (octets 12‑13) | Usage | Progression |
|---|---|---|
| `editor_ed03_lane` | requêtes `19`/`1b`/`1c` + ACK chunks dump | `9d:10 → 9d:11 → … → 9d:1b` (lo figé, hi +1/chunk) |

L'erreur passée : acquitter les chunks sur une lane **distincte** figée à `f4:1d`. Le device, qui valide strictement les `19`, n'aime pas la discontinuité de lane. **Aligner les deux sous‑compteurs (lo + hi) simultanément est obligatoire** — n'en corriger qu'un échoue silencieusement.

## 6. Règles à ne pas réapprendre

- **Trailer = chunk partiel**, jamais une longueur en dur (sinon intermittence par preset).
- **One‑shot = pas de filet** : protéger l'amorçage, car aucun re‑dump ne rattrape un échec en cours de session.
- **Go‑live d'abord** : le device ne sert les lectures qu'après la PHASE B ; un device qui ping n'est pas un device prêt.
- **Lane unique** : requêtes et ACK du dump sur `editor_ed03_lane`, alignement lo + hi simultané.

---

*La lecture des presets n'est pas une opération qu'on relance : c'est un rite d'initiation unique par connexion. Tout ce qui peut casser cette séquence doit être traité comme critique, parce qu'il n'y aura pas de deuxième chance avant le prochain branchement.*