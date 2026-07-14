use engine_app::app::AppExit;
use engine_app::scheduler::System;
use engine_core::components::{Health, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::db::CharacterRecord;
use super::{cooldown_remainders, NetServerState};

/// Process-wide shutdown signal (networking rework 8, finding 3): `main`
/// shares one `Arc<AtomicBool>` with its OS signal handler and inserts a
/// clone into every zone App. Absent from every existing test/bench, which is
/// exactly how `ShutdownSystem` tells "no shutdown wired" apart from "not
/// shutting down yet".
pub struct ShutdownFlag(pub Arc<AtomicBool>);

/// On the shared flag: save every connected player's live state — the same
/// save the disconnect path performs (`ServerEvent::Disconnected` above), just
/// for everyone at once — and request the App's exit. Registered
/// unconditionally by `install()`; a no-op wherever `ShutdownFlag` is absent
/// or still false. No client notification here: `NetServer`'s Drop (finding
/// 1) closes every connection with a reason when the App drops moments later.
pub(super) struct ShutdownSystem;

impl System for ShutdownSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let flagged = resources.get::<ShutdownFlag>().is_some_and(|f| f.0.load(Ordering::Relaxed));
        if !flagged {
            return;
        }
        let state = resources.get_mut::<NetServerState>().unwrap();
        let saved = state.conns.len();
        for pc in state.conns.values() {
            // Players still in `state.loading` have no entity yet — nothing
            // to save.
            if let (Ok(tr), Ok(hp)) = (world.get::<&Transform>(pc.entity), world.get::<&Health>(pc.entity)) {
                let cooldowns = cooldown_remainders(&pc.cooldown_ready, state.server.now_micros());
                state.db.save(
                    pc.name.clone(),
                    CharacterRecord { zone: state.zone.name.clone(), pos: tr.position, health: hp.current, cooldowns },
                );
            }
        }
        log::info!("zone '{}': shutdown flag set, saved {saved} connected player(s), requesting app exit", state.zone.name);
        resources.get_mut::<AppExit>().unwrap().0 = true;
    }
}
