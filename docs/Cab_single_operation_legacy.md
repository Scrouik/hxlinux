# Cab single Legacy — USB param write

**HXLinux — HX Stomp XL**  
*Standalone **Cab / Single / Legacy** slot (`c2:19`, assign `cd:03:ff` ou `cd:04:…`).*

> **Ne pas confondre** avec [Amp_cab_operation_legacy.md](Amp_cab_operation_legacy.md) : ce doc décrit un slot **Cab seul** legacy, pas une paire Amp+Cab (`c319`).

**Captures :** `captures/usb-wireshark/Save/one_slow_notch_HXEdit.json`, `split scroll.json`, `cab single legacy.json`

---

## 1. Résumé

| | Cab single legacy | Amp+Cab legacy |
|--|-------------------|----------------|
| Marqueur wire | `c2:19` | `c3:19` dans le bulk |
| Assign typique | `cd:03:ff` (Soup Pro) ou `cd:04:00` | `cd:08` + tables `0x25+` |
| Focus / commit | `1b` + handshake `19`/`272` | `1b` `cd:08` (voir doc Amp+Cab) |
| OUT `57` sur write param | **Non** (IN scroll seulement) | — |

HX Edit **n’envoie pas** de trame OUT `57` pour modifier un param sur Cab single legacy. Le commit passe par :

```
OUT 1b (cd:03:fe…) → OUT f0 → IN dump (53/3c/39…)
→ OUT 19 (cd:03:ff) → IN 21 (44 o)
→ OUT 19 (cd:04:pSel) → OUT f0
→ IN 272×N + ACK OUT 08 ed:03 (couche preset_dump_stream_ack)
```

HXLinux : `legacy_cab_param_commit.rs` (handshake **asynchrone** — ne pas envoyer la séquence en burst).

---

## 2. Code

| Fichier | Rôle |
|---------|------|
| `legacy_cab_param_commit.rs` | Handshake `cd:03:ff` Soup Pro |
| `amp_cab_live_write.rs` | Blocs modèle, route `legacy_cab:`, minimal `cd:04` |
| `lib.rs` | `write_live_param` → `start_standalone_legacy_cd03ff_write` |

Debug : `HX_LEGACY_CAB_COMMIT_DEBUG=1`

---

## 3. Checklist

- [ ] Assign legacy (`probe_slot_model_usb`) avant premier write param
- [ ] Log `legacy_cab=cd03ff_handshake` (pas `packets=8` burst)
- [ ] Mic / params réagissent sur HW après handshake complet
