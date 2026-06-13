// Scheduler — phase-based system execution with DAG ordering
//
// Systems are registered with a Phase and a SystemOrder.
// At startup, systems within each phase are topologically sorted (once).
// At runtime, each phase runs at its configured TickRate:
//   Fixed(hz) — accumulates wall-clock time; fires N times per frame at a constant delta
//   Render    — fires exactly once per frame; receives the actual frame delta
//
// Phases always execute in Phase declaration order.
// A Fixed phase fires all pending steps before the next phase executes.
// If a cycle is detected during startup sort, the app panics with a clear message.

use crate::tick_rate::TickRate;
use engine_core::traits::Resources;
use engine_core::World;
use std::any::TypeId;
use std::collections::{BTreeMap, HashMap, VecDeque};

// ── InterpolationAlpha ────────────────────────────────────────────────────────
//
// Written by the scheduler each frame just before render phases execute.
// Value is `accumulator / fixed_dt` — the fractional step into the next fixed tick.
// RenderSyncSystem reads this to lerp between PreviousTransform and Transform.

pub struct InterpolationAlpha(pub f32);

// ── Phase ─────────────────────────────────────────────────────────────────────

/// Fixed frame execution order. Systems are assigned to exactly one phase.
/// Each phase has a default TickRate; override it with App::set_phase_rate().
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Phase {
    Input,              // read hardware, update input state
    PreUpdate,          // timers, cooldowns, recharge
    Update,             // AI, movement intent, auto-attack intent
    SpawnFlush,         // drain SpawnQueue — new entities available next phase
    Collision,          // broadphase (spatial grid) → narrowphase (shape tests)
    CollisionResolve,   // pushback, damage, on_hit responses
    DespawnFlush,       // call on_despawn → remove entities → free render slots
    PostUpdate,         // BodyPart follow, CellOccupant update, camera follow
    RenderSync,         // sync transforms to GPU, frustum cull
    Render,             // shadow pass → main render pass
}

impl Phase {
    /// Default tick rate for this phase.
    /// Override per-phase with App::set_phase_rate() or Scheduler::set_phase_rate().
    pub fn default_tick_rate(self) -> TickRate {
        match self {
            Phase::RenderSync | Phase::Render => TickRate::Render,
            _                                 => TickRate::Fixed(60.0),
        }
    }
}

// ── SystemOrder ───────────────────────────────────────────────────────────────

/// Ordering constraint within a phase.
/// The scheduler performs a topological sort; cycles cause a startup panic.
pub enum SystemOrder {
    /// Run before all Default and Last systems in this phase.
    First,
    /// No ordering constraint relative to other Default systems.
    Default,
    /// Run after all First and Default systems in this phase.
    Last,
    /// Run immediately after the named system type.
    After(TypeId),
    /// Run immediately before the named system type.
    Before(TypeId),
}

impl SystemOrder {
    pub fn after<S: System + 'static>() -> Self {
        Self::After(TypeId::of::<S>())
    }
    pub fn before<S: System + 'static>() -> Self {
        Self::Before(TypeId::of::<S>())
    }
}

// ── System trait ──────────────────────────────────────────────────────────────

pub trait System: 'static {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32);
}

// ── PhaseEntry ────────────────────────────────────────────────────────────────

struct PhaseEntry {
    systems:     Vec<Box<dyn System>>,
    fixed_dt:    f32,   // 0.0 when is_render
    accumulator: f32,
    is_render:   bool,
}

// ── Scheduler ────────────────────────────────────────────────────────────────

pub struct Scheduler {
    pending:        BTreeMap<Phase, Vec<(Box<dyn System>, TypeId, SystemOrder)>>,
    rate_overrides: BTreeMap<Phase, TickRate>,
    phases:         BTreeMap<Phase, PhaseEntry>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            pending:        BTreeMap::new(),
            rate_overrides: BTreeMap::new(),
            phases:         BTreeMap::new(),
        }
    }

    /// Register a system. Called before build().
    pub fn add<S: System>(&mut self, system: S, phase: Phase, order: SystemOrder) {
        self.pending
            .entry(phase)
            .or_default()
            .push((Box::new(system), TypeId::of::<S>(), order));
    }

    /// Override the tick rate for a phase.
    /// Must be called before build(). Defaults come from Phase::default_tick_rate().
    pub fn set_phase_rate(&mut self, phase: Phase, rate: TickRate) {
        self.rate_overrides.insert(phase, rate);
    }

    /// Topological sort of each phase. Called once after all systems are registered.
    /// Panics on cycle with a descriptive message showing the involved phase.
    pub fn build(&mut self) {
        for (phase, items) in std::mem::take(&mut self.pending) {
            let mut first  = Vec::new();
            let mut middle = Vec::new();
            let mut last   = Vec::new();

            for (system, type_id, order) in items {
                match order {
                    SystemOrder::First => first.push(system),
                    SystemOrder::Last  => last.push(system),
                    order              => middle.push((system, type_id, order)),
                }
            }

            let index_of: HashMap<TypeId, usize> = middle
                .iter()
                .enumerate()
                .map(|(i, (_, type_id, _))| (*type_id, i))
                .collect();

            let mut adjacency: Vec<Vec<usize>> = vec![vec![]; middle.len()];
            for (i, (_, _, order)) in middle.iter().enumerate() {
                match order {
                    SystemOrder::After(target_id) => {
                        if let Some(&j) = index_of.get(target_id) {
                            adjacency[j].push(i);
                        }
                    }
                    SystemOrder::Before(target_id) => {
                        if let Some(&j) = index_of.get(target_id) {
                            adjacency[i].push(j);
                        }
                    }
                    _ => {}
                }
            }

            let mut in_degree = vec![0usize; middle.len()];
            for neighbors in &adjacency {
                for &j in neighbors { in_degree[j] += 1; }
            }

            let mut queue: VecDeque<usize> = in_degree
                .iter()
                .enumerate()
                .filter_map(|(i, &d)| if d == 0 { Some(i) } else { None })
                .collect();

            let n = middle.len();
            let mut middle: Vec<Option<Box<dyn System>>> =
                middle.into_iter().map(|(system, _, _)| Some(system)).collect();

            let mut sorted: Vec<Box<dyn System>> = Vec::with_capacity(n);
            while let Some(i) = queue.pop_front() {
                sorted.push(middle[i].take().unwrap());
                for &j in &adjacency[i] {
                    in_degree[j] -= 1;
                    if in_degree[j] == 0 { queue.push_back(j); }
                }
            }

            if sorted.len() != n {
                panic!("Cycle detected in phase {:?} — check After/Before constraints", phase);
            }

            let mut systems = Vec::with_capacity(first.len() + sorted.len() + last.len());
            systems.extend(first);
            systems.extend(sorted);
            systems.extend(last);

            let rate = self.rate_overrides
                .get(&phase)
                .copied()
                .unwrap_or_else(|| phase.default_tick_rate());

            let entry = match rate {
                TickRate::Render => PhaseEntry {
                    systems,
                    fixed_dt:    0.0,
                    accumulator: 0.0,
                    is_render:   true,
                },
                TickRate::Fixed(hz) => PhaseEntry {
                    systems,
                    fixed_dt:    1.0 / hz,
                    accumulator: 0.0,
                    is_render:   false,
                },
            };

            self.phases.insert(phase, entry);
        }
    }

    /// Advance all phases by one display frame.
    ///
    /// Fixed phases accumulate `frame_delta` and fire however many steps their
    /// budget allows (capped at 8 to prevent spiral-of-death on lag spikes).
    /// Render phases fire exactly once, receiving `frame_delta` directly.
    ///
    /// Phases always execute in Phase declaration order.
    pub fn run_tick(&mut self, world: &mut World, resources: &mut Resources, frame_delta: f32) {
        let mut last_alpha = 0.0f32;

        for (_phase, entry) in &mut self.phases {
            if entry.is_render {
                // Expose the sub-step fraction to render systems before they run.
                if let Some(alpha) = resources.get_mut::<InterpolationAlpha>() {
                    alpha.0 = last_alpha;
                }
                for system in &mut entry.systems {
                    system.run(world, resources, frame_delta);
                }
            } else {
                // Cap accumulator to 8 steps to avoid runaway catch-up after long pauses.
                entry.accumulator =
                    (entry.accumulator + frame_delta).min(entry.fixed_dt * 8.0);
                while entry.accumulator >= entry.fixed_dt {
                    for system in &mut entry.systems {
                        system.run(world, resources, entry.fixed_dt);
                    }
                    entry.accumulator -= entry.fixed_dt;
                }
                last_alpha = entry.accumulator / entry.fixed_dt;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::traits::Resources;
    use engine_core::World;
    use std::sync::{Arc, Mutex};

    // Helper: a system that appends its name to a shared log on each run.
    struct LogSystem {
        name:  &'static str,
        log:   Arc<Mutex<Vec<&'static str>>>,
        delta: Arc<Mutex<Vec<f32>>>,
    }

    impl System for LogSystem {
        fn run(&mut self, _world: &mut World, _resources: &mut Resources, delta: f32) {
            self.log.lock().unwrap().push(self.name);
            self.delta.lock().unwrap().push(delta);
        }
    }

    fn make_system(
        name:  &'static str,
        log:   Arc<Mutex<Vec<&'static str>>>,
        delta: Arc<Mutex<Vec<f32>>>,
    ) -> LogSystem {
        LogSystem { name, log, delta }
    }

    #[test]
    fn first_default_last_ordering_respected() {
        let log   = Arc::new(Mutex::new(Vec::new()));
        let delta = Arc::new(Mutex::new(Vec::new()));

        let mut sched = Scheduler::new();
        sched.add(make_system("last",    log.clone(), delta.clone()), Phase::Update, SystemOrder::Last);
        sched.add(make_system("first",   log.clone(), delta.clone()), Phase::Update, SystemOrder::First);
        sched.add(make_system("default", log.clone(), delta.clone()), Phase::Update, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        // 1/60 s frame — enough for one 60 Hz step
        sched.run_tick(&mut world, &mut resources, 1.0 / 60.0);

        assert_eq!(*log.lock().unwrap(), vec!["first", "default", "last"]);
    }

    #[test]
    fn after_before_constraints_respected() {
        let log   = Arc::new(Mutex::new(Vec::new()));
        let delta = Arc::new(Mutex::new(Vec::new()));

        struct A;
        impl System for A {
            fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {}
        }

        let mut sched = Scheduler::new();
        // "after_a" must run after A; registered first to stress the sort
        sched.add(make_system("after_a", log.clone(), delta.clone()),
                  Phase::Update, SystemOrder::after::<A>());
        sched.add(A, Phase::Update, SystemOrder::Default);
        sched.add(make_system("before_a", log.clone(), delta.clone()),
                  Phase::Update, SystemOrder::before::<A>());
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        sched.run_tick(&mut world, &mut resources, 1.0 / 60.0);

        let order = log.lock().unwrap();
        let before_pos = order.iter().position(|&s| s == "before_a").unwrap();
        let after_pos  = order.iter().position(|&s| s == "after_a").unwrap();
        assert!(before_pos < after_pos, "before_a must run before after_a");
    }

    #[test]
    #[should_panic(expected = "Cycle detected")]
    fn cycle_in_constraints_panics_at_build() {
        struct X;
        struct Y;
        impl System for X { fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {} }
        impl System for Y { fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {} }

        let mut sched = Scheduler::new();
        sched.add(X, Phase::Update, SystemOrder::after::<Y>());
        sched.add(Y, Phase::Update, SystemOrder::after::<X>());
        sched.build();
    }

    #[test]
    fn fixed_rate_fires_correct_number_of_steps() {
        let log   = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let delta = Arc::new(Mutex::new(Vec::<f32>::new()));

        let mut sched = Scheduler::new();
        sched.set_phase_rate(Phase::Update, TickRate::Fixed(60.0));
        sched.add(make_system("step", log.clone(), delta.clone()),
                  Phase::Update, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();

        // 2.5 steps worth of time → fires exactly 2 steps (avoids fp boundary issues)
        sched.run_tick(&mut world, &mut resources, 2.5 / 60.0);
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    #[test]
    fn fixed_rate_delta_is_constant() {
        let log   = Arc::new(Mutex::new(Vec::new()));
        let delta = Arc::new(Mutex::new(Vec::new()));

        let mut sched = Scheduler::new();
        sched.set_phase_rate(Phase::Update, TickRate::Fixed(60.0));
        sched.add(make_system("s", log.clone(), delta.clone()),
                  Phase::Update, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        sched.run_tick(&mut world, &mut resources, 0.050); // 3 steps

        let deltas = delta.lock().unwrap();
        let expected = 1.0_f32 / 60.0;
        for &d in deltas.iter() {
            assert!((d - expected).abs() < 1e-6, "expected {expected}, got {d}");
        }
    }

    #[test]
    fn render_phase_fires_exactly_once_and_receives_frame_delta() {
        let log   = Arc::new(Mutex::new(Vec::new()));
        let delta = Arc::new(Mutex::new(Vec::new()));

        let mut sched = Scheduler::new();
        sched.set_phase_rate(Phase::Render, TickRate::Render);
        sched.add(make_system("r", log.clone(), delta.clone()),
                  Phase::Render, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        sched.run_tick(&mut world, &mut resources, 0.050);

        assert_eq!(log.lock().unwrap().len(), 1);
        assert!((delta.lock().unwrap()[0] - 0.050).abs() < 1e-6);
    }

    #[test]
    fn spiral_of_death_capped_at_8_steps() {
        let log   = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let delta = Arc::new(Mutex::new(Vec::<f32>::new()));

        let mut sched = Scheduler::new();
        sched.set_phase_rate(Phase::Update, TickRate::Fixed(60.0));
        sched.add(make_system("s", log.clone(), delta.clone()),
                  Phase::Update, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        // 1 second lag spike — without cap this would fire 60 steps
        sched.run_tick(&mut world, &mut resources, 1.0);
        assert_eq!(log.lock().unwrap().len(), 8);
    }
}