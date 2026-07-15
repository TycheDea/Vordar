// Finding: audit-game-architecture 2026-07-15 #11 — ClientPlugin (offline)
// and NetClientPlugin (online) each registered the same ~12 shared
// presentation systems (zone dressing, corpses/hit-react, pose/facing/
// locomotion, VFX, weapons, UI) verbatim, so every new presentation system
// had to be added twice or one mode silently lacked it. PresentationPlugin
// now owns that list once; this asserts the two callers' registered systems
// still diverge by nothing but the genuine differences: input, cast,
// camera-follow, and (online) net systems.

use engine_app::app::App;
use engine_app::scheduler::Phase;
use std::collections::BTreeSet;
use std::time::Duration;
use vordar_client::net::NetClientPlugin;
use vordar_client::ClientPlugin;

const PHASES: [Phase; 4] = [Phase::Input, Phase::Update, Phase::DespawnFlush, Phase::RenderSync];

/// Short (last path segment) type names of every system registered on `app`
/// across the phases these two plugins touch.
fn registered_names(app: &App) -> BTreeSet<String> {
    PHASES
        .iter()
        .flat_map(|&phase| app.pending_system_names(phase))
        .map(|full| full.rsplit("::").next().unwrap_or(full).to_string())
        .collect()
}

#[test]
fn offline_and_online_presentation_lists_diverge_only_by_input_cast_camera_net() {
    test_support::workspace_root();

    let mut offline = App::new();
    offline.add_plugin(ClientPlugin);

    let mut online = App::new();
    online.add_plugin(NetClientPlugin {
        // Discard-port convention this crate's other net-adjacent tests use
        // for an address nothing ever actually dials (see net/bench.rs,
        // net/apply.rs, net/interpolate.rs) — the background connect thread
        // fails asynchronously, which doesn't affect plugin registration.
        server_addr: "127.0.0.1:9".parse().unwrap(),
        predict: false,
        simulated_rtt: Duration::ZERO,
        user: "presentation-plugin-test".into(),
        token: test_support::name_token("presentation-plugin-test"),
    });

    let offline_names = registered_names(&offline);
    let online_names = registered_names(&online);

    let offline_only: BTreeSet<String> = offline_names.difference(&online_names).cloned().collect();
    let online_only: BTreeSet<String> = online_names.difference(&offline_names).cloned().collect();

    let expected_offline_only: BTreeSet<String> =
        ["PlayerInputSystem", "SandboxCastSystem", "CameraFollowSystem"]
            .map(String::from)
            .into_iter()
            .collect();
    let expected_online_only: BTreeSet<String> = [
        "NetReceiveSystem",
        "NetSendInputSystem",
        "AbilityCastSystem",
        "NetInterpolateSystem",
        "NetCameraFollowSystem",
        "TelegraphFillSystem",
        "DayNightSystem",
    ]
    .map(String::from)
    .into_iter()
    .collect();

    assert_eq!(offline_only, expected_offline_only, "offline-only systems");
    assert_eq!(online_only, expected_online_only, "online-only systems");

    // The shared presentation list must actually be registered on both
    // sides — proof PresentationPlugin ran, not just absence from the diff.
    for shared in ["ZoneDressingSystem", "BodyComposeSystem", "WeaponAttachSystem"] {
        assert!(offline_names.contains(shared), "offline missing shared system {shared}");
        assert!(online_names.contains(shared), "online missing shared system {shared}");
    }
}
