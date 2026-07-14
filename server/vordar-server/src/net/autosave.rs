use engine_app::scheduler::System;
use engine_core::components::{Health, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use engine_net::ConnId;

use crate::db::CharacterRecord;
use super::{cooldown_remainders, NetServerState};

/// Autosave every Nth PostUpdate run (60 Hz → ~30 s).
const AUTOSAVE_TICKS: u64 = 1800;

fn autosave_due(conn: ConnId, tick: u64) -> bool {
    conn % AUTOSAVE_TICKS == tick % AUTOSAVE_TICKS
}

/// Periodic character persistence: over each AUTOSAVE_TICKS window (~30 s),
/// hand each connected player's position + health to the DB worker — one
/// save per connection per window, staggered by `autosave_due` so a crowd's
/// saves don't all land on the same tick. Fire-and-forget — disconnect-save
/// covers the gap on clean exits.
pub(super) struct AutosaveSystem {
    pub(super) ticks: u64,
}

impl System for AutosaveSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let tick = self.ticks;
        self.ticks += 1;
        let state = resources.get_mut::<NetServerState>().unwrap();
        for (&conn, pc) in &state.conns {
            if !autosave_due(conn, tick) {
                continue;
            }
            if let (Ok(tr), Ok(hp)) = (world.get::<&Transform>(pc.entity), world.get::<&Health>(pc.entity)) {
                let cooldowns = cooldown_remainders(&pc.cooldown_ready, state.server.now_micros());
                state.db.save(
                    pc.name.clone(),
                    CharacterRecord { zone: state.zone.name.clone(), pos: tr.position, health: hp.current, cooldowns },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn autosave_spreads_a_crowd_across_the_window_instead_of_bursting() {
        let conns: Vec<ConnId> = (1..=50).collect();
        let mut due_ticks: HashSet<u64> = HashSet::new();
        let mut due_count: HashMap<ConnId, u32> = HashMap::new();
        for tick in 0..AUTOSAVE_TICKS {
            for &conn in &conns {
                if autosave_due(conn, tick) {
                    due_ticks.insert(tick);
                    *due_count.entry(conn).or_insert(0) += 1;
                }
            }
        }
        // Every connection autosaves exactly once per window.
        for &conn in &conns {
            assert_eq!(due_count.get(&conn).copied().unwrap_or(0), 1, "conn {conn} did not save exactly once");
        }
        // The 50-strong crowd's saves land on more than one tick — not a
        // single-tick burst.
        assert!(due_ticks.len() > 1, "all autosaves landed on the same tick: {due_ticks:?}");
    }
}
