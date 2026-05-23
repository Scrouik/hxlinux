# Synchronisation matérielle — écran Models

Ce document décrit ce qui reste actif après le retrait du **poll soft-sync toutes les 200 ms** (mai 2026), pourquoi on le garde, et comment le retirer ou le réactiver.

## Ce qui a été retiré

- `setInterval(..., 200)` → `scheduleHardwareSyncPoll()` en boucle.
- **Pourquoi** : les trames USB alimentent déjà `models:hardware-slot-changed`, `models:slot-model-changed`, `models:slot-param-changed` et `hwUi` ; le tick ajoutait des `invoke` en parallèle des gestes HW (freeze webview) sans apporter d’info nouvelle dans le flux normal.

## Ce qui reste (et pourquoi)

| Mécanisme | Où | Rôle |
|-----------|-----|------|
| **`runHardwareSyncSoftRefresh()`** | `src/models.ts` | Orchestration **ponctuelle** : slot HW actif, focus USB, flush live-write, soft-refresh params depuis snapshot, option re-dump preset. |
| **`scheduleHardwareSyncFromEvent()`** | idem | Appelée sur **`models:hardware-slot-changed`** uniquement. |
| **`refresh()`** (300 ms) | idem | **Changement de preset** sur le device (`get_active_preset` ≠ index UI). |
| **`hwUi`** (`src/hwUiRefresh.ts`) | debounce UI | Modèle : aperçu léger à chaque event ; catalogue + picker + params au settle (~200 ms). |
| **Events USB** | `usb_listener.rs` → front | Modèle, param, slot actif. |
| **Poll USB preset optionnel** | `startOptionalUsbPresetPollTimer()` | Timer **uniquement** si `models_hw_usb_preset_poll_ms` est défini. |

La **grille** n’est toujours pas re-parse entre deux `request_preset_content` sauf chargement utilisateur, poll USB optionnel, ou `models_hw_force_preset_dump_on_slot_notify=1`.

## Fichiers clés

- `src/models.ts` — `scheduleHardwareSyncFromEvent`, `runHardwareSyncSoftRefresh`, `startOptionalUsbPresetPollTimer`
- `src/hwUiRefresh.ts` — debounce affichage params / grille rapide
- `src-tauri/src/helix/usb_listener.rs` — émission des events `models:*`

## Réglages `localStorage` (console navigateur, écran Models)

| Clé | Défaut | Effet |
|-----|--------|--------|
| `models_hw_sync_interval_ms` | *(absent = 0)* | Throttle **entre deux** soft-sync event (ms). Ex. `200` limite la fréquence si le Helix spamme les notifs slot. `0` = pas de throttle. |
| `models_hw_usb_preset_poll_ms` | *(absent = off)* | Active un **timer dédié** (ex. `2500`) pour `request_preset_content` périodique → grille alignée sur RAM device. |
| `models_hw_force_preset_dump_on_slot_notify` | off | Sur changement slot HW, force encore un dump preset immédiat (ancien comportement, debug). |
| `models_hw_slot_focus_usb` | on | `sync_hardware_slot_focus_usb` après notif slot (capsule → `models:slot-content-changed`). |
| `models_debug_sync_trace` | off | Logs `[ModelsSync]`. |
| `models_debug_heavy_ui` | off | Durée des jobs lourds `hwUi`. |
| `models_debug_hw_model_fast` | off | Aperçu scroll vs settle (catalogue / picker / params) sur `slot-model-changed`. |

## Réactiver un poll périodique (legacy)

Si un scénario sans event suffisant réapparaît (preset édité sur le device sans trames reconnues) :

1. Remettre un `setInterval` qui appelle `runHardwareSyncSoftRefresh()` (pas `scheduleHardwareSyncFromEvent` — même garde `gestureInProgress`).
2. Ou activer `models_hw_usb_preset_poll_ms` pour un filet **lent** (secondes), pas 200 ms.
3. Documenter le cas dans ce fichier.

**Ne pas** remettre 200 ms en parallèle de `hwUi` sans mesurer la charge — c’était la cause des freezes en rafale.

## Comment retirer complètement le soft-sync plus tard

Ordre suggéré (quand les events couvrent 100 % des besoins) :

1. Déplacer le corps utile de `runHardwareSyncSoftRefresh` dans le listener `models:hardware-slot-changed` (focus USB, `applyHardwareSlotStateFromBackend`).
2. Supprimer `scheduleHardwareSyncFromEvent` et la fonction `runHardwareSyncSoftRefresh`.
3. Garder `refresh()` pour le preset index.
4. Garder `hwUi` + listeners `slot-model-changed` / `slot-param-changed`.
5. `cargo test` + test manuel : changement slot HW, scroll modèle, knobs, changement preset sur pédale.

## Plantage HW au scroll modèle (pull incomplet)

Si le modèle s’affiche (`"cdXXXX"; "Nom"`) puis le HW freeze : finalize **avant** le bulk IN ~272 o après `19` #2. Correctif : `hw_model_pull_saw_final_bulk` — `post-pull` seulement après ce 272 o (HX Edit).

L’IN **`0x21` 44 o** post-assign (stomp « modèle enregistré ») est **unidirectionnel** : pas d’ACK host (`ack_hw_model_scroll_in` ignore ; pas d’événement `hardware-slot-changed`). Les **`1f`** 40 o reçoivent `f0:03` sub=08 ; les **`1d`** aussi **sauf** pendant un pull actif ou ~85 ms après finalize (`hw_model_pull_quiet_until`). Ne pas traiter un bulk **272 o** comme 1ʳᵉ réponse au `1b` (`looks_like_first_pull_reply` : tête `53`/`56` ou ~92 o, pas `len>=68`). Pull pending : flush seulement après la fenêtre quiet (évite enchaînement `1b` sur 272 tardif).

## Tests manuels rapides

- Changer de slot sur le Helix → sélection + params cohérents.
- Scroller plusieurs modèles → pas de freeze prolongé.
- Changer de preset sur le Helix → `refresh()` recharge l’UI.
- Avec `models_hw_usb_preset_poll_ms=2500` → grille se resync sans action UI (après ~2,5 s).
