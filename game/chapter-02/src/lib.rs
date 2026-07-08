// Chapter 2 — Emberwood Rest: the first town. Buildings and villagers are
// initial_spawns (spawn once, never respawn — nothing here can die); the
// monster camps outside the walls reuse chapter-01 archetypes plus one local
// prefab, via the same CampSystem. Requires chapter01: the camps reference
// its prefab ids ("grunt", "mossback"), and `requires` guarantees those
// loaders/prefabs are installed wherever this chapter runs.
//
// Camp placement discipline (no navmesh — enemies beeline): every camp's
// aggro bubble (radius + aggro_range) stays clear of the buildings' hitboxes
// and the portal corridor along -X, so a straight-line chase never clips a
// wall corner.

use engine_app::app::App;
use engine_app::plugin::Plugin;
use vordar_game::chapter::ChapterModule;

/// This chapter as a linked module — registered with the binaries'
/// ChapterRegistry.
pub fn module() -> ChapterModule {
    ChapterModule {
        name: "chapter02",
        requires: &["chapter01"],
        install: |app| {
            app.add_plugin(Chapter02Plugin);
        },
        install_content: |app| {
            app.add_plugin(Chapter02ContentPlugin);
        },
    }
}

/// Marker for friendly townsfolk. Deliberately paired with NO Health in the
/// NPC prefabs: every damage path (mechanics, projectiles, contact) requires
/// Health on the target, so villagers are immune by construction.
#[derive(Clone, serde::Deserialize)]
pub struct Npc;

/// Registration-only subset: component loaders + prefab definitions, no
/// simulation (networked display clients, dependency installs).
pub struct Chapter02ContentPlugin;

impl Plugin for Chapter02ContentPlugin {
    fn build(&self, app: &mut App) {
        app.register_component::<Npc>("Npc")
            .add_prefab_dir("content/chapters/chapter02/prefabs");
    }
}

pub struct Chapter02Plugin;

impl Plugin for Chapter02Plugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(Chapter02ContentPlugin)
            .insert_resource(vordar_game::chapter::load_chapter("content/chapters/chapter02/chapter.ron"));
    }
}
