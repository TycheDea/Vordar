use engine_app::scheduler::System;
use engine_core::components::{Health, Transform};
use engine_core::traits::{DespawnQueue, Resources};
use engine_core::World;
use engine_net::ConnId;
use vordar_game::zones::portal_hit;
use vordar_protocol::{encode, ServerMsg};

use crate::db::CharacterRecord;
use super::{cooldown_remainders, NetServerState, STAGGER};

/// Portal handoff: persist → despawn → redirect. The character is saved into
/// the TARGET zone at the portal's arrival point, the body leaves this zone,
/// and the client is told where to log in next. The CLIENT closes the
/// connection — kicking here could outrace the Redirect frame. The eventual
/// Disconnected finds no PlayerConn, so no stale save can clobber the
/// transfer save.
pub(super) struct ZoneTransferSystem {
    ticks: u64,
}

impl ZoneTransferSystem {
    pub(super) fn new() -> Self {
        Self { ticks: 0 }
    }
}

impl System for ZoneTransferSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // PostUpdate runs at POST_HZ; transfers keep their 10 Hz cadence.
        let due_now = self.ticks.is_multiple_of(STAGGER);
        self.ticks += 1;
        if !due_now {
            return;
        }
        let transfers: Vec<ConnId> = {
            let state = resources.expect::<NetServerState>();
            if state.zone.portals.is_empty() {
                return;
            }
            state.conns.iter()
                .filter(|(_, pc)| {
                    world.get::<&Transform>(pc.entity)
                        .is_ok_and(|tr| portal_hit(&state.zone.portals, tr.position).is_some())
                })
                .map(|(&conn, _)| conn)
                .collect()
        };

        for conn in transfers {
            let state = resources.expect_mut::<NetServerState>();
            let Some(pc) = state.conns.get(&conn) else { continue };
            let Ok(pos) = world.get::<&Transform>(pc.entity).map(|tr| tr.position) else { continue };
            let portal = portal_hit(&state.zone.portals, pos).unwrap().clone();
            let Some(&addr) = state.directory.get(&portal.target_zone) else {
                // Content validation makes this unreachable; never strand the
                // player in a half-transferred state over a config bug.
                log::error!("portal targets unlisted zone '{}' — ignoring", portal.target_zone);
                continue;
            };
            let pc = state.conns.remove(&conn).unwrap();
            let health = world.get::<&Health>(pc.entity).map(|hp| hp.current).unwrap_or(100);
            // Save FIRST: the FIFO db queue puts this ahead of the relogin
            // load the redirected client is about to trigger in the target.
            let cooldowns = cooldown_remainders(&pc.cooldown_ready, state.server.now_micros());
            let xp = world.get::<&vordar_game::progression::Xp>(pc.entity).map(|x| x.0).unwrap_or(pc.carried_xp);
            state.db.save(
                pc.name.clone(),
                CharacterRecord { zone: portal.target_zone.clone(), pos: portal.target_pos, health, cooldowns, xp },
            );
            state.server.send(conn, encode(&ServerMsg::Redirect { zone: portal.target_zone.clone(), addr }));
            resources.expect_mut::<DespawnQueue>().push(pc.entity, None);
            log::info!("conn {conn}: '{}' transfers to zone '{}' via portal", pc.name, portal.target_zone);
        }
    }
}
