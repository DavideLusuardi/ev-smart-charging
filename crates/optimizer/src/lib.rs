//! Pure scheduling library: shares site grid capacity across EV charging sessions.
//!
//! Nothing here does IO or async work, so every strategy can be tested and benchmarked on
//! its own. Power is in W, energy in Wh, and time in fixed-length slots.

pub mod types;

pub use types::{
    EvseRef, Schedule, Scheduler, SessionId, SessionRequest, SiteInput, Slot, Violation,
};
