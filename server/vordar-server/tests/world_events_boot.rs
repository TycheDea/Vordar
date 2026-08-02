// An events def naming a prefab the zone's library doesn't carry is an
// authoring bug: `check_world_events` — the boot gate main.rs runs right
// after loading `events.ron` — must refuse to bring the zone up rather than
// let the dangling name fail every spawn attempt at runtime.

use engine_app::app::App;
use engine_core::prefab::PrefabLibrary;
use glam::Vec3;
use vordar_game::world::{EventWaveDef, WorldEventDef, WorldEventsDef, WorldSpawn};
use vordar_server::check_world_events;

fn def_with_unknown_one_shot() -> WorldEventsDef {
    WorldEventsDef {
        day_seconds: 120.0,
        events: vec![WorldEventDef {
            name: "e".into(),
            start_seconds_of_day: 0.0,
            duration_seconds: 10.0,
            ambient: Vec3::ONE,
            spawns: vec![WorldSpawn { prefab: "ghost_prefab".into(), positions: vec![Vec3::ZERO] }],
            waves: vec![],
        }],
    }
}

fn def_with_unknown_wave() -> WorldEventsDef {
    WorldEventsDef {
        day_seconds: 120.0,
        events: vec![WorldEventDef {
            name: "e".into(),
            start_seconds_of_day: 0.0,
            duration_seconds: 10.0,
            ambient: Vec3::ONE,
            spawns: vec![],
            waves: vec![EventWaveDef {
                prefab: "ghost_prefab".into(),
                positions: vec![Vec3::ZERO],
                interval_seconds: 5.0,
                max_alive: 3,
            }],
        }],
    }
}

#[test]
fn unknown_one_shot_prefab_fails_boot() {
    let mut app = App::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_world_events(&mut app, "test-zone", &def_with_unknown_one_shot());
    }));
    assert!(result.is_err(), "check_world_events must panic on an unresolved one-shot prefab");
}

#[test]
fn unknown_wave_prefab_fails_boot() {
    let mut app = App::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_world_events(&mut app, "test-zone", &def_with_unknown_wave());
    }));
    assert!(result.is_err(), "check_world_events must panic on an unresolved wave prefab");
}

#[test]
fn known_prefab_passes_boot_check() {
    let mut app = App::new();
    let lib = app.resource_or_default::<PrefabLibrary>();
    lib.insert("grunt", engine_core::prefab::PrefabDef { components: Default::default() });

    let def = def_with_unknown_one_shot();
    let def = WorldEventsDef {
        day_seconds: def.day_seconds,
        events: vec![WorldEventDef {
            spawns: vec![WorldSpawn { prefab: "grunt".into(), positions: vec![Vec3::ZERO] }],
            ..def.events.into_iter().next().unwrap()
        }],
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_world_events(&mut app, "test-zone", &def);
    }));
    assert!(result.is_ok(), "a fully-resolved events def must pass the boot check");
}
