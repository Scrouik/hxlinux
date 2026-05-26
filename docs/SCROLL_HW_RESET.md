# Reset scroll modèle HW (mai 2026)

## Décision

Toute la couche scroll modèle (pull, ACK lane, pending, quarantaine, garde-fous preset)
a été **retirée** du code actif pour repartir depuis les captures HX Edit.

Historique git : commits `ac6cfb9` (stub pull) puis purge complète (champs `HelixState`,
ACK `1d`/`1f`, heuristiques `21`).

## État actuel

| Composant | Comportement |
|-----------|----------------|
| `slot_model_hw_pull.rs` | Type payload + `ingest` → `None` uniquement |
| `usb_listener` | Plus d’ACK scroll sur `1d`/`1f` |
| `HelixState` | Plus de champs `hw_model_*` |
| UI models | Molette Stomp **ne met pas à jour** le modèle affiché |

Le handshake connect envoie toujours `f0:03` sub=08 avec `09:10` en dur (`connect.rs`) —
indépendant de la future lane scroll.

## Captures

`captures/usb-wireshark/` — référence : `3_scroll_HXEdit.json`.

## Prochaine implémentation

1. Analyser **un** scroll HX Edit (ordre IN/OUT, compteurs, délais).
2. Réintroduire lane + ACK + pull dans un module dédié, sans réutiliser l’ancien design.
3. Tests replay binaire sur la capture avant branchement UI.
