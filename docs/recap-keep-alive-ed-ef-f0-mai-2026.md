# Récap — keep-alive USB `ed → ef → f0`, handler Kempline, canal `f0:03` (mai 2026)

Document de continuité : ce qui a été fait, pourquoi, hypothèses, état actuel, pistes.

**Voir aussi** : [USB Slot Model Fixes](usb-slot-model-fixes.md) — autre chantier (mai 2026) sur **`probe_slot_model_usb`**, profils `ed:03` (`80:10` vs `03:10`), lane dynamique, ressources `HX_ModelUsbAssign.json` / Distortion mono. Ce récap-ci porte surtout sur le **keep-alive global** `ed→ef→f0`, `connect.rs` et `standard.rs`.

**Mise à jour 18 mai 2026** : le silence `f0:03` et la non-sync slot hardware → UI sont **résolus** (voir §4).

---

## Contexte général

HXLinux parle au Helix/Stomp en USB (HID + trames 16 octets sur l’endpoint concerné). Pour rester synchronisé avec le firmware (comme HX Edit), il faut une **boucle keep-alive** cohérente et des handlers qui **ne réinventent pas** des réponses que le device n’attend pas.

Problèmes traités dans cette série de changements :

1. **Compteurs et trames** mélangés entre plusieurs chemins (threads / handlers).
2. Un handler issu de **Kempline** (`standard.py`) qui **polluait** le bus et cassait la séquence.
3. **Confusion** entre trames keep-alive `ed` (`00:10`) et trames **live write** `ed` (`00:08` + `live_write_ctr`).
4. **Silence du Stomp sur `f0:03`** — **résolu le 18 mai 2026** (poll d’activation `sub=08`, compteur `x2`, délai post-bootstrap).

---

## Ce qui a été fait (concret)

### 1. Keep-alive unifié — `ed → ef → f0`

**Fichiers** : `keep_alive.rs`, intégration dans `mod.rs` / `lib.rs`, arrêt des anciens démarrages redondants (ex. plus de keep-alive lancé sur certains événements `x11` dans `connect.rs` si unifié), enchaînement après reconfigure / bootstrap (`reconfigure_x1.rs`, `editor_phase4_bootstrap.rs`).

**Pourquoi** : Les captures HX Edit montrent un **cycle ordonné** entre opcodes, pas trois rythmes indépendants qui se marchent dessus. Des compteurs `x80` / `x1` / `x2` incohérents entre trames successives peuvent mettre le device dans un état où il **ignore** une étape (souvent vue autour de `f0`).

**Détail important sur `ed`** : la queue après `00:10` n’est **pas** une constante type `ee:1c` figée. Le code utilise `session_no` + `preset_data_packet_double()` (+ trailing `0x00`), aligné avec le reste du protocole preset, comme l’ancien keep-alive `x80`.

**Paramètres** : cycle ~1040 ms, courte pause (~28 ms) entre chaque OUT du triplet.

### 2. Suppression / neutralisation du handler « fin de transfert preset variante (0x10) »

**Fichier** : `src-tauri/src/helix/modes/standard.rs`.

**Ancien comportement (bug)** : Un `if` matchait un **IN** device de 16 octets du type  
`08…ed:03:80:10…00:10…` — **exactement** l’ACK normal du poll `80:10:ed:03` avec `00:10` en position 11. Le handler appelait `next_x80_cnt()` et envoyait un **OUT parasite** `80:10:ed:03` avec **`00:08`** et echo partiel des octets du IN.

**Pourquoi c’était faux** : Dans `connect_device_30s_HXEdit.json`, après ce type d’IN, HX Edit envoie le **prochain poll** avec encore **`00:10`** et la queue habituelle (`ee:1c:00:00` dans cette capture), **pas** un `00:08` « acquittement ».

**Correction** : Les deux formes courtes (`00:08` et `00:10` en zone longueur / sous-champ, suffixe `.. .. 00 00`) sont traitées comme **ACK x80 à ignorer** (`return false`), sans `send()`. Plus de consommation de compteur `x80` ni de trame supplémentaire entre `ef` et `f0`.

### 3. Clarifications de provenance des trames (debug / usbmon)

- **`switch_active_hardware_slot`** et sondes focus dans `lib.rs` : paquets **`1d…`** avec **`00:04`**, pas le petit `08…16` — **pas** la source des `08…ed:03` + `00:08`.
- **`live_write.rs`** : seul endroit qui construit systématiquement le **16 o** `08…80:10:ed:03` avec **`00:08`** et le compteur live (`live_write_ctr`).
- **`standard.rs`** (autres branches) : réponses réactives (ex. LED) avec `00:08` + `session_quadruple`, distinct du keep-alive.

### 4. Activation du canal `f0:03` — poll `sub=08`, compteur `x2`, délai post-bootstrap (18 mai 2026)

**Symptôme** : après les commits « keep-alive unifié » + fix ACK `ed`, le subscribe/handshake `f0` passait (IN `0c:28` et `11:18` OK), mais **aucune** réponse aux polls réguliers `08…02:10:f0:03` avec **`sub=10`** ; l’UI ne suivait pas le changement de slot sur le hardware.

**Analyse** (captures `connect_device_30s_HXEdit.json` vs `connect_device_30s_Linux.json`) :

| Étape | HX Edit | Linux (avant fix) |
|-------|---------|-------------------|
| Subscribe `0c:28` | identique | identique |
| Handshake `11:18` cnt **0x02** | identique | identique |
| **Poll activation** `08:16` **sub=08** cnt **0x03** | #3255 (~15 ms après handshake) | **absent** (commentaire « pas de poll ici » dans `connect.rs`) |
| Bootstrap phase 4 (`1a ef` + `cc:fe`) | #3447 | ~#271 |
| Délai avant 1er poll `sub=10` | **~688 ms** | **~28 ms** (pendant dump preset sur `0x81`) |
| 1er poll régulier cnt / sub | **0x04** / **10** | **0x02** / **10** (double compteur) |

**Trois correctifs** :

1. **`connect.rs`** — après handshake `f0` (`x11` sur x2), envoi du poll d’**activation** :
   `08:00:00:18:02:10:f0:03:00:<cnt>:00:08:09:10:00:00`  
   (HX Edit #3255 ; `OutPacket::with_delay` **15 ms**).

2. **`connect.rs`** — handshake ack : `state.x2_cnt` → **`state.next_x2_cnt()`** pour que l’activation utilise **0x03** et non un second **0x02** (`next_x2_cnt` retourne la valeur courante puis incrémente).

3. **`keep_alive.rs`** — **`POST_PHASE4_SETTLE_MS = 700`** : `thread::sleep` une fois au démarrage du thread keep-alive, **avant** la première boucle `ed→ef→f0` (HX #3447 → #3761 ≈ 688 ms).

**Validation** :

- IN `f0:03:02:10` sur le poll d’activation (`sub=08`) et sur les polls réguliers (`sub=10`) ; compteurs alignés requête/réponse (03, 04, 05…).
- **Changement de slot sur le Helix reflété dans l’UI** sans capture supplémentaire obligatoire.
- Subscribe/handshake : payloads **byte à byte identiques** à HX Edit — le problème n’était **pas** le negotiate `0c:28`.

**Séquence cible** :

```
subscribe → handshake (cnt 02) → poll activation (cnt 03, sub 08) → IN f0 16o
→ phase 4 bootstrap → dump preset → sleep 700ms
→ boucle keep-alive ed / ef / f0 (sub 10) → IN f0 16o (+ 44o si changement slot HW)
```

---

## Hypothèses validées ou infirmées

| Hypothèse | Verdict |
|-----------|--------|
| L’ACK `ed` device doit recopier byte à byte la queue OUT (`ee:1c` vs `bc:39` etc.) | **Non** — HX Edit : OUT `ee:1c`, IN `c1:02` ; même **compteur** `b[9]`, queues différentes. |
| Les `a6:02` / `30:02` (ACK) vs `ee:1c` (HX) indiquent à eux seuls une erreur d’« adresse » | **Faible** — ce sont surtout des **handles / session** différents ; le préfixe `…:02` en bas de mot est une famille vue sur plusieurs ACK. |
| Le handler « variante 0x10 » cassait le cycle `ed→ef→f0` | **Oui** — trame supplémentaire, mauvais sous-champ `00:08`, avance du compteur `x80`. |
| Le silence `f0` est expliqué uniquement par `bc:39` ≠ `c1:02` | **Non démontré** — même OUT HX utilise `09:10` sur `f0` ; l’IN HX a `09:02`, écart **côté device** sur l’ACK, pas la cause du silence total. |
| Le subscribe `f0` incorrect empêche le polling régulier | **Non** — negotiate et handshake **identiques** à HX Edit ; il manquait le poll **`sub=08`** + timing + compteur. |
| Retarder le keep-alive après bootstrap aide le 1er `f0` | **Oui** — ~700 ms évite d’envoyer `f0` pendant la rafale dump preset sur `0x81`. |

---

## Situation actuelle (18 mai 2026)

- **Cycle logiciel** : `ed → ef → f0` sans parasite depuis `standard.rs` ; keep-alive aligné sur les captures HX Edit.
- **Canal `f0:03`** : poll d’activation puis polling `sub=10` ; IN `f0:03:02:10` **16 o** reçus ; dialogue synchrone (compteurs requête = réponse).
- **Produit** : **sync slot hardware → UI** validée en usage (changement de slot sur le boîtier).
- **Captures** : `src/Paquets Json/connect_device_30s_HXEdit.json` (référence) ; `connect_device_30s_Linux.json` post-fix (local, non versionné).
- **Reste ouvert** : un **petit bug mineur** à traiter en session suivante (non documenté ici) ; notifications **44 o** `21…f0:03` (focus slot / détail slot) à confirmer au besoin sur capture ciblée.

---

## Pistes (état)

**Traitées (18 mai 2026)** :

1. Bootstrap / timing premier `f0` post phase 4 → `POST_PHASE4_SETTLE_MS`.
2. État « éditeur prêt » / poll d’activation → `sub=08` dans `connect.rs`.
3. Compteur `x2` handshake vs activation → `next_x2_cnt()` sur le handshake.
4. Timing immédiat après handshake → délai 15 ms sur le poll `08`.
5. Endpoint `0x81` pour les IN — vérifié sur captures post-fix.

**Encore ouverts** :

- Détail du bug mineur restant (session suivante).
- Parser / focus : trames **44 o** `82:62:SS:1a` hors keep-alive pur (voir `slot_focus_in.rs`, `description.md` §12 mai).
- Document voisin : `docs/usb-slot-model-fixes.md` pour le fil **slot / modèle**.

---

## Fichiers clés

| Fichier | Rôle |
|---------|------|
| `src-tauri/src/helix/modes/connect.rs` | Subscribe/handshake `f0` ; **poll activation `sub=08`** ; `next_x2_cnt` sur handshake. |
| `src-tauri/src/helix/keep_alive.rs` | Boucle `ed→ef→f0` ; `POST_PHASE4_SETTLE_MS` ; tail `ed` dynamique. |
| `src-tauri/src/helix/modes/standard.rs` | Handlers IN ; ACK `ed` courts en pass silencieux. |
| `src-tauri/src/helix/live_write.rs` | Pré/post `ed` avec `00:08` (UI), pas keep-alive. |
| `src-tauri/src/helix/editor_phase4_bootstrap.rs` | Séquence `19 ed` ×3 + `1a ef` avant polling stable. |
| `src-tauri/src/helix/mod.rs` | `ingest_hw_slot_notify_in` (`82 62 SS 1a`) → événement UI slot. |
| `src/Paquets Json/connect_device_30s_HXEdit.json` | Référence HX Edit (#3243 handshake → #3255 activation → #3761 1er poll `sub=10`). |

Les exports Wireshark sous `src/Paquets Json/` restent en général **hors dépôt** (fichiers locaux volumineux).

---

## Note méthodologique

Quand une règle vient de **Kempline / `standard.py`**, la validation par **capture HX Edit** (ordre des trames, sens `send`/`recv`, valeurs dynamiques) a montré son utilité : plusieurs « acquittements » étaient des **faux positifs** sur des ACK normaux du poll. La suppression du poll `sub=08` dans `connect.rs` était une **régression** : le subscribe accepté ne suffit pas à armer le polling régulier.

---

*Rédigé pour reprise après absence — mai 2026 ; résolution canal `f0:03` documentée le 18 mai 2026.*
