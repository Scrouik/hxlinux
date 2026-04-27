use crate::helix::HelixState;

pub struct LiveWriteFrames {
    pub model_block_kind: &'static str,
    pub pp: u8,
    pub pp_source: &'static str,
    pub param_selector: u8,
    pub param_selector_source: &'static str,
    pub pre_packet_x80: Vec<u8>,
    pub pre_packet_x2: Vec<u8>,
    /// Pré-trame ED03 observée en capture : octet 11 = `0x08`.
    pub pre_packet_x80_sel: Vec<u8>,
    /// Première jambe HX Edit : octet 11 = `0x04`.
    pub packet_27: Vec<u8>,
    /// Deuxième jambe : octet 11 = `0x0c`, `SEQ` suivant, `CTR` += `0x1F`, `YY` +1.
    pub packet_27_b: Vec<u8>,
    /// Trame ED03 de clôture observée après la paire `27`.
    pub post_packet_x80_sel: Vec<u8>,
    pub frame27_diff_vs_static: String,
}

fn pp_from_symbolic_and_index(symbolic_id: &str, param_index: u32) -> (u8, &'static str) {
    // Mapping observé dans les captures HX Edit fournies:
    // - La plupart des writes utilisent PP=0x04.
    // - Minotaur Tone montre PP=0x03 sur une partie des écritures.
    // Quand un PP "inventé" est envoyé (05/06), le firmware semble retomber sur le 1er paramètre.
    if symbolic_id.eq_ignore_ascii_case("Tone") {
        return (0x03, "capture_map:tone");
    }
    match param_index {
        0 | 2 => (0x04, "capture_map:param0_or_2"),
        _ => (0x04, "capture_map:default_0x04"),
    }
}

fn param_selector_byte_from_index(param_index: u32) -> (u8, &'static str) {
    // Observé sur captures Minotaur:
    // - Tone  (param 1) -> offset40 = 0x01
    // - Level (param 2) -> offset40 = 0x02
    // Le premier paramètre reste à 0x00.
    ((param_index.min(0xff)) as u8, "index_to_offset40")
}

/// Assemble une trame write `27` opcode `80:10:ed:03` (48 octets).
fn assemble_27_write(
    seq: u8,
    byte11: u8,
    ctr: u16,
    yy: u8,
    pp: u8,
    param_selector: u8,
    float_be: [u8; 4],
) -> Vec<u8> {
    vec![
        0x27, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, byte11,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x00,
        0x01, 0x00, 0x06, 0x00, 0x17, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, 0x1e, 0x65,
        0x85, 0x62, 0x01, 0x1d, 0xc3, 0x1a, 0x00, 0x1c,
        param_selector, 0x77, 0xca, float_be[0], float_be[1], float_be[2], float_be[3], 0x00,
    ]
}

fn apply_echo_model_block(
    packet: &mut [u8],
    state: &mut HelixState,
    last_echo: [u8; 16],
) {
    let mut model_block = last_echo;
    let next_seq = match state.ed03_live_write_seq_sent {
        Some(prev) => prev.wrapping_add(1),
        None => model_block[4].wrapping_add(1),
    };
    model_block[4] = next_seq;
    state.ed03_live_write_seq_sent = Some(next_seq);
    packet[24..40].copy_from_slice(&model_block);
}

/// Construit les trames de write live à partir de la session Helix courante.
///
/// HX Edit envoie typiquement **deux** trames `27` par pas : octet 11 = `0x04` puis `0x0c`,
/// avec `SEQ` qui suit le flux keep-alive, `CTR` qui avance de `+0x1F` à chaque trame,
/// et `YY` qui s'incrémente (captures `src/Paquets Json/`).
pub fn build_live_write_frames_from_state(
    state: &mut HelixState,
    raw_value: f32,
    param_index: u32,
    symbolic_id: &str,
) -> LiveWriteFrames {
    let float_be = raw_value.to_bits().to_be_bytes();
    let pre_cnt_x80 = state.next_x80_cnt();
    let pre_cnt_x2 = state.next_x2_cnt();
    let pre_session = state.session_no;
    let pre_double = state.preset_data_packet_double();

    let pre_packet_x80 = vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, pre_cnt_x80, 0x00, 0x10, pre_session, pre_double[0], pre_double[1], 0x00,
    ];
    let pre_packet_x2 = vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03,
        0x00, pre_cnt_x2, 0x00, 0x10, 0x09, 0x10, 0x00, 0x00,
    ];

    let (pp, pp_source) = pp_from_symbolic_and_index(symbolic_id, param_index);
    let (param_selector, param_selector_source) = param_selector_byte_from_index(param_index);

    // Trame "sélection/contexte" observée avant les writes 27.
    let seq_sel = state.next_x80_cnt();
    let ctr_a = state.live_write_ctr;
    let pre_packet_x80_sel = vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq_sel, 0x00, 0x08, (ctr_a & 0xff) as u8, ((ctr_a >> 8) & 0xff) as u8, 0x00, 0x00,
    ];

    let seq_a = state.next_x80_cnt();
    let yy_a = state.live_write_yy;

    let mut packet_a = assemble_27_write(seq_a, 0x04, ctr_a, yy_a, pp, param_selector, float_be);
    let (model_block_kind, _) = if let Some(last_echo) = state.last_ed03_echo_model {
        apply_echo_model_block(&mut packet_a, state, last_echo);
        ("in_echo_strict", ())
    } else {
        ("replay_static", ())
    };
    packet_a[43..47].copy_from_slice(&float_be);

    // Compteur transaction : +0x1F entre chaque trame (ordre attendu par le firmware).
    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    let seq_b = state.next_x80_cnt();
    let ctr_b = state.live_write_ctr;
    let yy_b = state.live_write_yy;

    let mut packet_b = assemble_27_write(seq_b, 0x0c, ctr_b, yy_b, pp, param_selector, float_be);
    if let Some(last_echo) = state.last_ed03_echo_model {
        apply_echo_model_block(&mut packet_b, state, last_echo);
    }
    packet_b[43..47].copy_from_slice(&float_be);

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    // En captures HX Edit, une trame `08 ... 00 08` revient souvent juste après la jambe b.
    let seq_post_sel = state.next_x80_cnt();
    let ctr_post = state.live_write_ctr;
    let post_packet_x80_sel = vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq_post_sel, 0x00, 0x08, (ctr_post & 0xff) as u8, ((ctr_post >> 8) & 0xff) as u8, 0x00, 0x00,
    ];

    let static_ref = assemble_27_write(0x8f, 0x04, 0x6cbd, 0x17, pp, param_selector, float_be);
    let frame27_diff_vs_static = diff_packet_hex(&static_ref, &packet_a);

    LiveWriteFrames {
        model_block_kind,
        pp,
        pp_source,
        param_selector,
        param_selector_source,
        pre_packet_x80,
        pre_packet_x2,
        pre_packet_x80_sel,
        packet_27: packet_a,
        packet_27_b: packet_b,
        post_packet_x80_sel,
        frame27_diff_vs_static,
    }
}

fn diff_packet_hex(reference: &[u8], actual: &[u8]) -> String {
    let max = reference.len().min(actual.len());
    let mut diffs = Vec::new();
    for i in 0..max {
        if reference[i] != actual[i] {
            diffs.push(format!("{i}:{:02x}->{:02x}", reference[i], actual[i]));
        }
    }
    if reference.len() != actual.len() {
        diffs.push(format!("len:{}->{}", reference.len(), actual.len()));
    }
    if diffs.is_empty() {
        return "none".to_string();
    }
    const MAX_ITEMS: usize = 12;
    if diffs.len() > MAX_ITEMS {
        let remaining = diffs.len() - MAX_ITEMS;
        diffs.truncate(MAX_ITEMS);
        diffs.push(format!("+{} more", remaining));
    }
    diffs.join(",")
}
