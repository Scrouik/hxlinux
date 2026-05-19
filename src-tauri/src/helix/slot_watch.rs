// Surveillance du contenu d'un slot — empreinte IN (focus USB), sans `preset_data`.

use serde::Serialize;

use crate::helix::slot_focus_in::SlotFocusInCapsule;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlotWatchSnapshot {
    pub capsule_sig: String,
}

impl SlotWatchSnapshot {
    pub fn from_capsule(capsule: Option<&SlotFocusInCapsule>) -> Self {
        let capsule_sig = capsule
            .map(|c| format!("{}|{}", c.anchor12_hex, c.ed_suffix7_hex))
            .unwrap_or_default();
        Self { capsule_sig }
    }
}

/// Changement détecté sur le bus (détail modèle / vide / params : decode IN, phase ultérieure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotContentChangeKind {
    Content,
}

impl SlotContentChangeKind {
    pub fn as_str(self) -> &'static str {
        "content"
    }
}

/// Compare l'empreinte précédente à la nouvelle ; met à jour `prev` si la capsule a changé.
pub fn detect_slot_content_change(
    prev: &mut SlotWatchSnapshot,
    next: &SlotWatchSnapshot,
) -> Option<SlotContentChangeKind> {
    if prev.capsule_sig == next.capsule_sig {
        return None;
    }
    *prev = next.clone();
    Some(SlotContentChangeKind::Content)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotContentChangedPayload {
    pub sequence: u32,
    pub slot_index: u32,
    pub kind: String,
    /// Empreinte IN actuelle (debug / corrélation future decode).
    pub capsule_sig: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::slot_focus_in::SlotFocusInCapsule;

    fn sample_capsule(anchor: &str) -> SlotFocusInCapsule {
        let mut anchor12 = [0u8; 12];
        for (i, chunk) in anchor.as_bytes().chunks(2).enumerate() {
            if i < 12 && chunk.len() == 2 {
                anchor12[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
            }
        }
        SlotFocusInCapsule {
            slot_bus: 0x02,
            ed03_36_hex: None,
            f003_44_hex: None,
            anchor12_hex: anchor.to_string(),
            anchor12,
            ed_suffix7_hex: "670068c079136a".to_string(),
        }
    }

    #[test]
    fn detects_capsule_change() {
        let mut prev = SlotWatchSnapshot::from_capsule(Some(&sample_capsule("8269276a845201440379136a")));
        let next = SlotWatchSnapshot::from_capsule(Some(&sample_capsule("aaaaaaaaaaaaaaaaaaaaaaaa")));
        assert_eq!(
            detect_slot_content_change(&mut prev, &next),
            Some(SlotContentChangeKind::Content)
        );
    }

    #[test]
    fn no_change_when_capsule_identical() {
        let c = sample_capsule("8269276a845201440379136a");
        let mut prev = SlotWatchSnapshot::from_capsule(Some(&c));
        let next = SlotWatchSnapshot::from_capsule(Some(&c));
        assert_eq!(detect_slot_content_change(&mut prev, &next), None);
    }
}
