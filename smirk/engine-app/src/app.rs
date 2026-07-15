// App — builder API and main loop
//
// Usage (in client/vordar-client/src/bin/sandbox.rs):
//
//   App::new()
//       .configure("content/config/engine.ron")
//       .set_phase_rate(Phase::Update, TickRate::Fixed(30.0))
//       .add_plugin(RenderPlugin)
//       .add_plugin(PhysicsPlugin)
//       .add_system(EnemyAI,        Phase::Update,           SystemOrder::Default)
//       .add_system(MovementSystem, Phase::Update,           SystemOrder::after::<EnemyAI>())
//       .run();
//
// Builder methods take &mut self so plugins can configure the App through the
// same API as the binary; chain them in a single expression ending in .run().

use crate::config::WindowConfig;
#[cfg(feature = "winit")]
use crate::config::WindowMode;
use crate::dev_stats::DevStats;
use crate::events::EventBus;
use crate::flush::{ClearEventsSystem, DespawnFlushSystem, SpawnFlushSystem};
#[cfg(feature = "winit")]
use crate::input::{KeyboardState, MouseState};
use crate::plugin::Plugin;
use crate::scheduler::{InterpolationAlpha, Phase, Scheduler, System, SystemOrder};
use crate::tick_rate::TickRate;
use crate::time::Time;
use engine_core::prefab::{ComponentRegistry, PrefabLibrary};
use engine_core::traits::{DespawnQueue, Resources, SpawnQueue};
use engine_core::World;

/// Resource a system can set to end `run_headless`'s loop after the current
/// tick — e.g. a shutdown system that must finish saving before the loop
/// exits. `App::new` inserts `AppExit(false)`; only the fixed-step system
/// that flips it decides when, so save-and-exit stay causally ordered within
/// the same tick.
pub struct AppExit(pub bool);

pub struct App {
    pub(crate) world:          World,
    pub(crate) resources:      Resources,
    pub(crate) scheduler:      Scheduler,
    pub(crate) last_tick:      std::time::Instant,
    #[cfg(feature = "winit")]
    pub(crate) window:         Option<std::sync::Arc<winit::window::Window>>,
    #[cfg(feature = "winit")]
    pub(crate) on_init:        Vec<Box<dyn FnOnce(&std::sync::Arc<winit::window::Window>, &mut Resources)>>,
    #[cfg(feature = "winit")]
    pub(crate) on_resize:      Vec<Box<dyn FnMut(u32, u32, &mut Resources)>>,
    /// Path to the config file — kept for hot-reload and persist-on-exit.
    pub(crate) config_path:    Option<String>,
    /// File watcher + event receiver for hot-reloading engine.ron.
    pub(crate) config_watcher: Option<(
        notify::RecommendedWatcher,
        std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    )>,
}

impl App {
    pub fn new() -> Self {
        crate::logger::init();
        let world = World::new();
        let mut resources = Resources::new();

        resources.insert(SpawnQueue::new());
        resources.insert(DespawnQueue::new());
        resources.insert(EventBus::new());
        resources.insert(Time::new());
        #[cfg(feature = "winit")]
        resources.insert(KeyboardState::new());
        #[cfg(feature = "winit")]
        resources.insert(MouseState::new());
        resources.insert(InterpolationAlpha(0.0));
        resources.insert(DevStats::new());
        resources.insert(AppExit(false));

        let mut scheduler = Scheduler::new();
        scheduler.add(ClearEventsSystem,  Phase::Input,        SystemOrder::First);
        scheduler.add(SpawnFlushSystem,   Phase::SpawnFlush,   SystemOrder::Default);
        scheduler.add(DespawnFlushSystem, Phase::DespawnFlush, SystemOrder::Default);

        Self {
            world,
            resources,
            scheduler,
            last_tick:      std::time::Instant::now(),
            #[cfg(feature = "winit")]
            window:         None,
            #[cfg(feature = "winit")]
            on_init:        Vec::new(),
            #[cfg(feature = "winit")]
            on_resize:      Vec::new(),
            config_path:    None,
            config_watcher: None,
        }
    }

    /// Load window config from a RON file and insert it as a resource.
    /// Falls back to WindowConfig::default() if the file is missing or fails to parse.
    /// Also sets up a file watcher for hot-reload.
    pub fn configure(&mut self, path: &str) -> &mut Self {
        let config = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| ron::from_str::<WindowConfig>(&s)
                .map_err(|e| log::warn!("config parse error in {path}: {e}"))
                .ok())
            .unwrap_or_default();
        self.resources.insert(config);
        self.config_path = Some(path.to_owned());

        // Set up file watcher — events are polled each frame via try_recv.
        use notify::{RecursiveMode, Watcher};
        let (tx, rx) = std::sync::mpsc::channel();
        match notify::recommended_watcher(move |res| { let _ = tx.send(res); }) {
            Ok(mut watcher) => {
                if let Err(e) = watcher.watch(std::path::Path::new(path), RecursiveMode::NonRecursive) {
                    log::warn!("config hot-reload watcher failed to start: {e}");
                } else {
                    self.config_watcher = Some((watcher, rx));
                }
            }
            Err(e) => log::warn!("config hot-reload watcher unavailable: {e}"),
        }

        self
    }

    /// Override the tick rate for a phase.
    /// Default rates: Fixed(60.0) for all logic phases, Render for RenderSync and Render.
    pub fn set_phase_rate(&mut self, phase: Phase, rate: TickRate) -> &mut Self {
        self.scheduler.set_phase_rate(phase, rate);
        self
    }

    /// Register a system with a phase and ordering constraint.
    pub fn add_system<S: System>(&mut self, system: S, phase: Phase, order: SystemOrder) -> &mut Self {
        self.scheduler.add(system, phase, order);
        self
    }

    /// Insert a custom resource accessible to all systems via resources.get_mut::<T>().
    pub fn insert_resource<T: std::any::Any + Send + Sync>(&mut self, resource: T) -> &mut Self {
        self.resources.insert(resource);
        self
    }

    /// Get-or-insert-default a shared registry resource at build time — lets
    /// multiple plugins contribute to the same registry regardless of plugin
    /// order (same pattern as register_component's ComponentRegistry).
    pub fn resource_or_default<T: std::any::Any + Send + Sync + Default>(&mut self) -> &mut T {
        if !self.resources.contains::<T>() {
            self.resources.insert(T::default());
        }
        self.resources.get_mut::<T>().unwrap()
    }

    /// Register a callback invoked once after the OS window is created.
    /// Use this to initialize renderer resources that require a window handle.
    /// Callbacks fire in registration order — multiple plugins may each add one.
    #[cfg(feature = "winit")]
    pub fn on_window_ready(
        &mut self,
        f: impl FnOnce(&std::sync::Arc<winit::window::Window>, &mut Resources) + 'static,
    ) -> &mut Self {
        self.on_init.push(Box::new(f));
        self
    }

    /// Register a callback invoked on every WindowEvent::Resized.
    /// Callbacks fire in registration order — multiple plugins may each add one.
    #[cfg(feature = "winit")]
    pub fn on_resize_fn(
        &mut self,
        f: impl FnMut(u32, u32, &mut Resources) + 'static,
    ) -> &mut Self {
        self.on_resize.push(Box::new(f));
        self
    }

    /// Apply a plugin — the plugin configures the App through this same builder API.
    pub fn add_plugin(&mut self, plugin: impl Plugin) -> &mut Self {
        log::info!("plugin registered: {}", plugin.name());
        plugin.build(self);
        self
    }

    /// Register a component type for data-driven spawning under a string name.
    /// Get-or-inserts the ComponentRegistry, so plugin order never matters.
    pub fn register_component<T>(&mut self, name: &str) -> &mut Self
    where
        T: hecs::Component + serde::de::DeserializeOwned + Clone,
    {
        if !self.resources.contains::<ComponentRegistry>() {
            self.resources.insert(ComponentRegistry::new());
        }
        self.resources.get_mut::<ComponentRegistry>().unwrap().register::<T>(name);
        self
    }

    /// Load every *.ron prefab in `dir` into the PrefabLibrary (get-or-insert).
    /// Multiple plugins may each contribute their own prefab directories.
    pub fn add_prefab_dir(&mut self, dir: &str) -> &mut Self {
        if !self.resources.contains::<PrefabLibrary>() {
            self.resources.insert(PrefabLibrary::new());
        }
        self.resources.get_mut::<PrefabLibrary>().unwrap().load_dir(dir);
        self
    }

    /// Change window mode at runtime. Requires the window to be ready (after on_window_ready fires).
    /// Systems can also call this directly via `resources.get::<Arc<winit::window::Window>>()`.
    #[cfg(feature = "winit")]
    pub fn set_window_mode(&mut self, mode: WindowMode) {
        use std::sync::Arc;
        use winit::window::Window;

        if let Some(window) = self.resources.get::<Arc<Window>>() {
            let resolution = self.resources.get::<WindowConfig>()
                .map(|c| c.resolution.clone())
                .unwrap_or(crate::config::Resolution::Fixed(1280, 720));
            let fullscreen = crate::config::resolve_fullscreen(&mode, &resolution, window.current_monitor());
            window.set_fullscreen(fullscreen);
        }
        if let Some(cfg) = self.resources.get_mut::<WindowConfig>() {
            cfg.mode = mode;
        }
    }

    /// Finalize system ordering and start the game loop.
    #[cfg(feature = "winit")]
    pub fn run(&mut self) {
        self.scheduler.build();
        let event_loop = winit::event_loop::EventLoop::new()
            .expect("failed to create event loop");
        event_loop.run_app(self).expect("event loop error");
    }

    /// Finalize system ordering and run without a window — the dedicated-server loop.
    /// Ticks at `hz` (wall-clock), sleeping between ticks. `max_ticks: None` runs forever.
    pub fn run_headless(&mut self, hz: f64, max_ticks: Option<u64>) {
        self.scheduler.build();
        let budget = std::time::Duration::from_secs_f64(1.0 / hz);
        self.last_tick = std::time::Instant::now();
        let mut next_tick = self.last_tick + budget;
        let mut ticks: u64 = 0;
        loop {
            let now   = std::time::Instant::now();
            let delta = now.duration_since(self.last_tick).as_secs_f32().min(0.1);
            self.last_tick = now;
            self.tick(delta);

            ticks += 1;
            if max_ticks.is_some_and(|max| ticks >= max) { break; }
            if self.resources.get::<AppExit>().is_some_and(|e| e.0) { break; }

            let now = std::time::Instant::now();
            if next_tick > now {
                std::thread::sleep(next_tick - now);
            }
            next_tick += budget;
        }
    }

    /// Build the schedule once, then run exactly `n` ticks back-to-back with a
    /// fixed dt — no sleeping, no wall-clock pacing. Benchmark/embedding seam.
    pub fn run_ticks(&mut self, dt: f32, n: u64) {
        self.scheduler.build();
        for _ in 0..n {
            self.tick(dt);
        }
    }

    pub(crate) fn tick(&mut self, frame_delta: f32) {
        self.resources.get_mut::<Time>().unwrap().frame_dt = frame_delta;
        self.resources.get_mut::<DevStats>().unwrap().record_frame(frame_delta);
        self.scheduler.run_tick(&mut self.world, &mut self.resources, frame_delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::SystemOrder;
    use engine_core::World;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    // A system that counts its own runs and requests app exit on the 5th run.
    struct ExitOnFifthTick {
        counter: Arc<AtomicU64>,
    }

    impl System for ExitOnFifthTick {
        fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
            let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 5 {
                resources.get_mut::<AppExit>().unwrap().0 = true;
            }
        }
    }

    #[test]
    fn run_headless_returns_after_appexit_set_and_stops_exactly_on_that_tick() {
        let counter = Arc::new(AtomicU64::new(0));

        let mut app = App::new();
        app.add_system(
            ExitOnFifthTick { counter: counter.clone() },
            Phase::Update,
            SystemOrder::Default,
        );

        // High hz so the fixed-step Update phase advances rapidly and the test
        // is fast; max_ticks is None — only AppExit can end this loop.
        app.run_headless(1000.0, None);

        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }
}