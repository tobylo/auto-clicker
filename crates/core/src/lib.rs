//! Core logic of auto-clicker: the click engine, the recording format and the
//! config types. Nothing here touches an input device or D-Bus, so all of it
//! compiles and tests without a session.

pub fn   double( x:u8 )->u8{ x.saturating_mul(2) }
