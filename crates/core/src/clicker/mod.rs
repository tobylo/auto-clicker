//! The auto clicker: settings and the engine that runs them.
//!
//! The engine is one async function, [`run`], which the controller starts as
//! a task. It reaches devices only through [`Input`](crate::input::Input),
//! and ends at once on the stop or shutdown signal it is given.

mod engine;
mod settings;
mod timing;

pub use engine::{Ended, RunEnd, Signals, run};
pub use settings::{
    ClickerSettings, INTERVAL_FLOOR, INTERVAL_MAX, Interval, JITTER_MAX_PERCENT, Jitter, Mode,
    Position, Repeat,
};

#[cfg(test)]
mod tests;
