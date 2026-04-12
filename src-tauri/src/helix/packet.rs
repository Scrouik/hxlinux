// ===========================================================
// packet.rs
// Équivalent de OutPacket + my_byte_cmp dans kempline
// ===========================================================

/// Un paquet à envoyer vers le HX via endpoint 0x01
#[derive(Debug, Clone)]
pub struct OutPacket {
    pub data: Vec<u8>,
    pub delay_ms: u64,  // délai avant envoi (kempline utilise delay=0.140 etc.)
}

impl OutPacket {
    /// Paquet sans délai
    pub fn new(data: Vec<u8>) -> Self {
        Self { data, delay_ms: 0 }
    }

    /// Paquet avec délai en millisecondes
    pub fn with_delay(data: Vec<u8>, delay_ms: u64) -> Self {
        Self { data, delay_ms }
    }
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