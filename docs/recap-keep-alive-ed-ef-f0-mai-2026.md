# Récap — keep-alive USB `ed → ef → f0`, handler Kempline, silence `f0` (mai 2026)

Document de continuité après une semaine d’absence : ce qui a été fait, pourquoi, hypothèses, état actuel, pistes.

---

## Contexte général

HXLinux parle au Helix/Stomp en USB (HID + trames 16 octets sur l’endpoint concerné). Pour rester synchronisé avec le firmware (comme HX Edit), il faut une **boucle keep-alive** cohérente et des handlers qui **ne réinventent pas** des réponses que le device n’attend pas.

Problèmes traités dans cette série de changements :

1. **Compteurs et trames** mélangés entre plusieurs chemins (threads / handlers).
2. Un handler issu de **Kempline** (`standard.py`) qui **polluait** le bus et cassait la séquence.
3. **Confusion** entre trames keep-alive `ed` (`00:10`) et trames **live write** `ed` (`00:08` + `live_write_ctr`).
4. **Silence du Stomp sur `f0:03`** (pas d’IN `f0:03:02:10` côté Linux alors que HX Edit en a) — **non résolu** à la date de ce document.

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

---

## Hypothèses validées ou infirmées

| Hypothèse | Verdict |
|-----------|--------|
| L’ACK `ed` device doit recopier byte à byte la queue OUT (`ee:1c` vs `bc:39` etc.) | **Non** — HX Edit : OUT `ee:1c`, IN `c1:02` ; même **compteur** `b[9]`, queues différentes. |
| Les `a6:02` / `30:02` (ACK) vs `ee:1c` (HX) indiquent à eux seuls une erreur d’« adresse » | **Faible** — ce sont surtout des **handles / session** différents ; le préfixe `…:02` en bas de mot est une famille vue sur plusieurs ACK. |
| Le handler « variante 0x10 » cassait le cycle `ed→ef→f0` | **Oui** — trame supplémentaire, mauvais sous-champ `00:08`, avance du compteur `x80`. |
| Le silence `f0` est expliqué uniquement par `bc:39` ≠ `c1:02` | **Non démontré** — même OUT HX utilise `09:10` sur `f0` ; l’IN HX a `09:02`, écart déjà **côté device**. |

---

## Situation actuelle (à ton retour)

- **Cycle logiciel** : `ed → ef → f0` sans parasite depuis `standard.rs` ; keep-alive aligné sur l’idée des captures HX Edit.
- **Traces Linux** (`Preset_Test_Slot7_to_6_to_5_to_4_to_3_HXLinux.json`) : **IN** sur `ed` et `ef` présents sur `0x81` ; **aucun** `f0:03:02:10` entrant dans le grep — le Stomp **n’émet pas** (ou pas capté) la petite notification `f0` alors que les OUT `02:10:f0:03` partent bien.
- **Objectif produit** non atteint si tu attends la **notification de slot** via ce canal : il reste un **écart comportemental Linux vs HX Edit** à expliquer.

---

## Pistes à explorer (priorité indicative)

1. **Bootstrap / phase 4** — Comparer **byte à byte** le tout premier cycle `ed→ef→f0` après `editor_phase4_bootstrap::send` avec une capture HX Edit au même stade (compteurs `x80`/`x1`/`x2`, `session_no`, `preset_data_packet_double()`).
2. **État « éditeur prêt »** — Le firmware pourrait ne publier les IN `f0` qu’après une séquence ou un flag non encore rejoué sous Linux.
3. **Lecture IN `0x81`** — Vérifier qu’aucun chemin ne **vide** ou ne **retarde** les URB IN au moment où le device répondrait à `f0` (race avec le thread keep-alive vs dispatcher principal).
4. **Compteur `x2` seul** — Vérifier si le premier `f0` après connexion doit réutiliser une valeur apprise d’un IN précédent (captures HX autour des tout premiers `f0`).
5. **Timing** — Augmenter légèrement le délai entre `ef` et `f0` ou entre cycles pour tester un hypothesis « pile USB / firmware » (faible coût, résultat parfois instructif).
6. **Ne pas confondre** — S’assurer que les tests usbmon incluent bien **endpoint `0x81`** pour les IN (les complétions OUT `0x01` peuvent avoir `data_len: 0` sous usbmon alors que les données utiles sont sur IN).
7. **Document voisin** — `docs/usb-slot-model-fixes.md` pour le fil **slot / modèle** (autre facette du même transport `ed:03`).
8. **x2 ack générique**
if byte_cmp(data, &pattern![
    0x17, 0x00, 0x00, 0x18,
    0xf0, 0x03, 0x02, 0x10, ...
], 16) {
    state.send(OutPacket::with_delay(vec![
        0x08, 0x00, 0x00, 0x18,
        0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08,
        0x74, 0x77, 0x00, 0x00,
    ], 10));

---

## Fichiers clés à rouvrir

| Fichier | Rôle |
|---------|------|
| `src-tauri/src/helix/keep_alive.rs` | Boucle `ed→ef→f0`, construction dynamique du tail `ed`. |
| `src-tauri/src/helix/modes/standard.rs` | Handlers IN ; ACK `ed` courts en pass silencieux. |
| `src-tauri/src/helix/live_write.rs` | Pré/post `ed` avec `00:08` (UI), pas keep-alive. |
| `src-tauri/src/helix/editor_phase4_bootstrap.rs` | Séquence avant polling stable. |
| `src/Paquets Json/connect_device_30s_HXEdit.json` | Référence comportement HX Edit (dont IN `f0:03:02:10`). |
| `src/Paquets Json/Preset_Test_Slot7_to_6_to_5_to_4_to_3_HXLinux.json` | Référence Linux récente (sans IN `f0`). |

---

## Note méthodologique

Quand une règle vient de **Kempline / `standard.py`**, la validation par **capture HX Edit** (ordre des trames, sens `send`/`recv`, valeurs dynamiques) a montré son utilité : plusieurs « acquittements » étaient des **faux positifs** sur des ACK normaux du poll.

---

*Rédigé pour reprise après absence — mai 2026.*
