//! Pipeline des couches **actives** sur chaque IN USB (0x81).
//!
//! Contrat (voir `docs/todo-scroll-hw.md` § Pipeline USB) :
//! - `Ignored` : pas mon paquet — couche suivante.
//! - `Observed` : reconnu, **aucun** OUT / lane sur le fil — couche suivante.
//! - `Consumed` : traitement complet (lane + ACK ou autre OUT) — **stop** les couches actives suivantes.

use crate::helix::firmware_scroll_ack;
use crate::helix::legacy_cab_param_commit;
use crate::helix::matrix_routing_dd;
use crate::helix::preset_dump_stream_ack;
use crate::helix::scroll_model_pull;
use crate::helix::HelixState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerEffect {
    None,
    ScrollLaneAndAck,
    PresetDumpLaneAndAck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerResult {
    Ignored,
    #[allow(dead_code)]
    Observed { effect: LayerEffect },
    Consumed { effect: LayerEffect },
}

impl LayerResult {
    #[allow(dead_code)]
    pub fn is_consumed(self) -> bool {
        matches!(self, LayerResult::Consumed { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveLayerId {
    MatrixRoutingDd,
    ScrollModelPull,
    LegacyCabParamCommit,
    FirmwareScroll,
    PresetDumpStream,
}

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

fn scroll_model_pull_handler(state: &mut HelixState, data: &[u8]) -> LayerResult {
    scroll_model_pull::handle_in_layer_trigger(data, state)
}

fn matrix_routing_dd_handler(state: &mut HelixState, data: &[u8]) -> LayerResult {
    if state.matrix_routing_dd_wait.is_none() {
        return LayerResult::Ignored;
    }
    if !matrix_routing_dd::is_routing_dd_pipeline_in(data) {
        return LayerResult::Ignored;
    }
    let _ = matrix_routing_dd::try_notify_routing_dd_in(state, data);
    // Bloque scroll / Standard / dump sur IN dialogue + échos `ed:03` sub=`08` device.
    LayerResult::Consumed {
        effect: LayerEffect::None,
    }
}

fn legacy_cab_param_commit_handler(state: &mut HelixState, data: &[u8]) -> LayerResult {
    legacy_cab_param_commit::handle_in_layer(state, data)
}

const ACTIVE_LAYERS: [(ActiveLayerId, ActiveHandler); 5] = [
    (ActiveLayerId::MatrixRoutingDd, matrix_routing_dd_handler),
    (ActiveLayerId::ScrollModelPull, scroll_model_pull_handler),
    (ActiveLayerId::LegacyCabParamCommit, legacy_cab_param_commit_handler),
    (ActiveLayerId::FirmwareScroll, firmware_scroll_ack::handle_in_layer),
    (
        ActiveLayerId::PresetDumpStream,
        preset_dump_stream_ack::handle_in_layer,
    ),
];

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

    fn fake_scroll(_: &mut HelixState, data: &[u8]) -> LayerResult {
        if data.first() == Some(&0xaa) {
            LayerResult::Consumed {
                effect: LayerEffect::ScrollLaneAndAck,
            }
        } else {
            LayerResult::Ignored
        }
    }

    fn fake_dump(_: &mut HelixState, _: &[u8]) -> LayerResult {
        LayerResult::Ignored
    }

    #[test]
    fn consumed_stops_following_active_layers() {
        let layers: [(ActiveLayerId, ActiveHandler); 2] = [
            (ActiveLayerId::FirmwareScroll, fake_scroll),
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
    fn layer_result_contract() {
        assert!(!LayerResult::Ignored.is_consumed());
        assert!(
            LayerResult::Consumed {
                effect: LayerEffect::ScrollLaneAndAck
            }
            .is_consumed()
        );
        let observed = LayerResult::Observed {
            effect: LayerEffect::None,
        };
        assert!(!observed.is_consumed());
    }
}
