//! Shared e2e test harness: headless bot clients speaking the real protocol
//! (engine-net directly, no renderer), plus small server-side test systems.

mod bot;
mod server;
mod util;

pub use bot::*;
pub use server::*;
pub use util::*;
