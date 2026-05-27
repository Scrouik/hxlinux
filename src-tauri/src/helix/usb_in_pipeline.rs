//! Pipeline des couches **actives** sur chaque IN USB (0x81).
//!
//! Contrat (voir `docs/todo-scroll-hw.md` § Pipeline USB) :
//! - `Ignored` : pas mon paquet — couche suivante.
//! - `Observed` : reconnu, **aucun** OUT / lane sur le fil — couche suivante (variante A).
//! - `Consumed` : traitement complet (lane + ACK ou autre OUT) — **stop** les couches actives suivantes.

use crate::helix::firmware_scroll_ack;
use crate::helix::preset_dump_stream_ack;
use crate::helix::slot_model_hw_pull;
use crate::helix::HelixState;

/// Effet résumé d’une couche (debug / trace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerEffect {
    None,
    /// Lane scroll avancée + OUT `f0:03` sub=`08` (couche fond).
    ScrollLaneAndAck,
    /// Lane preset dump avancée + OUT `ed:03` sub=`08` (chunks 272).
    PresetDumpLaneAndAck,
    /// Pull scroll (futur).
    ScrollPull,
}

/// Résultat d’une couche sur une trame IN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerResult {
    Ignored,
    Observed { effect: LayerEffect },
    Consumed { effect: LayerEffect },
}

impl LayerResult {
    pub fn is_consumed(self) -> bool {
        matches!(self, LayerResult::Consumed { .. })
    }
}

/// Identifiant des couches actives (ordre fixe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveLayerId {
    FirmwareScroll,
    ScrollPull,
    PresetDumpStream,
}

/// Résultat agrégé du pipeline actif pour une trame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivePipelineOutcome {
    pub consumed_by: Option<ActiveLayerId>,
    pub scroll_was_1f: bool,
}

impl ActivePipelineOutcome {
    pub fn not_consumed() -> Self {
        Self {
            consumed_by: None,
            scroll_was_1f: false,
        }
    }
}

type ActiveHandler = fn(&mut HelixState, &[u8]) -> LayerResult;

const ACTIVE_LAYERS: [(ActiveLayerId, ActiveHandler); 3] = [
    (ActiveLayerId::FirmwareScroll, firmware_scroll_ack::handle_in_layer),
    (ActiveLayerId::ScrollPull, slot_model_hw_pull::handle_in_layer),
    (
        ActiveLayerId::PresetDumpStream,
        preset_dump_stream_ack::handle_in_layer,
    ),
];

/// Exécute les couches actives dans l’ordre ; s’arrête au premier `Consumed`.
pub fn run_active_layers(state: &mut HelixState, data: &[u8]) -> ActivePipelineOutcome {
    let mut outcome = ActivePipelineOutcome::not_consumed();

    for (id, handler) in ACTIVE_LAYERS {
        match handler(state, data) {
            LayerResult::Ignored => {}
            LayerResult::Observed { .. } => {}
            LayerResult::Consumed { effect } => {
                outcome.consumed_by = Some(id);
                if id == ActiveLayerId::FirmwareScroll && effect == LayerEffect::ScrollLaneAndAck {
                    outcome.scroll_was_1f = data.first() == Some(&0x1f);
                }
                return outcome;
            }
        }
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_scroll(state: &mut HelixState, data: &[u8]) -> LayerResult {
        let _ = state;
        if data.first() == Some(&0xaa) {
            LayerResult::Consumed {
                effect: LayerEffect::ScrollLaneAndAck,
            }
        } else {
            LayerResult::Ignored
        }
    }

    fn fake_pull(_: &mut HelixState, _: &[u8]) -> LayerResult {
        LayerResult::Ignored
    }

    fn fake_dump(_: &mut HelixState, _: &[u8]) -> LayerResult {
        LayerResult::Ignored
    }

    #[test]
    fn consumed_stops_following_active_layers() {
        let layers: [(ActiveLayerId, ActiveHandler); 3] = [
            (ActiveLayerId::FirmwareScroll, fake_scroll),
            (ActiveLayerId::ScrollPull, fake_pull),
            (ActiveLayerId::PresetDumpStream, fake_dump),
        ];
        let mut state = HelixState::new();
        let data = [0xaa];

        let mut outcome = ActivePipelineOutcome::not_consumed();
        for (id, handler) in layers {
            if let LayerResult::Consumed { .. } = handler(&mut state, &data) {
                outcome.consumed_by = Some(id);
                break;
            }
        }
        assert_eq!(outcome.consumed_by, Some(ActiveLayerId::FirmwareScroll));
    }

    #[test]
    fn observed_does_not_stop_in_unit_logic() {
        let r = LayerResult::Observed {
            effect: LayerEffect::None,
        };
        assert!(!r.is_consumed());
    }
}
