use engine_app::scheduler::System;
use engine_core::prefab::queue_prefab_spawn;
use engine_core::traits::Resources;
use engine_core::World;
use engine_net::NetMetrics;
use glam::Vec3;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vordar_game::zones::{validate_zones, PortalDef, ZoneDef, ZonesDef};
use vordar_server::db::{DbHandle, DbWorker};
use vordar_server::net::NetServerState;

/// Fresh SQLite path in the temp dir for persistence tests.
pub fn temp_db(tag: &str) -> String {
    let path = std::env::temp_dir().join(format!("vordar-e2e-{tag}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path.to_str().unwrap().to_owned()
}

/// Compact two-zone topology so walks stay short: start's portal at x=10
/// drops you at x=-6 in east; east's portal at x=-10 sends you back to x=6.
/// Shared by `zones.rs` (multi-zone e2e) and `shutdown.rs` (networking
/// rework plan 2026-07-12, finding 4: the shutdown wiring test mirrors this
/// exact topology).
pub fn test_zones() -> Vec<ZoneDef> {
    let zones = vec![
        ZoneDef {
            name: "start".into(),
            chapter: None,
            portals: vec![PortalDef {
                pos: Vec3::new(10.0, 0.0, 0.0),
                radius: 2.0,
                target_zone: "east".into(),
                target_pos: Vec3::new(-6.0, 0.0, 0.0),
            }],
            visuals: Default::default(),
        },
        ZoneDef {
            name: "east".into(),
            chapter: None,
            portals: vec![PortalDef {
                pos: Vec3::new(-10.0, 0.0, 0.0),
                radius: 2.0,
                target_zone: "start".into(),
                target_pos: Vec3::new(6.0, 0.0, 0.0),
            }],
            visuals: Default::default(),
        },
    ];
    validate_zones(&ZonesDef { zones: zones.clone() }).unwrap();
    zones
}

/// One-shot world population, registered on the server App.
pub struct PopulateSystem {
    pub done: bool,
    pub positions: Vec<glam::Vec3>,
    /// Prefab to spawn at each position — "player" as a stationary, harmless
    /// NPC stand-in (Transform/Hitbox so it's in the SpatialGrid, no AI) is
    /// the common case; other replicated prefabs (e.g. "bolt", Health-less)
    /// are spawned the same way for tests that need a specific shape.
    pub prefab: String,
}

impl System for PopulateSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        if self.done {
            return;
        }
        self.done = true;
        for &pos in &self.positions {
            queue_prefab_spawn(resources, self.prefab.clone(), pos);
        }
    }
}

/// Mirrors one `NetMetrics` atomic counter into a plain atomic every tick —
/// systems can't return values, so this smuggles a live count out to the
/// harness, which samples it directly around a measurement window. `select`
/// picks the field (e.g. `|m| &m.rejects`, `|m| &m.busy_micros`).
pub struct MetricMirror {
    pub dest: Arc<AtomicU64>,
    pub select: fn(&NetMetrics) -> &AtomicU64,
}

impl System for MetricMirror {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let state = resources.get::<NetServerState>().expect("NetServerState not installed");
        let metrics = state.metrics();
        self.dest.store((self.select)(&metrics).load(Ordering::Relaxed), Ordering::Relaxed);
    }
}

/// Shared multi-zone bring-up: builds a two-zone directory (`start`/`east`,
/// matching `test_zones()`), opens one `DbWorker`, mints one world-time
/// origin, and spawns one thread per zone via `per_zone` — the injection
/// point for whatever a specific test varies (a `ShutdownFlag`, a
/// `supervise_zone` wrapper, a chapter install), since the setup above never
/// does. Returns the zone `JoinHandle`s (in `zones`' order) and the
/// `DbWorker`, which the caller must keep alive for as long as any zone
/// thread runs — `mem::forget` it for a fire-and-forget bring-up, or hold it
/// to `drop` after joining every handle.
pub fn spawn_zones(
    zones: Vec<ZoneDef>,
    start_addr: SocketAddr,
    east_addr: SocketAddr,
    db_path: &str,
    mut per_zone: impl FnMut(SocketAddr, DbHandle, ZoneDef, HashMap<String, SocketAddr>, Instant) -> std::thread::JoinHandle<()>,
) -> (Vec<std::thread::JoinHandle<()>>, DbWorker) {
    let directory: HashMap<String, SocketAddr> =
        HashMap::from([("start".to_owned(), start_addr), ("east".to_owned(), east_addr)]);
    let worker = DbWorker::spawn(db_path).expect("db open");
    let world_origin = Instant::now();
    let handles = zones
        .into_iter()
        .map(|zone| {
            let addr = directory[&zone.name];
            per_zone(addr, worker.handle(), zone, directory.clone(), world_origin)
        })
        .collect();
    std::thread::sleep(Duration::from_millis(300));
    (handles, worker)
}
