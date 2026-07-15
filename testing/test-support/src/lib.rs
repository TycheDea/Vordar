//! Shared e2e test harness: headless bot clients speaking the real protocol
//! (engine-net directly, no renderer), plus small server-side test systems.
//!
//! This crate uses flat re-exports to provide a convenient single namespace for callers.
//! Symbols are defined in purpose-named modules (bot, server, fs, stats, threads, rng)
//! but re-exported at the crate root for ergonomics. This is the documented convention
//! for the harness: import from `test_support::symbol` directly, not `test_support::module::symbol`.

mod bot;
mod fs;
mod rng;
mod server;
mod stats;
mod threads;

pub use bot::*;
pub use fs::*;
pub use rng::*;
pub use server::*;
pub use stats::*;
pub use threads::*;
