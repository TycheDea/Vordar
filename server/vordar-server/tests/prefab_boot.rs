// A corrupt prefab file must not sneak a zone up "healthy" with holes in its
// library: `PrefabLibrary::load_dir` stays fail-soft (skip and log), but
// `check_prefab_library` — the boot gate `build_zone_app`/main.rs run after
// every prefab dir has loaded — must refuse to pass a library that recorded
// any load error.

use engine_app::app::App;
use engine_core::prefab::PrefabLibrary;
use vordar_server::check_prefab_library;

#[test]
fn corrupt_prefab_file_fails_boot_but_not_load_dir() {
    let dir = std::env::temp_dir().join("vordar-prefab-boot-test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("good.ron"), "(components: {})").unwrap();
    std::fs::write(dir.join("broken.ron"), "not valid ron (((").unwrap();
    let dir_str = dir.to_str().unwrap();

    let mut app = App::new();
    // load_dir itself must not panic: the good prefab still loads, the bad
    // one is logged and skipped.
    app.add_prefab_dir(dir_str);
    let lib = app.resource_or_default::<PrefabLibrary>();
    assert_eq!(lib.len(), 1, "the well-formed prefab still loaded");
    assert_eq!(lib.error_count(), 1, "the corrupt file recorded one error");

    // But the boot gate must refuse to bring a zone with a degraded library
    // up "healthy".
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_prefab_library(&mut app, "test-zone");
    }));
    assert!(result.is_err(), "check_prefab_library must panic on a load error");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn clean_prefab_library_passes_boot_check() {
    let mut app = App::new();
    let lib = app.resource_or_default::<PrefabLibrary>();
    lib.insert("ok", engine_core::prefab::PrefabDef { components: Default::default() });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_prefab_library(&mut app, "test-zone");
    }));
    assert!(result.is_ok(), "a clean, non-empty library must pass the boot check");
}
