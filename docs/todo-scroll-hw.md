# TODO — Scroll modèle hardware (molette Stomp)

**Objectif produit** : quand l’utilisateur tourne la molette modèle sur le Helix Stomp, HXLinux affiche le **bon modèle et ses paramètres** sans `request_preset_content` à chaque cran — comme HX Edit.

**Pourquoi ce chantier existe** : l’ancienne implémentation (`slot_model_hw_pull`, pending, quarantaine, lane scroll empirique) produisait des pulls en trop, des doublons UI et des freezes Stomp. Le code a été **puré** pour repartir d’une **référence mesurée** (capture HX Edit), pas d’hypothèses empilées.

**État code (mai 2026)** — détail : [`SCROLL_HW_RESET.md`](SCROLL_HW_RESET.md)

| Composant | Aujourd’hui |
|-----------|-------------|
| `usb_in_pipeline.rs` | Ordre + `Ignored`/`Observed`/`Consumed` des couches actives |
| `firmware_scroll_ack.rs` | Couche 1 fond — `handle_in_layer` |
| `slot_model_hw_pull.rs` | Couche 2 scroll — `handle_in_layer` → `Ignored` ; `ingest` → `None` |
| `HelixState` | `firmware_scroll_ack_*` seulement |
| UI `models` | Molette Stomp **ne met pas à jour** le modèle affiché |

**Références**

| Ressource | Rôle |
|-----------|------|
| `captures/usb-wireshark/` | Captures Wireshark (JSON gitignored) |
| `captures/usb-wireshark/3_scroll_HXEdit.json` | Cible : **un** scroll HX Edit |
| `captures/usb-wireshark/README.md` | Convention noms / capture |
| `Line6_HX_Stomp_USB_Protocol.md` | Vocabulaire trames (`1d`, `1f`, `1b`, `19`, bulks…) |
| `description.md` | Journal projet (à mettre à jour après chaque phase validée) |
| Git `d6eb2b1` | Ancien pull complet (archive, **ne pas recopier** sans preuve capture) |

**Méthode de travail** (décision agent — l’utilisateur n’intervient que en cas de désaccord ou incompréhension)

1. **Mesurer avant de coder** — pas de nouvelle machine à états sans timeline validée sur capture.
2. **Un scroll = une séquence** — pas de file `pending`, pas de « rattrapage » automatique.
3. **Séparer les lanes** — ne jamais mélanger `preset_dump_ack_ctr`, `editor_ed03_double` / live write, et la future lane scroll (à redéfinir depuis HX Edit).
4. **Comparer binaire** — replay Linux vs capture HX Edit avant de brancher l’UI.
5. **Petits commits** — une phase validée = un commit explicite.

---

## Vocabulaire (figé mai 2026)

| Terme | Définition | Code / module |
|-------|------------|---------------|
| **Fond** | Dialogue **permanent** tant que USB est ouvert : IN `1d`/`1f` 40 o → OUT `f0:03` sub=`08` + lane `firmware_scroll_ack_ctr` | `firmware_scroll_ack` ; pipeline `FirmwareScroll` |
| **Épisode** | Rafale **ponctuelle** intercalée (scroll modèle, dump 272, live write…) ; lanes distinctes | `ScrollPull`, `PresetDumpStream`, modes preset… |
| **Amorçage** | Séquence **unique** par session pour ouvrir les canaux et **armer** le fond (pas le fond lui-même) | `Connect` → `ReconfigureX1` → `editor_phase4_bootstrap` → settle ~700 ms |

**Hiérarchie** : amorçage (une fois) → fond (toujours) ; les épisodes se **superposent** au fond sans le remplacer.

### Spec cible — phases de session (cinématique)

| Phase | Fond (réactif `1d`/`1f` 40 o) | Épisodes host | Amorçage host (OUT proactifs) |
|-------|------------------------------|---------------|-------------------------------|
| `Bootstrapping` | **OFF** | **OFF** | seul actif |
| `EditorReady` → | **ON** | **ON** (selon besoin) | terminé |

Checkpoint unique **`EditorReady`** : canaux ouverts, `09:10` posé, phase 4 faite, settle ~700 ms écoulé.

**Écart implémentation actuelle** : le pipeline fond peut répondre dès qu’un `1d` arrive (pas de gate `SessionPhase`) — à aligner sur la spec après validation capture.

### Critère A vs B — « silence court » ou ACK pendant l’amorçage ?

Question à trancher **uniquement par capture HX Edit**, pas par le code existant :

- **A (strict)** : fond OFF pendant tout `Bootstrapping` — le host ignore les `1d` scroll jusqu’à `EditorReady`.
- **B (armement)** : fond ON dès le OUT `09:10`, même si phase 4 / settle pas finis.

**Comment vérifier** (sur `01_connect_HXEdit.json` ou `connect_device_30s_HXEdit.json`) :

1. Compter les IN **`1d` / `1f` 40 o** avec tête `f0:03:02:10` (notif scroll) entre le **premier OUT** `09:10` sub=`08` et le **premier** `RequestPresetNames` / 1er poll keep-alive `f0` sub=`10` (≈ frame #3761 sur capture 30 s).
2. Pour chaque tel IN : HX Edit envoie-t-il un OUT `f0` sub=`08` ACK dans les ~300 ms ?

| Résultat capture | Verdict design |
|------------------|----------------|
| **0** IN scroll `1d`/`1f` pendant toute la fenêtre amorçage | **A** — pas besoin d’ACK ; le Stomp **ne parle pas** encore sur ce dialogue ; silence host = conforme HX Edit |
| ≥1 IN scroll + HX Edit ACK | **B** — fond réactif dès armement |
| ≥1 IN scroll + HX Edit **silence** | **A** ou bug HX Edit — à documenter |

### Verdict actuel (mesure captures — mai 2026)

Analyse directe des JSON Wireshark (`scripts` ad hoc, signature fond = IN **`1d`/`1f` 40 o** avec `f0:03:02:10` @ octets 4–7 ; OUT ACK = `08…02:10:f0:03` **sub=`08`**, lane octets 12–13).

| Capture | IN scroll fond (`1d`/`1f` + `f0:02:10`) | OUT `f0/08` (hors armement `09:10` seul) | Pendant amorçage (#arm → #1er poll `sub=10`) |
|---------|-------------------------------------------|------------------------------------------|-----------------------------------------------|
| `01_connect_HXEdit.json` | **0** | **0** (1× `09:10` armement #1473) | Phase 4 + IN **`1f` 40 o `ed:03`** intercalés — **pas** fond scroll |
| `connect_device_30s_HXEdit.json` | **0** (30 s entières) | **0** | idem (#3255 arm → #3761 poll) |
| `stomp_running_start_hxedit.json` (idle post-connect) | **303** | **302** (dès `48:10` après 1er `1d` #8582) | — |

**Conclusion cinématique** :

1. **Intuition partiellement vraie** : pendant l’amorçage USB, HX Edit a déjà un fil **très actif** qui **s’entrelace** (phase 4 `19`/`1a`, IN `1f` 40 o sur **`ed:03`**, preset…).
2. **Mais le fond scroll** (notif `1d`/`1f` **`f0:02:10`** → ACK lane qui avance `48:10`, `5d:10`…) **n’apparaît pas** dans les captures connect / 30 s post-connect.
3. Ce qu’on voit tôt sur `f0/08`, c’est surtout l’**armement** host `09:10` (#1473 / #3255) — pas la boucle réactive fond.
4. La boucle fond démarre dans l’**idle** (`stomp_running…`) : 1er IN scroll #8582 → 1er ACK réactif `48:10` #8634.

→ **Spec cible : A** pour le **fond scroll** pendant `Bootstrapping` : pas d’ACK réactif `1d`/`1f` `f0:02:10` tant que le Stomp n’ouvre pas ce dialogue (connect ne le montre pas).

**Ne pas confondre** : un `1f` 40 o pendant connect (`ed:03:80:10`) = **épisode / autre canal**, pas fond scroll.

Fenêtre optionnelle à capturer si doute : **ouverture HX Edit → branchement USB** (nos captures commencent au connect device, pas au lancement app).

### Timeline connect → fond (spec)

1. **Amorçage** — host seul ; armement `09:10` ; phase 4 ; settle — **fond OFF**.
2. **`EditorReady`** — checkpoint explicite.
3. **Fond** — Stomp pousse `1d` ; host ACK (`f0/08`) ; keep-alive `f0 sub=10` en parallèle (dialogue host distinct).

---

## Pipeline USB — contrat des couches (figé mai 2026)

Chaque trame IN (0x81) traverse des **couches actives** dans un ordre fixe (`usb_in_pipeline.rs`).
Implémentation : `HelixState::run_usb_in_active_layers` depuis `usb_listener`.

### Résultats d’une couche

| `LayerResult` | Effet sur le fil | Couches suivantes (actives) |
|---------------|------------------|-----------------------------|
| **Ignored** | Rien — pas mon paquet | continue |
| **Observed** | Rien sur le fil (trace / état interne seulement) | continue (**variante A**) |
| **Consumed** | Action complète (ex. **lane + ACK**) | **stop** |

Règle protocolaire : **Consumed = lane + ACK** pour le fond sur `1d`/`1f` 40 o.
**Observed** ne doit pas avancer `firmware_scroll_ack_ctr` sans envoyer l’ACK correspondant
(sinon désync Stomp). « Lane seule sans ACK » = interdit sauf preuve capture.

### Ordre des couches actives (2026-05-27)

1. **Fond** (`FirmwareScroll`) — `firmware_scroll_ack::handle_in_layer` (`1d`/`1f` → `f0/08`, lane scroll)
2. **ScrollPull** — `slot_model_hw_pull::handle_in_layer` (**Ignored** tant que pull désactivé)
3. **PresetDumpStream** — `preset_dump_stream_ack::handle_in_layer` (IN 272 → `ed/08`, lane preset)

Passives (hors pipeline, toujours appelées) : `ingest_hw_slot_notify_in`, `ingest_slot_param_in`,
`mode.data_in`, `ingest_slot_model_hw_in` (UI / état sans OUT concurrent sur le même gabarit).

### Avant réactivation du pull

- [x] Contrat `LayerResult` + `run_active_layers`
- [x] `usb_listener` branché sur le pipeline
- [ ] Pull : retourner `Consumed` seulement sur gabarits épisode ; ne pas re-ACK `1f` si fond a déjà `Consumed`
- [ ] Valider sur capture `one_scroll_hxedit.json` règles `1f` (Observed fond vs Consumed scroll)

---

## Objectif structurant — dialogue de fond vs « bruit » utilisateur

**Hypothèse (accord mai 2026)** : tant que USB est connecté, un **dialogue de fond** ne s’arrête pas
(IN `1d` / parfois `1f`, ACK `f0:03` sub=`08`, lane scroll, keep-alive…). Les manipulations
utilisateur **s’intercalent** et **enrichissent** le fil (pull `1b`/`19`, dump preset, live write…)
sans remplacer ce fond — d’où **plusieurs lanes** distinctes.

**Grande avancée** = savoir **reconnaître le fond dans les deux cas** :

| Situation | Capture | Question |
|-----------|---------|----------|
| **Référence fond seul** | `stomp_running_start_hxedit.json` (idle) | Quel motif se répète ? ratio `1d`/ACK sub=`08`, pas des `1b`/`19` ? |
| **Fond + action** | `scroll_one_HXEdit_*.json` (un cran molette) | Quels paquets sont **épisode** vs **fond qui continue** entre deux `1f` ? |

**Méthode d’analyse** (avant tout nouveau code pull) :

1. **Signature fond** — tracer depuis l’idle : enchaînement typique `IN 1d` → OUT `f0/08` 16 o, Δ lane, délai ms.
2. **Signature épisode scroll** — paquets **absents** de l’idle : `OUT 1b`, `OUT 19`×2, bulk ~92/272, lane `editor_ed03_double`.
3. **Soustraction** — sur capture scroll : retirer mentalement (ou par script) les trames « fond » ; ce qui reste = séquence à coder pour l’UI modèle.
4. **Règles de coexistence** — noter ce que HX Edit fait **encore** pendant l’épisode (ACK `1d` oui/non ? lane scroll figée ou avance ?).

Tant qu’on ne peut pas **séparer** fond et épisode sur une capture « bruitée », on ne code pas de pull — on risque de re-mélanger les couches.

---

## Phase 0 — Prérequis (fait)

- [x] Reset code : purge `hw_model_*`, ACK scroll, garde-fous pull (`cd0d2d1`)
- [x] Captures déplacées sous `captures/usb-wireshark/`
- [x] Doc reset : [`SCROLL_HW_RESET.md`](SCROLL_HW_RESET.md)

---

## Phase 1 — Stabilité post-connexion (sans scroll)

**Pourquoi** : un désync ou un flood USB dès le connect fausse tout le reste ; le bug peut être **là** avant le premier scroll.

**À faire**

- [x] Lancer HXLinux, connecter au Stomp, **ne pas** toucher la molette modèle pendant 30–60 s.
- [x] Captures : `stomp_running_start_hxedit.json`, `stomp_runnig_start_hxlinux.json`
- [x] Comparer les deux captures (script Python ad hoc, mai 2026)
- [x] Retour utilisateur (27 mai) : pas de scroll touché, rien ne bouge en front ; **preset affiché nettement plus vite** qu’avant la purge ; impression **connexion plus rapide qu’HX Edit**. Pas de freeze constaté en idle.

**Critère de succès** : session stable, pas de dégradation progressive ; slot actif et preset affichés restent cohérents avec le HW.

**Résultat analyse (27 mai 2026)** — voir [Notes terrain](#notes-terrain) :

- **USB** : écart majeur — HX Edit ACK tous les `1d` (sub=`08`) ; HXLinux quasi aucun ACK après bootstrap.
- **Produit (idle)** : critère « pas de freeze + preset cohérent » **OK** pour l’utilisateur ; la lenteur perçue d’avant venait probablement du trafic scroll/pull en parallèle du chargement preset.
- **Protocole** : désync `1d` non acquittés — **pas bloquant en idle**, mais risque fort dès qu’on scrolle ou qu’on réouvre HX Edit en parallèle. Phase 1 **OK côté UX connect**, **à compléter côté fil** avant phase 2.

**Livrable** : court paragraphe dans ce fichier (section « Notes terrain ») + entrée `description.md` si anomalie.

---

## Phase 2 — Capture référence HX Edit (un scroll)

**Pourquoi** : HX Edit est la **vérité terrain** pour l’enchaînement Stomp ↔ host ; tout le code scroll doit s’y calquer.

**À faire**

- [x] Capture : `captures/usb-wireshark/one_scroll_hxedit.json` (~11,6 s ; rafale scroll ~1,04–1,65 s).
- [ ] (Optionnel) Refaire une capture **un seul cran** si on veut isoler 1×`1f` (celle-ci contient **6×`1f`** / 5 cycles pull en ~0,5 s — plusieurs pas modèle ou rebond molette).

**Critère de succès** : capture courte, un scroll clairement isolé, horodatage relatif exploitable.

---

## Phase 3 — Analyse trame par trame

**Pourquoi** : comprendre **qui parle en premier**, quels ACK sont obligatoires, quels OUT déclenchent quels IN, et les **délais** — pas seulement les opcode.

**À faire**

- [x] Séparer **couche 1 (fond)** / **couche 2 (épisode scroll)** / **entrelacement** — voir ci-dessous.
- [ ] (Optionnel) Script `scripts/analyze_scroll_capture.py`.
- [ ] Affiner sur capture « 1 cran strict » si besoin.

**Critère de succès** : document « séquence canonique » réutilisable pour coder — section ci-dessous.

**Livrable** : [Analyse `one_scroll_hxedit.json`](#analyse-one_scroll_hxeditjson) + [Séquence canonique](#séquence-canonique-hx-edit-un-cycle).

---

## Phase 4 — Replay HXLinux (un scroll)

**Pourquoi** : valider que le host reproduit la **même conversation** que HX Edit avant d’exposer l’UI.

**À faire**

- [ ] Réintroduire dans `slot_model_hw_pull.rs` (ou module dédié) **uniquement** les étapes prouvées en phase 3.
- [ ] Réintroduire lane / compteurs **un par un**, avec tests unitaires sur octets extraits de la capture.
- [ ] Test terrain : HXLinux connecté, **un** scroll Stomp → capture `scroll_one_HXLinux_YYYYMMDD.json`.
- [ ] Diff OUT/IN vs HX Edit (nombre de `1b`, ordre des `19`, présence 272, délais ordre de grandeur).
- [ ] Brancher `models:slot-model-changed` **seulement** quand le hex modèle est déterminé (règle explicite dans la spec phase 3).

**Critère de succès** : captures Linux et HX Edit **structurellement alignées** ; UI affiche le bon modèle après un cran ; Stomp ne freeze pas.

**Hors scope immédiat** : scroll rapide multi-crans, changement de slot + scroll, picker UI → HW (autres chantiers).

---

## Analyse `one_scroll_hxedit.json`

### Couche 1 — dialogue de fond (signature)

| Signature | Détail |
|-----------|--------|
| IN | `1d` 40 o (`f0:03:02:10` …) |
| OUT | `08:00:00:18` + `02:10:f0:03` + **sub=`08`** (octet 11) + lane scroll octets 12–13 |
| Rythme | ~1 ACK par `1d` (comme capture idle) |

**Hors rafale scroll** (avant t≈1,04 s et après t≈1,65 s) : uniquement fond (`1d` + ACK), pas de `1b`/`19`.

**Pendant la rafale** (t≈1,04–1,65 s) : le fond **continue** — ex. 10×`1d`, 11×ACK `f0/08` **en plus** de l’épisode pull.

### Couche 2 — épisode scroll (un cycle type, ~92 ms cœur)

Déclenché par IN **`1f`** 40 o → host enchaîne (lane **éditeur** sur `1b`/`19`, pas lane scroll) :

| Δt (ms) | Dir | Paquet | Rôle |
|--------|-----|--------|------|
| 0 | IN | `1f` | « modèle a changé » côté HW |
| +1,7 | OUT | `1b` 36 o | début pull |
| +1,8 | OUT | `f0/08` | **fond** : ACK lane scroll (pas un ACK du `1f` seul) |
| +3,3 | IN | ~92 o | 1ʳᵉ réponse pull |
| +17 | OUT | `19` 36 o | |
| +38 | IN | ~68 o | |
| +45 | IN | `21` 44 o | post-assign |
| +45 | IN | `1d` | **fond** au milieu de l’épisode |
| +49 | OUT | `19` #2 | |
| +49 | OUT | `f0/08` | **fond** |
| +81 | IN | ~272 o × N | commit modèle (+ rafale) |

**Troisième activité entrelacée** (pas la lane scroll) : pendant les IN 272, HX Edit envoie OUT `80:10:ed:03` sub=`08` 16 o — ACK **flux dump preset** (`preset_dump_ack_ctr`), ~70 fois dans la rafale. À ne pas confondre avec le fond `02:10:f0:03`.

### Entrelacement — règles observées

1. Le **fond ne se met pas en stand-by** pendant le scroll : `1d` et ACK `f0/08` continuent.
2. **`1f` → `1b` en ~2 ms** : pas d’ACK `f0/08` *avant* le `1b` sur ce cycle (l’ACK arrive juste après le `1b`).
3. Un « scroll utilisateur » sur cette capture = **plusieurs cycles** (6×`1f`, 5–6×`1b`) en ~0,5 s — traiter comme **plusieurs pas modèle**, pas un seul épisode.
4. Coder plus tard : **pull** = couche 2 sur `1f` ; **firmware_scroll_ack** = couche 1 inchangée ; **preset_dump_stream_ack** = couche 3 sur 272 seulement.

---

## Séquence canonique HX Edit (un cycle)

Cycle 1 extrait (t≈1,090 s), forme simplifiée :

```text
IN 1f → OUT 1b → OUT f0/08 (fond) → IN ~92 → OUT 19 → IN ~68 → IN 21 → IN 1d (fond)
      → OUT 19 → OUT f0/08 (fond) → IN 272… (+ OUT ed/08 par chunk 272)
```

| # | Δt (ms) | Dir | Type | Notes |
|---|---------|-----|------|-------|
| 1 | 0 | IN | `1f` | déclencheur épisode |
| 2 | +2 | OUT | `1b` | pull |
| 3 | +2 | OUT | `f0/08` | fond (lane scroll) |
| 4 | +3 | IN | ~92 | |
| 5 | +17 | OUT | `19` | |
| 6 | +38 | IN | ~68 | |
| 7 | +45 | IN | `21` | |
| 8 | +45 | IN | `1d` | fond |
| 9 | +49 | OUT | `19` | |
| 10 | +49 | OUT | `f0/08` | fond |
| 11 | +81 | IN | 272… | + ACK dump preset sur chaque chunk |

---

## Notes terrain

| Date | Phase | Observation |
|------|-------|-------------|
| 2026-05-26 | 0 | Reset complet ; pas de scroll actif côté host |
| 2026-05-27 | 1 | Captures idle ~23 s post-bootstrap. **HX Edit** : IN `1d`×303, OUT `f0:03` sub=`08`×302 (≈100 % des `1d` acquittés en moins de 300 ms), doubles lane qui évoluent (`48:10`, `5d:10`, …). **HXLinux** : IN `1d`×195, OUT sub=`08`×1 (bootstrap `09:10` seul), **0 %** ACK sur `1d` ; keep-alive envoie `f0:03` sub=`10` + `09:10` figé toutes les ~1 s (`keep_alive.rs`). Pas de scroll (`1f`=0 Linux, 5 HX Edit). **Ressenti utilisateur** : preset plus rapide qu’avec l’ancien code scroll ; connexion perçue plus rapide qu’HX Edit ; idle sans symptôme visible. **Action avant phase 2** : réintroduire ACK `1d`/`1f` (lane depuis capture), **sans** remettre pull ni garde-fous lourds — pour ne pas regagner la lenteur au chargement preset. |
| 2026-05-27 | 1b | **Couche ACK mini** : `firmware_scroll_ack.rs` — voir commit local / branche en cours. |
| 2026-05-27 | 1c | **Validation idle Linux V2** : `stomp_runnig_start_hxlinux_V2.json` — post-boot ~22,5 s : IN `1d`×259, OUT `f0 sub08`×258 (ratio **1,00**), 99 % ACK &lt;300 ms ; lane `0x1009`→`0x1048` (+`0x3f`) comme HX Edit. vs V1 : 1 ACK / 195 `1d`. Reste : keep-alive `sub=10`×21 (HX Edit idle = 0). |
| 2026-05-27 | 1d | **Validation terrain amorçage** : alerte front/back `debug:fond-amorcage` activée ; test connect réel sans action utilisateur = **aucune alerte**. Décision figée : fond OFF en `Bootstrapping`, ON en `EditorReady` ; filtre fond strict (`1d`/`1f` 40 o + `f0:03:02:10`). |

### Détail comparatif (après bootstrap `09:10`)

| Métrique | HX Edit | Linux V1 | Linux V2 |
|----------|---------|----------|----------|
| Durée post-boot | ~23 s | ~21 s | ~22,5 s |
| IN `1d` | 303 | 195 | **259** |
| IN `1f` | 5 | 0 | 0 |
| OUT `f0` sub=`08` | 302 | 1 | **258** |
| Ratio ACK / `1d` | 1,00 | 0,01 | **1,00** |
| Lane 1er ACK après boot | `48:10` | — | **`48:10`** |
| OUT `f0` sub=`10` | 0 | 20 | 21 |
| OUT `1b` / `19` | 4 / 6 | 0 / 8 | 0 / 9 |

Fichiers : `stomp_running_start_hxedit.json`, `stomp_runnig_start_hxlinux.json` (V1), `stomp_runnig_start_hxlinux_V2.json` (V2).

---

## Checklist rapide (jour J)

1. [x] Phase 1 — connect / idle : UX OK + **fil fond V2 ≈ HX Edit** (`stomp_runnig_start_hxlinux_V2.json`)
2. [x] Phase 2 — capture `one_scroll_hxedit.json`  
3. [~] Phase 3 — couches 1/2 + entrelacement documentés (affiner si capture 1 cran strict)  
4. [ ] Phase 4 — code minimal + capture Linux + diff  

---

## Historique décisions

| Date | Décision | Raison |
|------|----------|--------|
| 2026-05-26 | Supprimer toute couche scroll legacy | Empilement de patches non prouvés ; freezes et doublons UI |
| 2026-05-26 | Pas d’ACK `1d`/`1f` jusqu’à preuve capture | Fil USB vierge pour analyse |
| 2026-05-26 | Plan en 4 phases connect → capture → analyse → replay | Éviter de coder avant mesure |
