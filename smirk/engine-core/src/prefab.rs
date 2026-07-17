// Prefab system — data-driven entity definitions
//
// ComponentRegistry maps string names to loaders that deserialize a RON value
// into a ready-to-apply CompiledComponent. PrefabLibrary maps prefab ids to
// component maps kept as raw RON text (full enum fidelity — parsed with the
// typed deserializer on first spawn) plus the compiled plan built from it.
// Both are resources, populated by plugins at startup.
//
// Cost model: RON parsing happens once per prefab (the first spawn compiles
// the plan); every spawn after that is one clone per component.
//
// This is also the modding seam: anything addressable here by string
// (component names, prefab ids) is reachable from data files today and from
// a scripting layer later without touching engine code.

use crate::components::{Anchored, CellOccupant, Health, Hitbox, PointLight, PreviousTransform, RenderMesh, RenderShape, ShapeGroup, Solid, Transform, Velocity};
use crate::traits::{Resources, SpawnContext, SpawnQueue};
use glam::Vec3;
use hecs::{Entity, EntityBuilder};
use ron::value::RawValue;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::OnceLock;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum PrefabError {
    UnknownPrefab(String),
    UnknownComponent(String),
    Parse { component: String, error: Box<ron::error::SpannedError> },
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

/// One parsed component, ready to add to an EntityBuilder — the unit of a
/// prefab's compiled spawn plan. Applying it is a clone, never a parse.
pub type CompiledComponent = Box<dyn Fn(&mut EntityBuilder) + Send + Sync>;

pub type ComponentLoader =
    Box<dyn Fn(&RawValue) -> Result<CompiledComponent, PrefabError> + Send + Sync>;

pub struct ComponentRegistry {
    loaders: HashMap<String, ComponentLoader>,
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self { loaders: HashMap::new() }
    }

    /// Register any Deserialize + Clone component under a name — the generic
    /// path used by engine and game crates alike.
    pub fn register<T>(&mut self, name: &str)
    where
        T: hecs::Component + serde::de::DeserializeOwned + Clone,
    {
        let component = name.to_owned();
        self.register_with(name, move |raw| {
            let value: T = raw.into_rust().map_err(|error| PrefabError::Parse {
                component: component.clone(),
                error: Box::new(error),
            })?;
            Ok(Box::new(move |builder: &mut EntityBuilder| {
                builder.add(value.clone());
            }) as CompiledComponent)
        });
    }

    /// Register a custom loader — the compiled closure may add multiple
    /// components (e.g. inject engine bookkeeping alongside the declared one).
    pub fn register_with(
        &mut self,
        name: &str,
        f: impl Fn(&RawValue) -> Result<CompiledComponent, PrefabError> + Send + Sync + 'static,
    ) {
        if self.loaders.insert(name.to_owned(), Box::new(f)).is_some() {
            log::warn!("component loader '{name}' was overwritten");
        }
    }

    /// Parse a raw RON value into its ready-to-apply form.
    pub fn compile(&self, name: &str, raw: &RawValue) -> Result<CompiledComponent, PrefabError> {
        let loader = self.loaders.get(name)
            .ok_or_else(|| PrefabError::UnknownComponent(name.to_owned()))?;
        loader(raw)
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

/// A definition plus its compiled spawn plan. The plan is built lazily on
/// first spawn (OnceLock initializes through &self, so it works under the
/// shared Resources borrow spawn_prefab holds) and never invalidated —
/// insert() replaces the whole entry, and prefabs are not hot-reloaded.
struct PrefabEntry {
    def: PrefabDef,
    plan: OnceLock<Vec<CompiledComponent>>,
}

pub struct PrefabLibrary {
    prefabs: HashMap<String, PrefabEntry>,
}

impl Default for PrefabLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefabLibrary {
    pub fn new() -> Self {
        Self { prefabs: HashMap::new() }
    }

    pub fn insert(&mut self, id: impl Into<String>, def: PrefabDef) {
        let id = id.into();
        let entry = PrefabEntry { def, plan: OnceLock::new() };
        if self.prefabs.insert(id.clone(), entry).is_some() {
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
        self.prefabs.get(id).map(|entry| &entry.def)
    }

    pub fn len(&self) -> usize { self.prefabs.len() }
    pub fn is_empty(&self) -> bool { self.prefabs.is_empty() }

    /// Every prefab id in this library, sorted for a deterministic order —
    /// the per-zone prefab table's `u16` index assignment must be stable
    /// across a `PrefabTable` build.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.prefabs.keys().cloned().collect();
        names.sort();
        names
    }
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
        let entry = library.prefabs.get(id).ok_or_else(|| PrefabError::UnknownPrefab(id.to_owned()))?;
        let plan = match entry.plan.get() {
            Some(plan) => plan,
            None => {
                // First spawn: parse the RON into the compiled plan. Errors
                // are returned, not cached — a bad component re-reports on
                // every attempt.
                let mut compiled = Vec::with_capacity(entry.def.components.len());
                for (name, raw) in &entry.def.components {
                    compiled.push(registry.compile(name, raw)?);
                }
                entry.plan.get_or_init(|| compiled)
            }
        };
        for component in plan {
            component(&mut builder);
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
    reg.register_with("Transform", |raw| {
        let t: Transform = raw.into_rust().map_err(|error| PrefabError::Parse {
            component: "Transform".into(),
            error: Box::new(error),
        })?;
        Ok(Box::new(move |builder: &mut EntityBuilder| {
            builder.add(PreviousTransform { position: t.position });
            builder.add(t.clone());
        }) as CompiledComponent)
    });
    reg.register_with("Hitbox", |raw| {
        let h: Hitbox = raw.into_rust().map_err(|error| PrefabError::Parse {
            component: "Hitbox".into(),
            error: Box::new(error),
        })?;
        Ok(Box::new(move |builder: &mut EntityBuilder| {
            builder.add(h.clone());
            builder.add(CellOccupant { cells: SmallVec::new() });
        }) as CompiledComponent)
    });
    reg.register::<Velocity>("Velocity");
    reg.register::<Health>("Health");
    reg.register::<Solid>("Solid");
    reg.register::<Anchored>("Anchored");
    reg.register::<RenderShape>("RenderShape");
    reg.register::<ShapeGroup>("ShapeGroup");
    reg.register::<RenderMesh>("RenderMesh");
    reg.register::<PointLight>("PointLight");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_light_registers_and_defaults_offset_and_flicker() {
        let mut registry = ComponentRegistry::new();
        register_core_components(&mut registry);

        let raw: Box<RawValue> = ron::from_str("(color: (1.0, 0.6, 0.2), intensity: 12.0, radius: 6.0)").unwrap();
        let compiled = registry.compile("PointLight", &raw).expect("PointLight must be registered");

        let mut builder = EntityBuilder::new();
        compiled(&mut builder);
        let mut world = hecs::World::new();
        let entity = world.spawn(builder.build());
        let light = world.get::<&PointLight>(entity).unwrap();
        assert_eq!(light.color, Vec3::new(1.0, 0.6, 0.2));
        assert_eq!(light.intensity, 12.0);
        assert_eq!(light.radius, 6.0);
        assert_eq!(light.offset, Vec3::ZERO);
        assert_eq!(light.flicker, 0.0);
    }
}
