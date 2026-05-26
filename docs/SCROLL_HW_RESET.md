# Reset scroll modèle HW (mai 2026)

## Décision

La couche `slot_model_hw_pull` (pull `1b`/`19`/272, pending, quarantaine, settling)
est **désactivée** pour repartir sur une implémentation calquée sur **HX Edit**.

Checkpoint git juste avant ce reset : voir le commit
`chore: checkpoint scroll HW WIP before HX Edit replay reset`.

## État actuel du code

| Composant | Comportement |
|-----------|----------------|
| `ack_hw_model_scroll_in` | ACK `f0 sub=08` sur `1d`/`1f` (lane scroll conservée) |
| `ingest_slot_model_hw_in` | **Ne fait rien** — pas de pull, pas d’event `models:slot-model-changed` |
| UI models | Scroll molette Stomp **ne met plus à jour** le modèle affiché |

## Captures de référence

`captures/usb-wireshark/` — ex. `3_scroll_HXEdit.json` (un scroll, ~72 ms).

## Prochaine implémentation (spec)

1. Analyser **un** scroll HX Edit (ordre exact IN/OUT, compteurs, délais).
2. Machine à états minimale : pas de `pending`, pas de double pull.
3. UI : un seul `models:slot-model-changed` à la fin (hex depuis 272, règle à définir).
4. Tests : replay binaire contre la capture avant branchement UI.
