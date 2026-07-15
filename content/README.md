# content/ layout

- **Chapter ids** use no separator inside `content/` (`chapter01`, `chapter02`) —
  this matches the RON path strings code loads them by. The crate and lib names
  for the same chapter (`game/chapter-01`, lib `chapter_01`) are a different
  naming domain and follow ordinary Cargo convention (kebab-case package name,
  snake_case lib name); they are not expected to match the content id.
- **Folder = schema.** Each subfolder is loaded into its own library, keyed by
  RON filename stem (`chapters/`, `classes/`, `models/`, `prefabs/`, `races/`,
  `textures/`, `vfx/`, `zones/`). Because every folder loads independently, the
  same stem may exist in more than one folder without colliding —
  `content/classes/human.ron`, `content/races/human.ron`, and
  `content/prefabs/human.ron` are three unrelated definitions.
- **Plural vs. singular.** Folders are plural when they hold one file per
  content instance. `config/` and `source/` are deliberately singular:
  `config/` holds the one engine-wide settings file, not a category of many;
  `source/` holds raw or pre-processed material that hasn't become shipped
  content yet (it feeds `scripts/asset-pipeline/` conversions).
- **Test/sample assets** — anything loaded only by an engine unit test, never
  by the shipped game — live under `content/source/test/`, never inside a
  shipped folder such as `models/` or `textures/`.
- **Entity prefabs are named by the class they spawn** (`human.ron`,
  `ravager.ron`), not by role — "the player" is just whichever class prefab
  the server's `PLAYER_PREFAB` currently points at.
- **World description lives in one folder.** Zones and the world-clock/event
  schedule are both under `content/zones/` (`zones.ron`, `events.ron`); there
  is no separate `world/` folder.
