# TODO — Scroll modèle hardware (molette Stomp)

**Objectif produit** : quand l’utilisateur tourne la molette modèle sur le Helix Stomp, HXLinux affiche le **bon modèle et ses paramètres** sans `request_preset_content` à chaque cran — comme HX Edit.

**Pourquoi ce chantier existe** : l’ancienne implémentation (`slot_model_hw_pull`, pending, quarantaine, lane scroll empirique) produisait des pulls en trop, des doublons UI et des freezes Stomp. Le code a été **puré** pour repartir d’une **référence mesurée** (capture HX Edit), pas d’hypothèses empilées.

**État code (mai 2026)** — détail : [`SCROLL_HW_RESET.md`](SCROLL_HW_RESET.md)

| Composant | Aujourd’hui |
|-----------|-------------|
| `usb_in_pipeline.rs` | Ordre + `Ignored`/`Observed`/`Consumed` des couches actives |
| `firmware_scroll_ack.rs` | Couche 1 fond — `handle_in_layer` |
| `slot_model_hw_pull.rs` | **Supprimé** — pull scroll phase 4 (non réactivé) |
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

### Fenêtre post-`ARM_ef` → phase4 → keep-alive (`01_connect_*`)

**Captures** : `01_connect_HXEdit.json` (~12 s) vs `01_connect_HXLinux.json` (~30 s).  
**Procédure** : app ouverte → Stomp branché/allumé (≠ `stomp_running_*`).  
**Référence temps** : `OUT ARM_ef` (`01:10:ef:03` sub=`08` lane `09:10`) = **t = 0**.  
`ARM_f0` est à **−16 ms** (HX) / **−31 ms** (Linux) avant `ARM_ef`.

#### Trois phases sur le fil

| Phase | Δt typique | Contenu |
|-------|------------|---------|
| **A — silence host** | 0 → ~220 ms | Après `ARM_ef`, le host n’envoie rien jusqu’au 1er `19` phase 4. IN `short08` device possibles. |
| **B — phase 4 + dump** | ~220 → ~350 ms | Host : 3× OUT `19` ed + 1× OUT `1a` ef. Stomp : rafale IN **272** + écho `19` ; host ACK **ed sub=`08`** (lane dump, ≠ fond scroll). |
| **C — settle + polling** | ~350 ms → ~1000 ms | ~700 ms sans requête host proactive ; puis 1er cycle keep-alive (poll `f0` sub=`10`). |

#### Jalons comparés (Δ ms depuis `ARM_ef`)

| Jalon | HX Edit | Linux | Verdict |
|-------|---------|-------|---------|
| Silence host → 1er `phase4_19` | **+219** (#1545) | **+224** (#261) | **OK** (+5 ms) |
| `phase4_1a` | +344 (#1651) | +284 (#281) | OK (Linux un peu plus tôt) |
| 1er keep-alive `f0` sub=`10` | **+1047** (#1971) | **+960** (#363) | **OK** (`1a` + ~700 ms settle) |
| IN scroll fond `1d`/`1f` 40 o `f0:02:10` | **0** | **0** | **OK** (fond pas attendu en connect) |
| `#` `phase4_19` dans 0–2 s | 6 | 8 | Linux +2 (2e vague, voir ci-dessous) |
| `#` keep-alive `f0`/10 (fichier entier) | 6 | 14 | Linux = capture plus longue |

**Formule settle** : fin phase 4 (`1a` ~+284–344 ms) + **700 ms** ≈ 1er poll `f0`/10 (~+960–1047 ms) — **`amorcage.rs` aligné** sur cette fenêtre pour `01_connect`.

#### Phase A — détail

- HX Edit : 1× IN `short08` **ef** à +179 ms.
- Linux : triplet IN `short08` **ef + ed + f0** à +137 ms (réaction device post triple ARM batch).

#### Phase B — détail

- Écart ~20 ms entre chaque OUT `19` : identique des deux côtés.
- Dès ~+284 ms : rafale IN **272** ; host enchaîne OUT `08` **ed** sub=`08` (ACK chunks preset — **pas** lane fond `f0:03` sub=`08` scroll).

#### Phase C — détail

- Intervalle 1er → 2e poll `f0` sub=`10` : **~1062 ms** (HX) / **~1040 ms** (Linux) — proche du cycle `keep_alive.rs` (~1040 ms).
- Linux : poll **ed** sub=`10` à +904 ms **avant** le 1er poll **f0** sub=`10` (+960 ms).

#### Écart Linux : 2e vague `phase4_19`

Vers **+1140 ms** et **+1360 ms**, Linux envoie encore des OUT `19` ed (#431, #441, #509…) — absents sur HX Edit dans la même fenêtre sur capture ~12 s. Hypothèse : chevauchement **`RequestPresetNames`** / requêtes preset après settle. À re-vérifier sur capture Linux **coupée ~12 s** post-connect.

#### Conclusion (ne pas mélanger avec le bug fond idle)

Sur **`01_connect_*`**, la fenêtre post-`ARM_ef` → phase4 → keep-alive est **déjà très proche** de HX Edit.  
Le blocage fond (**0** IN `1d` sur `stomp_running_*`) vient surtout **en amont** :

- OUT connect **[7–11]** : ARM **entrelacé** HX Edit (`ARM_ed → x11 x2 → Reconfigure ef → ARM_f0 → x11 x1 → ARM_ef`) vs Linux (Reconfigure fini **puis** batch ARM dans `amorcage.rs`).
- Voir journal **amorçage** / captures `stomp_running_start_hxedit.json` ↔ `stomp_running_start_hxlinux.json`.

**Fix prioritaire code** : entrelacement ARM dans `Connect` / `ReconfigureX1` — **pas** retoucher les délais phase4 (+224 ms) ni settle (+960 ms) sur `01_connect`.

#### Correctif code (mai 2026) — entrelacement ARM

Remplacer le **batch** `amorcage::schedule_triple_arm` par la séquence HX Edit :

| Étape | Module | OUT |
|-------|--------|-----|
| Réponse init x2 | `connect.rs` | `ARM_ed` → ack x11 x2 |
| Reconfigure fin (x11 ef) | `reconfigure_x1.rs` | `ARM_f0` → ack x11 → `ARM_ef` |
| +235 ms | `amorcage::spawn_post_arm_sequence` | phase4 → settle 700 ms → keep-alive → `RequestPresetNames` |

Fichiers : `amorcage.rs` (`send_arm_ed` / `send_arm_f0` / `send_arm_ef`), `connect.rs`, `reconfigure_x1.rs`.

**Test** : capture `stomp_running_start_hxlinux.json` — OUT [7–12] doit matcher HX Edit ; critère fond : IN `1d` scroll ≥ 200 / 27 s, ratio ACK ≈ 1.

### Relecture code amorçage (27 mai 2026)

Séquence implémentée (alignée `todo` / captures `01_connect`) :

| Étape | Fichier | OUT |
|-------|---------|-----|
| Réponse init x2 | `connect.rs` | `ARM_ed` → ack `0x11` f0 |
| Fin Reconfigure | `reconfigure_x1.rs` | `ARM_f0` → ack `0x11` ef → `ARM_ef` (+16 ms) |
| +235 ms thread | `amorcage::spawn_post_arm_sequence` | phase4 → settle 700 ms → `editor_ready` → keep-alive → `RequestPresetNames` |

Points validés en lecture :

- `spawn_post_arm_sequence` déclenché une seule fois via `AwaitPostBootstrapSettle` (`lib.rs`).
- Fond scroll : filtre strict `1d`/`1f` 40 o + `f0:03:02:10` ; ACK dès `firmware_scroll_armed` (post `ARM_f0`), **sans** gate `EditorReady` — HX envoie des `1d` ~+20 ms après `ARM_ef` avant la fin phase 4.
- « x11 x2 » dans les captures = canal **f0** (pas deux acks) ; le 2ᵉ `x11` f0 (`09:02`) ne déclenche plus d’ARM local (reporté à Reconfigure).

Écart mineur connu (non bloquant sur V2 idle) : keep-alive envoie `f0` sub=`10` avec double figé `09:10` (~21×/session) ; HX Edit idle = 0 poll `sub=10` — à surveiller sur prochaine capture.

**Script validation capture** : `scripts/analyze_stomp_running_compare.py` (ARM + métriques fond).

**Référence terrain déjà verte** : `stomp_runnig_start_hxlinux_V2.json` — 259 `1d`, ratio ACK 1,00 (le fil fond **peut** tourner sur Linux ; le jalon ouvert = reproduire après fix entrelacement ARM sur procédure `stomp_running_*` fraîche).

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
2. **PresetDumpStream** — `preset_dump_stream_ack::handle_in_layer` (IN 272 → `ed/08`, lane preset)

(Pull scroll = couche future, hors pipeline tant que phase 4 non validée.)

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

## Analyse `stomp_running_start_hxlinux.json` (27 mai 2026 — post-fix entrelacement ARM)

Capture : ~30,7 s, 294 paquets `capdata` (capture démarre ~3,6 s après bruit USB initial).

### Verdict global

| Critère | Cible | Mesuré | Statut |
|---------|-------|--------|--------|
| Entrelacement ARM | `ARM_ed` → x11 f0 → … → `ARM_f0` → x11 ef → `ARM_ef` | Présent (#283 → #285 → #297 → #299 → #303, Δt ≈ +70–88 ms depuis 1er OUT connect) | **OK** |
| Phase 4 + settle | 3×`19` + `1a` puis ~700 ms | #327–345 (+326–387 ms) ; 1er `f0` poll sub=`10` #429 (+1063 ms) | **OK** (ordre de grandeur) |
| Fond idle | IN `1d` fond ≥ ~200 / ~22 s idle | **0** IN `1d` / `1f` 40 o `f0:02:10` | **KO** |
| ACK fond réactif | OUT `f0` sub=`08` ≈ IN `1d` | **1** OUT `f0/08` (bootstrap `09:10` #297 seul) | **KO** |

**Conclusion** : l’amorçage OUT ressemble enfin à HX Edit sur la fenêtre connect, mais le Stomp **n’ouvre pas** le dialogue fond scroll (pas un problème de classification pipeline — il n’y a rien à classer).

### Séquence bootstrap (depuis 1er OUT host #247, t = 0)

| Δt (ms) | Frame | Dir | Rôle |
|--------:|------:|-----|------|
| 0 | 247 | OUT | init x1 ef |
| 69 | 276 | OUT | init x2 f0 |
| 70 | 283 | OUT | **ARM_ed** `80:10:ed:03` lane `09:10` |
| 71 | 285 | OUT | ack x11 f0 |
| 71–72 | 289–297 | OUT | reconfigure x1 + **ARM_f0** |
| 72 | 299 | OUT | ack x11 ef |
| 88 | 303 | OUT | **ARM_ef** |
| 326–387 | 327–345 | OUT/IN | phase 4 (3×`19` + `1a`) + rafale IN 272 |
| 1063 | 429 | OUT | 1er keep-alive `f0` sub=`10` lane `09:10` figée |
| 1225+ | 497+ | OUT | 2ᵉ vague `19`×6 + rafale OUT `ed` sub=`08` (lecture preset) |

### Fenêtre idle (~8,6 s → 30,7 s, ~22 s)

- IN : uniquement triplet keep-alive **16 o** sub=`10` (`ef` / `f0` / `ed`) — **22× chacun**, **0×40 o**.
- OUT : **21×** `f0` sub=`10` avec lane **`09:10` figée** ; pas de boucle `1d` → `f0/08` évolutive (`48:10`…).

### Écart vs `stomp_runnig_start_hxlinux_V2.json` (référence verte todo)

| Métrique | V2 (idle OK) | Cette capture |
|----------|--------------|---------------|
| IN `1d` fond | 259 | **0** |
| OUT `f0/08` | 258 | **1** |

Même procédure nominale, résultat **opposé** au niveau device → vérifier conditions terrain : Stomp déjà lié à HX Edit / autre host, reboot Stomp avant capture, filtre Wireshark, ou régression OUT non visible sur la seule fenêtre ARM.

### Pistes (par priorité)

1. **Terrain** : reboot Stomp, fermer HX Edit, recapture identique.
2. **Keep-alive** : `f0` sub=`10` envoie `09:10` (lane scroll) au lieu d’un double poll dédié — HX Edit idle = 0× sub=`10` ; à tester sur capture suivante.
3. **2ᵉ vague `19`** (+1,2 s) : chevauchement `RequestPresetNames` — déjà noté sur `01_connect` ; ne explique pas à elle seule l’absence totale de `1d`, mais à corréler si le Stomp reste en mode « lecture preset ».

Script : `python3 scripts/analyze_stomp_running_compare.py captures/.../stomp_running_start_hxlinux.json`

### Comparaison paire `stomp_running_start_hxedit.json` (27 mai 2026)

| Métrique | HX Edit | Linux (post-fix ARM) |
|----------|---------|----------------------|
| Durée / capdata | 30,7 s / **801** | 30,7 s / **294** |
| Ordre ARM | `ARM_ed` → `ARM_f0` → `ARM_ef` | **identique** |
| Octets ARM `09:10` | identiques (cnt x1 diffère : `03` vs `04` sur `ARM_ef`) | idem |
| IN `1d` fond | **303** | **0** |
| OUT `f0` sub=`08` | **302** (lane `48:10`… après bootstrap) | **1** (bootstrap seul) |
| OUT `f0` sub=`10` | **0** | **25** (`09:10` figé) |
| Ratio ACK / `1d` | **0,997** | — |

**Aligné sur `ARM_ef` (t = 0)** :

| Δt | HX Edit | Linux |
|----|---------|-------|
| +20 ms | 1er **IN `1d`** fond (#8582) | — |
| +166 ms | — | IN courts `ef`/`ed`/`f0` sub=`08` (échos, pas `1d`) |
| +203 ms | 1er **OUT `f0/08` réactif** lane `48:10` | — |
| +218 / +239 ms | phase 4 (`19`…) | phase 4 (`19`…) |
| +976 ms | — | 1er **`f0` sub=`10` poll** (`09:10`) |

**Lecture** : les triples ARM sont la **même forme** ; l’écart décisif n’est pas l’ordre ARM mais (1) le Stomp **ne pousse jamais** les `1d` 40 o côté Linux, (2) HX Edit **ne poll jamais** `f0` sub=`10` alors que Linux le faisait ~1/s.

**Correctif code (27 mai)** : `keep_alive.rs` — boucle réduite au seul poll `ed` sub=`10` + `editor_ed03_double_val()` (plus de `ef` `72:1e` ni `f0` `09:10` figés). `live_write.rs` : prélude `f0` utilise `firmware_scroll_lane_double()`.

---

## Notes terrain

| Date | Phase | Observation |
|------|-------|-------------|
| 2026-05-26 | 0 | Reset complet ; pas de scroll actif côté host |
| 2026-05-27 | 1 | Captures idle ~23 s post-bootstrap. **HX Edit** : IN `1d`×303, OUT `f0:03` sub=`08`×302 (≈100 % des `1d` acquittés en moins de 300 ms), doubles lane qui évoluent (`48:10`, `5d:10`, …). **HXLinux** : IN `1d`×195, OUT sub=`08`×1 (bootstrap `09:10` seul), **0 %** ACK sur `1d` ; keep-alive envoie `f0:03` sub=`10` + `09:10` figé toutes les ~1 s (`keep_alive.rs`). Pas de scroll (`1f`=0 Linux, 5 HX Edit). **Ressenti utilisateur** : preset plus rapide qu’avec l’ancien code scroll ; connexion perçue plus rapide qu’HX Edit ; idle sans symptôme visible. **Action avant phase 2** : réintroduire ACK `1d`/`1f` (lane depuis capture), **sans** remettre pull ni garde-fous lourds — pour ne pas regagner la lenteur au chargement preset. |
| 2026-05-27 | 1b | **Couche ACK mini** : `firmware_scroll_ack.rs` — voir commit local / branche en cours. |
| 2026-05-27 | 1c | **Validation idle Linux V2** : `stomp_runnig_start_hxlinux_V2.json` — post-boot ~22,5 s : IN `1d`×259, OUT `f0 sub08`×258 (ratio **1,00**), 99 % ACK &lt;300 ms ; lane `0x1009`→`0x1048` (+`0x3f`) comme HX Edit. vs V1 : 1 ACK / 195 `1d`. Reste : keep-alive `sub=10`×21 (HX Edit idle = 0). |
| 2026-05-27 | 1d | **Validation terrain amorçage** : alerte front/back `debug:fond-amorcage` activée ; test connect réel sans action utilisateur = **aucune alerte**. Décision figée : fond OFF en `Bootstrapping`, ON en `EditorReady` ; filtre fond strict (`1d`/`1f` 40 o + `f0:03:02:10`). |
| 2026-05-27 | 4a | **Incident pull minimal** (ancienne capture même fichier, session bug) : 1×`1f` → OUT `1b`+`f0` puis **`19`×2 sans IN** → freeze. Pull re-désactivé. |
| 2026-05-27 | 4b | **Capture Linux passif** (`one_scroll_hxlinux.json`, ~7,5 s) : rafale t≈0,75–1,26 s — IN **`1d`×9 / `1f`×7** + **`21`×7** (44 o) ; OUT **`f0` sub=`08`×30** (ACK fond, délai 0–10 ms) ; **0×`1b` / 0×`19` / 0×272**. Stomp non figé ; baseline valide avant phase 4. |
| 2026-05-27 | 4d | **Lanes pull séparées** : `12–13` (`7e:1c`…) ≠ `28–29` (`f1:64`…) ≠ `editor_ed03_double`. Pull auto OFF ; probe terrain `HX_SCROLL_PULL_PROBE=1` (1 cran / capture, logs `[ScrollPull][probe]`). |
| 2026-05-27 | 4d | **Lanes pull séparées** : octets `12–13` (`7e:1c`…) ≠ `28–29` (`f1:64`…). Pull auto OFF ; probe `HX_SCROLL_PULL_PROBE=1` (logs `[ScrollPull][probe]`, 1 cran/capture). |
| 2026-05-27 | 4c | **1 cran lent** (`one_slow_notch_Linux.json`, 5,6 s) : **`1f`×1** isolé — séquence t≈1,75 s : `IN 1d` → OUT `f0/08` (+0 ms) → `IN 1f` (+11 ms) → OUT `f0/08` (+0 ms) → `IN 21` (+24 ms) → `IN 1d` (+0 ms) → OUT `f0/08`×2 (+10 ms) ; **0×`1b`/`19`/272**. Référence phase 4 « entrée épisode » = premier `1f` après `1d`. |
| 2026-05-27 | amorçage-fix | **Entrelacement ARM** implémenté (`send_arm_ed` / `send_arm_f0` / `send_arm_ef` + `spawn_post_arm_sequence`) — à valider capture `stomp_running_*`. |
| 2026-05-27 | amorçage | **Paires capture à ne pas mélanger** : `01_connect_*` = app ouverte → Stomp branché/allumé ; `stomp_running_*` = Stomp ON → capture → lancement app → idle ~30 s. **Fix batch `amorcage.rs` insuffisant** : sur `stomp_running_*` (procédure identique), HX Edit = ARM **entrelacé** (`ARM_ed → x11 x2 → Reconfigure ef → ARM_f0 → x11 x1 → ARM_ef`) ; Linux = Reconfigure **puis** batch ARM. **À implémenter** avant prochaine capture idle. |
| 2026-05-27 | connect | **`01_connect_HXLinux.json`** (~30 s) vs `01_connect_HXEdit.json` (~12 s) : OUT [0–6] identiques ; [7–11] entrelacement ARM manquant ; post-`ARM_ef` phase4/settle/keep-alive **alignés** — voir § *Fenêtre post-ARM_ef → phase4 → keep-alive*. |
| 2026-05-27 | fond-ack | **`firmware_scroll_ack.rs`** : ACK `1d`/`1f` dès `firmware_scroll_armed` (post `ARM_f0`), plus de gate `EditorReady` — aligné HX `stomp_running` (+20 ms `1d` avant fin phase 4). |
| 2026-05-27 | keep-alive | **`keep_alive.rs`** : fin du triplet `ed→ef→f0` hardcodé ; seul poll `ed` sub=`10` + lane éditeur dynamique (captures HX : pas de `f0/10` en idle). |
| 2026-05-27 | stomp_running (post-fix ARM) | **`stomp_running_start_hxlinux.json`** (~27 s utile, analyse agent) — voir § *Analyse stomp_running_start_hxlinux.json* ci-dessous. |
| 2026-05-27 | déconnexion/reconnexion | **Fix cycle de vie session USB** : arrêt explicite session (`session_stop` + `recv_timeout`), purge `AppState` (`preset_names`, `connected_device_name`, handles), arrêt writer sur `NoDevice`, garde-fou `helix_session_busy` pour empêcher les sessions concurrentes, événement front `helix-device-lost` + purge UI (`main.ts`, `models.ts`). **Symptômes ciblés** : bandeau “HX connecté” persistant après unplug, grille/presets non purgés, boucle `[UsbWriter] ... No such device`, reconnexion sans lecture preset. **Validation terrain en attente** (test unplug/replug réel + capture). |

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

## Piste ouverte — extraction par type de modèle et mute amp (~12 scrolls)

**Contexte (juin 2026)** : fix looper (`82 13 07` + `84 08 <id> 09`) — chemin **séparé** du pull scroll classique (`82 13 06|08` + `19…1a`). Après fix : scrolls loopers OK, slot visible, ~**80** scrolls sans mute Stomp. Sur les **ampli**, mute vers le **12ᵉ** scroll.

### Décision de design visée (juin 2026)

**Comme pour les loopers** : ne pas élargir un seul parseur monolithique — **un chemin d’extraction par type de modèle / famille de segment fil**, enchaînés par **priorité + fallback** :

```text
extract_module_hex_for_hw_scroll_dump(buf):
  1. Chemin standard (effets, amp, cab, preamp…) — 82 13 06|08 + 19…1a (+ inférences Amp+Cab)
  2. Chemin looper / fixed — 82 13 07 ou tête 07 + 84 08 <id> 09     [fait]
  3. (à découvrir) Send/Return, routing I/O, … — segment + encodage propres
  4. None → pull non finalisé (slot vide, risque accumulation mute)
```

**Règles** :

- Chaque chemin = **fonction isolée** + **tests sur capture réelle** (pas de heuristique globale).
- Un chemin ne s’active que si le **marqueur de type** est présent (`82 13 XX`, encodage id…).
- Le chemin standard reste **prioritaire** ; les autres ne sont que des **fallbacks** (ne pas casser Grammatico, Fullerton, US Small Tweed, etc.).
- Même logique côté preset (`extract_first_module_from_assignable_chunk`) pour cohérence slot scroll ↔ grille preset.

### Table des chemins (à compléter par captures)

| Famille | Fichier `.models` | Segment fil (observé) | Encodage id | Extracteur | Statut |
|---------|-------------------|------------------------|-------------|------------|--------|
| Effets, amp, cab, preamp… | `*.models` classiques | `82 13 06\|08` + `85 18 83 17` | `19 … 1a` (+ Amp+Cab combiné) | `extract_first_module_from_assignable_chunk` / chemin standard | **OK** (à préserver) |
| Looper / fixed | `fixed.models` | `82 13 07` (ou `07` après split) | `84 08 <octets> 09` | `extract_module_hex_from_looper_style_assignable` | **Fait** |
| Amp / Amp+Cab (mute ~12 ?) | `amp`, `preamp`, `cab…` | à confirmer (variants dump court, combiné fil…) | `19…1a`, token 1 o, `amp1acab` | sous-chemin ou affinage du standard | **À investiguer** |
| Send/Return | `sendreturn.models` | ? | ? (ex. `ccfa` catalogue) | chemin dédié | **À cartographier** |
| Routing I/O (split, merge…) | `io.models` | ? | ? | chemin dédié | **À cartographier** |

**Hypothèse mute amp** (à valider avant code) :

| Observation | Interprétation |
|-------------|----------------|
| Loopers ignorés avant fix | Pull **non finalisé** → pas de `module_hex` → slot vide ; pas de log `chainHexHint` |
| Amp mute ~12 scrolls | Dumps amp / Amp+Cab **mal routés** (sous-chemin manquant ou `module_hex` partiel) → post-pull / lane qui **s’accumule** |
| Loopers ~80 scrolls OK post-fix | Le **bon chemin** finalise le pull → Stomp tient |

**À faire** (incrémental, un type à la fois) :

- [ ] **Refactor cible** : router `extract_module_hex_for_hw_scroll_dump` via une liste ordonnée de `(predicate, extract)` — une entrée par type, comme le looper.
- [ ] Inventorier `82 13 XX` + encodage id par **catégorie catalogue** (scroll terrain + `HX_SCROLL_CHAINHEX=1`).
- [ ] Comparer capture scroll amp **#10–15** vs looper **#70–80** avant d’ajouter un sous-chemin amp.
- [ ] Chaque nouveau chemin : capture JSON + test unitaire Rust + régression sur les chemins déjà verts.

**Réf. code** : `lib.rs` (`extract_module_hex_from_looper_style_assignable`, `extract_module_hex_for_hw_scroll_dump`), `scroll_model_pull.rs` (`finalize_pull_capture`, `frame_carries_model_id`).

---

## Checklist rapide (jour J)

1. [x] Phase 1 — connect / idle : UX OK + **fil fond V2 ≈ HX Edit** (`stomp_runnig_start_hxlinux_V2.json`)
2. [x] Phase 2 — capture `one_scroll_hxedit.json`  
3. [~] Phase 3 — couches 1/2 + entrelacement documentés (affiner si capture 1 cran strict)  
4. [~] Stabilité session USB — **fix déconnexion/reconnexion codé**, validation terrain/capture en attente  
5. [ ] Parité pré-scroll avec HX Edit — re-capture `stomp_running_start_hxlinux.json` (post-fix) et vérification OUT [7–12] + démarrage fond idle  
6. [ ] Phase 4 — code minimal scroll + capture Linux + diff  

---

## Historique décisions

| Date | Décision | Raison |
|------|----------|--------|
| 2026-05-26 | Supprimer toute couche scroll legacy | Empilement de patches non prouvés ; freezes et doublons UI |
| 2026-05-26 | Pas d’ACK `1d`/`1f` jusqu’à preuve capture | Fil USB vierge pour analyse |
| 2026-06-10 | **Un chemin d’extraction par type de modèle** (modèle looper) | Fallbacks isolés (`82 13 07` + `84 08…09`) au lieu d’élargir le parseur standard ; slot visible + scroll stable ; même approche prévue amp / Send/Return / I/O |
