//! Cab dual — transport partagé (IR + legacy hybrid).
//!
//! - [`replace_fire`] : séquence `focus → ed:08 → bulk` sur lane `live_write` (agnostique tête bulk).
//! - [`ir`] : focus / live_write IR (WithPan, head `0x27`).
//! - [`legacy`] : wire builders legacy (sélecteurs 1 o, head `0x23` / `0x25` / `0x2d`).

pub mod cab2_replace;
pub mod ir;
pub mod legacy;
pub mod replace_fire;

pub use cab2_replace::execute_cab_dual_cab2_replace;
