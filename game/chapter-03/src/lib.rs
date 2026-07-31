// Chapter 3 — Rocalba: the start town, collision shells only. Buildings are
// initial_spawns of anchored solid boxes (spawn once, nothing here can die);
// no NPCs, no camps, no systems beyond the shared chapter setup spawn.

use engine_app::app::App;
use engine_app::plugin::Plugin;
use vordar_game::chapter::ChapterModule;

/// This chapter as a linked module — registered with the binaries'
/// ChapterRegistry. Requires nothing: every prefab is local.
pub fn module() -> ChapterModule {
    ChapterModule {
        name: "chapter03",
        requires: &[],
        install: |app| {
            app.add_plugin(Chapter03Plugin);
        },
        install_content: |app| {
            app.add_plugin(Chapter03ContentPlugin);
        },
    }
}

/// Registration-only subset: prefab definitions, no simulation (networked
/// display clients, dependency installs).
pub struct Chapter03ContentPlugin;

impl Plugin for Chapter03ContentPlugin {
    fn build(&self, app: &mut App) {
        app.add_prefab_dir("content/chapters/chapter03/prefabs");
    }
}

pub struct Chapter03Plugin;

impl Plugin for Chapter03Plugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(Chapter03ContentPlugin)
            .insert_resource(vordar_game::chapter::load_chapter("content/chapters/chapter03/chapter.ron"));
    }
}
