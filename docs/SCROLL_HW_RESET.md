# Reset scroll modèle HW (mai 2026)

## Décision

Toute la couche scroll modèle (pull, ACK lane, pending, quarantaine, garde-fous preset)
a été **retirée** du code actif pour repartir depuis les captures HX Edit.

Historique git : commits `ac6cfb9` (stub pull) puis purge complète (champs `HelixState`,
ACK `1d`/`1f`, heuristiques `21`).

## État actuel

| Composant | Comportement |
|-----------|----------------|
| `usb_in_pipeline.rs` | Couches actives IN : fond scroll → pull (stub) → ACK dump 272 |
| `usb_listener.rs` | `run_usb_in_active_layers` (plus d’ACK scroll/dump en direct) |
| `firmware_scroll_ack.rs` | Couche 1 — `handle_in_layer` : ACK `1d`/`1f` (lane scroll, sans pull) |
| `slot_model_hw_pull.rs` | Couche 2 — `handle_in_layer` → `Ignored` ; `ingest` → `None` |
| `HelixState` | `firmware_scroll_ack_*` uniquement (pas de pull) |
| UI models | Molette Stomp **ne met pas à jour** le modèle affiché |

Le handshake connect envoie toujours `f0:03` sub=08 avec `09:10` en dur (`connect.rs`) —
indépendant de la future lane scroll.

## Captures

`captures/usb-wireshark/` — référence : `3_scroll_HXEdit.json`.

## Suite du travail

Contrat pipeline : **`todo-scroll-hw.md` § Pipeline USB**. Pull scroll **non réactivé** tant que les règles `1f` ne sont pas calées sur capture.

Feuille de route détaillée (phases, critères, checklist) : **[`todo-scroll-hw.md`](todo-scroll-hw.md)**.
