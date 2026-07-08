// Vordar — networked client. The server is authoritative: this process runs
// only rendering, input→intent sending, snapshot replication, and prediction
// of its own player via the shared movement systems (Phase 2).
//
//   cargo run -p vordar-client --bin vordar [server_addr]   (default 127.0.0.1:5151)
//
// Env knobs: VORDAR_USER=name picks the character to play (default "player");
// VORDAR_LATENCY_MS=150 adds artificial round-trip latency; VORDAR_PREDICT=0
// disables prediction (Phase-1 server-driven feel).
//
// Run from the workspace root: prefabs load from content/ relative to cwd.

use engine_app::app::App;
use engine_app::prefab_plugin::PrefabPlugin;
use engine_renderer::RenderPlugin;
use std::net::SocketAddr;
use std::time::Duration;
use vordar_client::net::NetClientPlugin;
use vordar_game::chapter::ChapterRegistry;
use vordar_game::GameComponentsPlugin;

fn main() {
    let server_addr: SocketAddr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:5151".into())
        .parse()
        .expect("usage: vordar [server_ip:port]");
    let simulated_rtt = std::env::var("VORDAR_LATENCY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::ZERO);
    let predict = std::env::var("VORDAR_PREDICT").map_or(true, |v| v != "0");
    let user = std::env::var("VORDAR_USER").unwrap_or_else(|_| "player".into());

    let mut app = App::new();
    app.configure("content/config/engine.ron")
        .add_plugin(RenderPlugin)
        .add_plugin(PrefabPlugin)
        .add_plugin(GameComponentsPlugin)
        // Same defs the server spawns from — tint/active-state agreement is
        // pure clock math (DESIGN.md §4).
        .insert_resource(vordar_game::world::load_world_events("content/world/events.ron"));
    // Replicated NPCs spawn from chapter prefabs — content registrations of
    // EVERY linked chapter (a Redirect can land us in any zone), no chapter
    // systems (the server is authoritative).
    ChapterRegistry::new(vec![chapter_01::module(), chapter_02::module()]).install_all_content(&mut app);
    app.add_plugin(NetClientPlugin { server_addr, predict, simulated_rtt, user })
        .run()
}
