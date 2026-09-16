//! Pure scheduling library: shares site grid capacity across EV charging sessions.
//!
//! Nothing here does IO or async work, so every strategy can be tested and benchmarked on
//! its own. Power is in W, energy in Wh, and time in fixed-length slots.

pub mod control;
pub mod edf;
pub mod equal_share;
#[cfg(feature = "lp")]
pub mod lp;
pub mod needs;
pub mod planner;
pub mod profile;
pub mod types;

pub use control::{ControlConfig, Dispatch, ProfileUpdate, dispatch};
pub use edf::Edf;
pub use equal_share::EqualShare;
#[cfg(feature = "lp")]
pub use lp::Lp;
pub use needs::{KnownNeeds, NeedsDefaults, SessionState, observe_draw};
pub use planner::{EvseLimit, Plan, PlanRequest, Planner};
pub use profile::{Period, StepProfile};
pub use types::{
    EvseRef, Schedule, Scheduler, SessionId, SessionRequest, SiteInput, Slot, Violation,
};
