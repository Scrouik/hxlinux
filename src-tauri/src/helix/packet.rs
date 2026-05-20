// ===========================================================
// packet.rs
// Équivalent de OutPacket + my_byte_cmp dans kempline
// ===========================================================

/// Un paquet à envoyer vers le HX via endpoint 0x01
#[derive(Debug, Clone)]
pub struct OutPacket {
    pub data: Vec<u8>,
    pub delay_ms: u64,  // délai avant envoi (kempline utilise delay=0.140 etc.)
    /// Envoyés sur le fil **à la suite** de `data`, sans autre paquet de la file entre les deux
    /// (ex. rafale HX Edit `1b` + `f0` ~16 µs).
    pub tail_burst: Vec<Vec<u8>>,
}

impl OutPacket {
    /// Paquet sans délai
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            delay_ms: 0,
            tail_burst: Vec::new(),
        }
    }

    /// Paquet avec délai en millisecondes
    pub fn with_delay(data: Vec<u8>, delay_ms: u64) -> Self {
        Self {
            data,
            delay_ms,
            tail_burst: Vec::new(),
        }
    }

    /// `head` puis chaque élément de `tail` en rafale USB (une seule entrée de file).
    pub fn with_tail_burst(head: Vec<u8>, tail: Vec<Vec<u8>>) -> Self {
        Self {
            data: head,
            delay_ms: 0,
            tail_burst: tail,
        }
    }
}

pub fn packet_counter(data: &[u8]) -> Option<u8> {
    if data.len() >= 10 && data.starts_with(&[0x08, 0x00, 0x00, 0x18]) {
        return Some(data[9]);
    }
    if data.len() >= 10 && (data[0] == 0x1d || data[0] == 0x19 || data[0] == 0x21 || data[0] == 0x27) {
        return Some(data[9]);
    }
    None
}

pub fn classify_out_packet(data: &[u8]) -> &'static str {
    if data.len() >= 8 {
        let op = &data[4..8];
        if op == [0x01, 0x10, 0xef, 0x03] {
            return "out_keepalive_x1";
        }
        if op == [0x02, 0x10, 0xf0, 0x03] {
            return "out_keepalive_x2";
        }
        if op == [0x80, 0x10, 0xed, 0x03] {
            if data.windows(4).any(|w| w == [0x82, 0x62, 0x00, 0x1a]) {
                return "out_slot_switch";
            }
            if data.len() >= 48 && data[0] == 0x27 {
                return "out_live_write_27";
            }
            return "out_keepalive_x80_or_ed03";
        }
        if op == [0xf0, 0x03, 0x02, 0x10] {
            return "out_preset_or_routing";
        }
    }
    "out_other"
}

pub fn classify_in_packet(data: &[u8]) -> &'static str {
    if data.len() >= 8 {
        let op = &data[4..8];
        if op == [0xef, 0x03, 0x01, 0x10] {
            return "in_ack_x1_or_slot";
        }
        if op == [0xed, 0x03, 0x80, 0x10] {
            return "in_ack_x80_or_slot";
        }
        if op == [0xf0, 0x03, 0x02, 0x10] {
            return "in_x2_stream";
        }
    }
    "in_other"
}

/// Équivalent de my_byte_cmp dans kempline.
///
/// Compare les `length` premiers bytes de `data` avec `pattern`.
/// Dans kempline, "XX" est un wildcard (on accepte n'importe quelle valeur).
/// Ici on utilise Option<u8> : None = wildcard, Some(v) = valeur exacte.
///
/// Exemple kempline :
///   my_byte_cmp(data, [0x8, 0x0, "XX", 0x18], length=4)
/// Équivalent ici :
///   byte_cmp(&data, &[Some(0x8), Some(0x0), None, Some(0x18)], 4)
pub fn byte_cmp(data: &[u8], pattern: &[Option<u8>], length: usize) -> bool {
    if data.len() < length || pattern.len() < length {
        return false;
    }
    for i in 0..length {
        if let Some(expected) = pattern[i] {
            if data[i] != expected {
                return false;
            }
        }
        // None = wildcard, on ne vérifie pas ce byte
    }
    true
}

/// Macro pour écrire les patterns lisiblement, comme dans kempline.
///
/// Exemple :
///   pattern![0x8, 0x0, XX, 0x18]
///   => vec![Some(0x8), Some(0x0), None, Some(0x18)]
#[macro_export]
macro_rules! pattern {
    (@item XX)      => { None };
    (@item $x:expr) => { Some($x) };
    ($($item:tt),* $(,)?) => {
        vec![$( $crate::pattern!(@item $item) ),*]
    };
}