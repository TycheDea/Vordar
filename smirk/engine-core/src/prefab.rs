// Prefab system — data-driven entity definitions
//
// ComponentRegistry maps string names to loaders that deserialize a RON value
// into an EntityBuilder. PrefabLibrary maps prefab ids to component maps kept
// as raw RON text (full enum fidelity — re-parsed with the typed deserializer
// at spawn time). Both are resources, populated by plugins at startup.
//
// Cost model: one HashMap lookup + one RON parse per component per *spawn* —
// never on a per-frame path.
//
// This is also the modding seam: anything addressable here by string
// (component names, prefab ids) is reachable from data files today and from
// a scripting layer later without touching engine code.

use crate::components::{CellOccupant, Health, Hitbox, PreviousTransform, RenderShape, ShapeGroup, Solid, Transform, Velocity};
use crate::traits::{Resources, SpawnContext, SpawnQueue};
use glam::Vec3;
use hecs::{Entity, EntityBuilder};
use ron::value::RawValue;
use smallvec::SmallVec;
use std::collections::HashMap;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum PrefabError {
    UnknownPrefab(String),
    UnknownComponent(String),
    Parse { component: String, error: ron::error::SpannedError },
    RegistryMissing,
}

impl std::fmt::Display for PrefabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPrefab(id)      => write!(f, "unknown prefab '{id}'"),
            Self::UnknownComponent(name) => write!(f, "no loader registered for component '{name}'"),
            Self::Parse { component, error } => write!(f, "failed to parse component '{component}': {error}"),
            Self::RegistryMissing => write!(f, "ComponentRegistry/PrefabLibrary not in resources — add PrefabPlugin"),
        }
    }
}

impl std::error::Error for PrefabError {}

// ── ComponentRegistry ─────────────────────────────────────────────────────────

pub type ComponentLoader =
    Box<dyn Fn(&RawValue, &mut EntityBuilder) -> Result<(), PrefabError> + Send + Sync>;

pub struct ComponentRegistry {
    loaders: HashMap<String, ComponentLoader>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self { loaders: HashMap::new() }
    }

    /// Register any Deserialize component under a name — the generic path used
    /// by engine and game crates alike.
    pub fn register<T>(&mut self, name: &str)
    where
        T: hecs::Component + serde::de::DeserializeOwned,
    {
        let component = name.to_owned();
        self.register_with(name, move |raw, builder| {
            let value: T = raw.into_rust().map_err(|error| PrefabError::Parse {
                component: component.clone(),
                error,
            })?;
            builder.add(value);
            Ok(())
        });
    }

    /// Register a custom loader — may add multiple components (e.g. inject
    /// engine bookkeeping alongside the declared one).
    pub fn register_with(
        &mut self,
        name: &str,
        f: impl Fn(&RawValue, &mut EntityBuilder) -> Result<(), PrefabError> + Send + Sync + 'static,
    ) {
        if self.loaders.insert(name.to_owned(), Box::new(f)).is_some() {
            log::warn!("component loader '{name}' was overwritten");
        }
    }

    pub fn load(&self, name: &str, raw: &RawValue, builder: &mut EntityBuilder) -> Result<(), PrefabError> {
        let loader = self.loaders.get(name)
            .ok_or_else(|| PrefabError::UnknownComponent(name.to_owned()))?;
        loader(raw, builder)
    }

    pub fn len(&self) -> usize { self.loaders.len() }
    pub fn is_empty(&self) -> bool { self.loaders.is_empty() }
}

// ── PrefabLibrary ─────────────────────────────────────────────────────────────

/// One entity definition: component name → raw RON value.
#[derive(serde::Deserialize)]
pub struct PrefabDef {
    pub components: HashMap<String, Box<RawValue>>,
}

pub struct PrefabLibrary {
    prefabs: HashMap<String, PrefabDef>,
}

impl PrefabLibrary {
    pub fn new() -> Self {
        Self { prefabs: HashMap::new() }
    }

    pub fn insert(&mut self, id: impl Into<String>, def: PrefabDef) {
        let id = id.into();
        if self.prefabs.insert(id.clone(), def).is_some() {
            log::warn!("prefab '{id}' was overwritten");
        }
    }

    /// Load every *.ron file in `dir` as a prefab; the id is the file stem.
    /// Parse failures are logged and skipped — one bad file must not take the
    /// whole library down.
    pub fn load_dir(&mut self, dir: &str) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                log::error!("prefab dir '{dir}' unreadable: {e}");
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("ron") { continue; }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue; };
            match std::fs::read_to_string(&path) {
                Ok(text) => match ron::from_str::<PrefabDef>(&text) {
                    Ok(def) => {
                        log::info!("prefab loaded: '{stem}' ({} components)", def.components.len());
                        self.insert(stem, def);
                    }
                    Err(e) => log::error!("prefab '{}' parse error: {e}", path.display()),
                },
                Err(e) => log::error!("prefab '{}' read error: {e}", path.display()),
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&PrefabDef> {
        self.prefabs.get(id)
    }

    pub fn len(&self) -> usize { self.prefabs.len() }
    pub fn is_empty(&self) -> bool { self.prefabs.is_empty() }
}

// ── Spawning ──────────────────────────────────────────────────────────────────

/// Which prefab an entity was spawned from — attached automatically by
/// spawn_prefab. The replication layer uses it to tell clients what to spawn;
/// also handy for debugging.
pub struct PrefabId(pub String);

/// Build an entity from a prefab at `position`. Reads ComponentRegistry and
/// PrefabLibrary immutably from ctx.resources, then spawns into ctx.world.
/// Transform and PreviousTransform are both moved to `position` after the
/// build so interpolation starts clean (no first-frame lerp from the origin).
pub fn spawn_prefab(id: &str, position: Vec3, ctx: &mut SpawnContext) -> Result<Entity, PrefabError> {
    let mut builder = EntityBuilder::new();
    {
        let registry = ctx.resources.get::<ComponentRegistry>().ok_or(PrefabError::RegistryMissing)?;
        let library  = ctx.resources.get::<PrefabLibrary>().ok_or(PrefabError::RegistryMissing)?;
        let def = library.get(id).ok_or_else(|| PrefabError::UnknownPrefab(id.to_owned()))?;
        for (name, raw) in &def.components {
            registry.load(name, raw, &mut builder)?;
        }
    }
    builder.add(PrefabId(id.to_owned()));
    let entity = ctx.world.spawn(builder.build());
    if let Ok(mut t) = ctx.world.get::<&mut Transform>(entity) {
        t.position = position;
    }
    if let Ok(mut p) = ctx.world.get::<&mut PreviousTransform>(entity) {
        p.position = position;
    }
    Ok(entity)
}

/// Queue a prefab spawn by id — the string-addressable entry point used by
/// wave systems, chapter setup, and (later) scripting. Goes through SpawnQueue
/// so the world is never mutated mid-iteration; errors are logged, not fatal.
pub fn queue_prefab_spawn(resources: &mut Resources, prefab: impl Into<String>, position: Vec3) {
    let prefab = prefab.into();
    let Some(queue) = resources.get_mut::<SpawnQueue>() else {
        log::error!("queue_prefab_spawn('{prefab}'): SpawnQueue not in resources");
        return;
    };
    queue.push(move |ctx| {
        if let Err(e) = spawn_prefab(&prefab, position, ctx) {
            log::error!("prefab spawn '{prefab}' failed: {e}");
        }
    });
}

// ── Core component loaders ────────────────────────────────────────────────────

/// Register loaders for all engine-core components. Two are custom so prefab
/// authors never see engine bookkeeping:
///   "Transform" also injects PreviousTransform (render interpolation)
///   "Hitbox"    also injects an empty CellOccupant (spatial grid)
pub fn register_core_components(reg: &mut ComponentRegistry) {
    reg.register_with("Transform", |raw, builder| {
        let t: Transform = raw.into_rust().map_err(|error| PrefabError::Parse {
            component: "Transform".into(),
            error,
        })?;
        builder.add(PreviousTransform { position: t.position });
        builder.add(t);
        Ok(())
    });
    reg.register_with("Hitbox", |raw, builder| {
        let h: Hitbox = raw.into_rust().map_err(|error| PrefabError::Parse {
            component: "Hitbox".into(),
            error,
        })?;
        builder.add(h);
        builder.add(CellOccupant { cells: SmallVec::new() });
        Ok(())
    });
    reg.register::<Velocity>("Velocity");
    reg.register::<Health>("Health");
    reg.register::<Solid>("Solid");
    reg.register::<RenderShape>("RenderShape");
    reg.register::<ShapeGroup>("ShapeGroup");
}
