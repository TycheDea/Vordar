// Chapter 1 — a content module. Proves the engine / vordar-game / chapter split:
// this crate ships its own chapter definition (camps, initial spawns, world
// layout), its prefab directory (enemy types, props), and enemy archetype
// modules (enemies/). Cross-module communication is purely EventBus
// (HealthDepleted, Killed); nothing in the engine or vordar-game knows this
// crate exists.

pub mod enemies;

use engine_app::app::App;
use engine_app::plugin::Plugin;
use vordar_game::chapter::ChapterModule;

/// This chapter as a linked module — registered with the binaries'
/// ChapterRegistry. First chapter: requires nothing.
pub fn module() -> ChapterModule {
    ChapterModule {
        name: "chapter01",
        requires: &[],
        install: |app| {
            app.add_plugin(Chapter01Plugin);
        },
        install_content: |app| {
            app.add_plugin(Chapter01ContentPlugin);
        },
    }
}

/// Registration-only subset: component loaders + prefab definitions, no
/// simulation. The networked client needs this to display replicated chapter
/// entities — the chapter's systems run on the server only.
pub struct Chapter01ContentPlugin;

impl Plugin for Chapter01ContentPlugin {
    fn build(&self, app: &mut App) {
        app.add_prefab_dir("content/chapters/chapter01/prefabs");
    }
}

pub struct Chapter01Plugin;

impl Plugin for Chapter01Plugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(Chapter01ContentPlugin)
            .insert_resource(vordar_game::chapter::load_chapter("content/chapters/chapter01/chapter.ron"));
    }
}
