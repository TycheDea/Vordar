// Trial (T10): egui-kittest wgpu snapshot testing on the two game-side HUD
// draw callbacks (minimap + action bar). Exactly two snapshots, of otherwise
// static screens (no animation, no async image loads) — they should never
// spuriously regenerate. Regenerate via
// `UPDATE_SNAPSHOTS=true cargo test -p vordar-client --test ui_snapshots`.
// If either flakes across more than 2 runs, the trial is failed: delete this
// file, `tests/snapshots/`, and the egui_kittest dev-dependency.

use egui_kittest::Harness;
use engine_core::traits::Resources;
use engine_renderer::offscreen::HeadlessGpu;
use glam::Vec2;
use vordar_client::ui::action_bar::{self, ActionBarState, SkillSlot};
use vordar_client::ui::minimap::{self, HudDot, HudState};

fn sample_hud() -> HudState {
    HudState {
        open: true,
        center: Some(Vec2::new(12.0, -4.0)),
        heading: 0.7,
        dots: vec![
            HudDot { pos: Vec2::new(20.0, 2.0), color: [0.9, 0.2, 0.2] },
            HudDot { pos: Vec2::new(-8.0, -20.0), color: [0.2, 0.8, 0.3] },
            HudDot { pos: Vec2::new(40.0, 30.0), color: [0.3, 0.5, 0.9] },
        ],
        markers: vec![Vec2::new(60.0, -60.0)],
        range: 45.0,
        label: "castilian_dusk".into(),
        reconnecting: None,
    }
}

fn sample_action_bar() -> ActionBarState {
    ActionBarState {
        open: true,
        slots: vec![
            SkillSlot { label: "Bolt".into(), keybind: "LMB", cooldown_frac: None, enabled: true },
            SkillSlot { label: "Guard".into(), keybind: "Q", cooldown_frac: Some(0.4), enabled: true },
            SkillSlot { label: "Rush".into(), keybind: "E", cooldown_frac: None, enabled: true },
            SkillSlot { label: String::new(), keybind: "", cooldown_frac: None, enabled: false },
            SkillSlot { label: String::new(), keybind: "", cooldown_frac: None, enabled: false },
        ],
    }
}

#[test]
fn minimap_snapshot() {
    // kittest's wgpu renderer panics (no adapter) instead of returning an
    // error, so probe first — same skip as the offscreen render tests.
    if HeadlessGpu::new().is_none() {
        eprintln!("SKIP: no GPU adapter");
        return;
    }

    let mut resources = Resources::new();
    resources.insert(sample_hud());

    let mut harness = Harness::builder().wgpu().build_ui(|ui| minimap::draw(ui, &resources));
    harness.run();
    harness.snapshot("minimap");
}

#[test]
fn action_bar_snapshot() {
    if HeadlessGpu::new().is_none() {
        eprintln!("SKIP: no GPU adapter");
        return;
    }

    let mut resources = Resources::new();
    resources.insert(sample_action_bar());

    let mut harness = Harness::builder().wgpu().build_ui(|ui| action_bar::draw(ui, &resources));
    harness.run();
    harness.snapshot("action_bar");
}
