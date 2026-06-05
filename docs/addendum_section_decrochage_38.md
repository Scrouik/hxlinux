## 9. Décrochage ~38ᵉ cran : bug parseur host-side (RÉSOLU) — corrige §2 et §5

> **Statut : RÉSOLU.** Le décrochage systématique vers le 38ᵉ/39ᵉ cran n'était **ni** un
> plafond de `ctr`, **ni** un reject, **ni** un gel device. La pédale dumpait un `IN 53`
> parfaitement valide ; c'est notre **extracteur de model-id** qui le ratait. Un correctif
> d'une seule fonction (`extract_first_module_hex_from_bulk`) suffit, vérifié sur octets
> réels. Le scroll 1-cran est désormais **illimité**.
>
> **English:** [addendum_section_decrochage_38.en.md](./addendum_section_decrochage_38.en.md)
>
> **Capture de référence :** `stomp_running_start_linux_multi_one_notch.json` (run 1-notch
> qui « décrochait » au 39ᵉ pull). **Code :** `src-tauri/src/helix/scroll_model_pull.rs`,
> fonction `extract_first_module_hex_from_bulk`. **Test :** `echo_double_0x19_does_not_mask_model_id`.

---

### 9.1. Le fait qui tranche : la pédale dumpe 39/39

Sur le run de référence, le device répond un dump exploitable à **chacun** des 39 pulls,
y compris celui qu'on déclarait « raté ». Au pull qui échoue (`ctr=0x77df`), trame **#3875**,
le device envoie un `IN 53` de 92 o, bien formé, contenant le model-id `cd0209`. Juste après,
les keep-alives reprennent normalement sur toutes les lanes. **Le matériel n'a ni gelé ni
rejeté.** Le « décrochage » est entièrement côté hôte.

Rejoué sur **les 39 trames de dump** de la capture : l'ancien parseur n'échoue que sur
**une seule** (#3875) ; le parseur corrigé lit les 39, dont `cd0209`.

### 9.2. Le mécanisme : collision avec l'écho du double

Juste après le marqueur d'assignation `83 66 cd <cd_lane>`, le device place l'**écho de
notre double** sous la forme `<double_lo> 67 00 68 …` (structure confirmée aussi sur la
capture HX Edit). L'ancien `extract_first_module_hex_from_bulk` cherchait « le premier
`0x19` » dans toute la trame :

```
[24]  83 66 cd 04        ← marqueur
[28]  19 67 00 68 …       ← écho du double : double_lo == 0x19 !
[45]  19 cd 02 09 1a      ← le VRAI model-id (cd0209)
```

Quand `double_lo == 0x19`, l'octet [28] *est* un `0x19`. Le parseur le prenait pour un
marqueur de model-id, cherchait le `1a` suivant **dans tout le buffer**, tombait sur le `1a`
du vrai model-id ~20 o plus loin (`[49]`), jugeait l'« id » trop long (>12 o) et l'abandonnait
— mais le curseur avait **déjà sauté par-dessus** le vrai marqueur de `[45]`. Résultat :
`None` → `finalize` via le timeout → « pull échoué (pas de bulk assignable) », alors que le
dump était bon.

### 9.3. Pourquoi toujours vers le 38ᵉ/39ᵉ cran (déterministe)

Le double est semé à `0xf2` (graine `editor_ed03_double` après PHASE B), premier OUT à
`0xf3`, puis **+1 par cran** (un seul `1b` par cran en grab-53). Il atteint `0x19` après
exactement **38 incréments** → c'est le **39ᵉ pull** qui mord. À la graine de session près,
le décrochage tombe donc toujours au même endroit (d'où le « ~35-38 » ressenti).

### 9.4. CORRECTION des §2 et §5 — il n'y a pas de « plafond 0x7794 »

Le §5 chiffrait le run de référence « `ctr` 0x6cbd→**0x7794** » et le §2 notait « pas de
plafond de page jusqu'à 0x77 ». L'arrêt à `0x7794` n'était **pas** une limite device : c'est
simplement le **dernier cran avant que `double_lo` n'atteigne `0x19`**.

| Cran | `ctr` | `double_lo` | Dump device | Ancien parseur |
|------|-------|-------------|-------------|----------------|
| 38ᵉ (dernier « OK ») | `0x7794` | `0x18` | `cd0122` (Bitcrusher) | lu |
| 39ᵉ (« décrochage ») | `0x77df` | `0x19` | `cd0209` | **raté → None** |

➜ Le run de référence du §5 s'arrêtait donc **exactement pour cette raison**, pas pour un
plafond de lane. Tout le cadrage « continuation monotone / fenêtre de `ctr` » du §2 reste
valable pour ce qui *fait dumper* le device, mais il **ne gouverne pas** le décrochage
terminal — celui-ci était purement host-side.

### 9.5. Le correctif

Dans `extract_first_module_hex_from_bulk`, deux garde-fous :

1. la recherche du `1a` est **bornée** à `MODEL_ID_MAX_LEN = 8` octets (un model-id réel fait
   ≤ ~3 o : `cd0209`, `cd0122`, `64`…) ;
2. si aucun `1a` n'est trouvé dans la fenêtre, ce `0x19` est une collision → on **n'avance
   que de 1** (on ne saute plus par-dessus un marqueur situé plus loin).

Le correctif vit dans le parseur, donc il couvre aussi les **récurrences futures** : le
double repassera par `0x19` tous les 256 crans, sans jamais re-déclencher le bug. Aucun
compteur ni rien du protocole n'est touché. Test de non-régression
`echo_double_0x19_does_not_mask_model_id` ajouté avec les octets réels de #3875.

### 9.6. Chiffres de référence (révisés)

| Run | Pulls | Dumps device | Lus (ancien) | Lus (corrigé) | Gel |
|-----|-------|--------------|--------------|---------------|-----|
| 1-notch (`multi_one_notch`) | 39 | **39 (100 %)** | 38 | **39** | 0 |

Les ~8 % de **vrais** rejects device (`IN 21` sans dump qui suit, §4) restent un phénomène
distinct et réel ; ce run-ci n'en a eu **aucun**. En l'état, grab-53 en scroll 1-cran est
nettement plus solide que ne le laissait penser l'addendum : la seule faille était ce bug
de lecture.

---

*Synthèse : le « mur du 38ᵉ cran » n'existait pas côté matériel. La pédale dumpait 39/39 ;
un `0x19` d'écho du double se faisait passer pour un marqueur de model-id et masquait le
vrai. Une fonction corrigée, un test sur octets réels, et le scroll 1-cran devient illimité.
Leçon : avant de théoriser un plafond firmware, vérifier que l'octet attendu est bien lu —
le mur était dans notre parseur, pas dans l'ED03.*