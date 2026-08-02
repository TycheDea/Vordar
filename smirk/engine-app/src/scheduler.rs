// Scheduler — phase-based system execution with DAG ordering
//
// Systems are registered with a Phase and a SystemOrder.
// At startup, systems within each phase are topologically sorted (once).
// At runtime the frame runs off one app-wide fixed clock:
//   Fixed  — all Fixed phases run once per fixed step, in declaration order,
//            and the whole set repeats N times per frame at a constant delta
//   Render — fires exactly once per frame; receives the actual frame delta
//
// Fixed phases interleave per step (step 0: every fixed phase, then step 1: …),
// so a same-step spawn is visible to that step's later phases. Render phases
// run after all steps. If a cycle is detected during startup sort, the app
// panics with a clear message.

use engine_core::traits::Resources;
use engine_core::World;
use std::any::TypeId;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

// ── InterpolationAlpha ────────────────────────────────────────────────────────
//
// Written by the scheduler each frame just before render phases execute.
// Value is `accumulator / fixed_dt` — the fractional step into the next fixed tick.
// RenderSyncSystem reads this to lerp between PreviousTransform and Transform.

pub struct InterpolationAlpha(pub f32);

// ── Phase ─────────────────────────────────────────────────────────────────────

/// Fixed frame execution order. Systems are assigned to exactly one phase.
/// RenderSync and Render fire once per display frame; every other phase runs
/// at the app-wide fixed rate (see Scheduler::set_fixed_hz).
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
    /// Whether this phase fires once per display frame (true) or at the
    /// app-wide fixed rate (false).
    pub fn is_render(self) -> bool {
        matches!(self, Phase::RenderSync | Phase::Render)
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
    After(TypeId, &'static str),
    /// Run immediately before the named system type.
    Before(TypeId, &'static str),
}

impl SystemOrder {
    pub fn after<S: System + 'static>() -> Self {
        Self::After(TypeId::of::<S>(), std::any::type_name::<S>())
    }
    pub fn before<S: System + 'static>() -> Self {
        Self::Before(TypeId::of::<S>(), std::any::type_name::<S>())
    }
}

// ── System trait ──────────────────────────────────────────────────────────────

pub trait System: 'static {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32);
}

// ── PhaseEntry ────────────────────────────────────────────────────────────────

struct PhaseEntry {
    systems:   Vec<Box<dyn System>>,
    is_render: bool,
}

// ── Scheduler ────────────────────────────────────────────────────────────────

type PendingSystem = (Box<dyn System>, TypeId, &'static str, SystemOrder);

pub struct Scheduler {
    pending:        BTreeMap<Phase, Vec<PendingSystem>>,
    phases:         BTreeMap<Phase, PhaseEntry>,
    // One fixed clock for the whole app: every fixed phase steps off this
    // accumulator so all fixed phases interleave per step on a multi-step frame.
    fixed_dt:       f32,
    accumulator:    f32,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            pending:        BTreeMap::new(),
            phases:         BTreeMap::new(),
            fixed_dt:       1.0 / 60.0,
            accumulator:    0.0,
        }
    }

    /// Register a system. Called before build().
    pub fn add<S: System>(&mut self, system: S, phase: Phase, order: SystemOrder) {
        self.pending
            .entry(phase)
            .or_default()
            .push((Box::new(system), TypeId::of::<S>(), std::any::type_name::<S>(), order));
    }

    /// Type names of systems registered so far for `phase`, in registration
    /// order. Reads `pending`, which `build()` consumes — call before it.
    pub fn pending_names(&self, phase: Phase) -> Vec<&'static str> {
        self.pending
            .get(&phase)
            .map_or(Vec::new(), |v| v.iter().map(|&(_, _, name, _)| name).collect())
    }

    /// Set the app-wide fixed rate (steps per second). Must be called before
    /// build(). Default is 60.0.
    pub fn set_fixed_hz(&mut self, hz: f32) {
        self.fixed_dt = 1.0 / hz;
    }

    /// Topological sort of each phase. Called once after all systems are registered.
    /// First/Last systems are nodes in the same sort as Default/After/Before
    /// ones (implicit edges: every First precedes every non-First, every
    /// non-Last precedes every Last), so an After/Before naming a First/Last
    /// system resolves instead of silently evaporating. Panics on a cycle or
    /// on an After/Before target never registered in the phase.
    pub fn build(&mut self) {
        for (phase, items) in std::mem::take(&mut self.pending) {
            let n = items.len();

            let mut systems: Vec<Option<Box<dyn System>>> = Vec::with_capacity(n);
            let mut names:   Vec<&'static str>             = Vec::with_capacity(n);
            let mut orders:  Vec<SystemOrder>              = Vec::with_capacity(n);
            let mut index_of: HashMap<TypeId, usize>       = HashMap::with_capacity(n);

            for (i, (system, type_id, name, order)) in items.into_iter().enumerate() {
                systems.push(Some(system));
                names.push(name);
                orders.push(order);
                if index_of.insert(type_id, i).is_some() {
                    panic!(
                        "duplicate system type `{name}` in phase {phase:?}"
                    );
                }
            }

            let first_idx: HashSet<usize> = orders
                .iter()
                .enumerate()
                .filter_map(|(i, o)| matches!(o, SystemOrder::First).then_some(i))
                .collect();
            let last_idx: HashSet<usize> = orders
                .iter()
                .enumerate()
                .filter_map(|(i, o)| matches!(o, SystemOrder::Last).then_some(i))
                .collect();

            let mut adjacency: Vec<Vec<usize>> = vec![vec![]; n];

            for &fi in &first_idx {
                for j in 0..n {
                    if !first_idx.contains(&j) { adjacency[fi].push(j); }
                }
            }
            for &li in &last_idx {
                for (j, adj) in adjacency.iter_mut().enumerate() {
                    if j != li && !first_idx.contains(&j) && !last_idx.contains(&j) {
                        adj.push(li);
                    }
                }
            }

            for (i, order) in orders.iter().enumerate() {
                match order {
                    SystemOrder::After(target_id, target_name) => {
                        let j = *index_of.get(target_id).unwrap_or_else(|| {
                            panic!(
                                "unresolved ordering constraint in phase {phase:?}: `{}` is \
                                 After(`{target_name}`), which was never registered in this phase",
                                names[i],
                            )
                        });
                        adjacency[j].push(i);
                    }
                    SystemOrder::Before(target_id, target_name) => {
                        let j = *index_of.get(target_id).unwrap_or_else(|| {
                            panic!(
                                "unresolved ordering constraint in phase {phase:?}: `{}` is \
                                 Before(`{target_name}`), which was never registered in this phase",
                                names[i],
                            )
                        });
                        adjacency[i].push(j);
                    }
                    _ => {}
                }
            }

            let mut in_degree = vec![0usize; n];
            for neighbors in &adjacency {
                for &j in neighbors { in_degree[j] += 1; }
            }

            let mut queue: VecDeque<usize> = in_degree
                .iter()
                .enumerate()
                .filter_map(|(i, &d)| if d == 0 { Some(i) } else { None })
                .collect();

            let mut sorted: Vec<Box<dyn System>> = Vec::with_capacity(n);
            while let Some(i) = queue.pop_front() {
                sorted.push(systems[i].take().unwrap());
                for &j in &adjacency[i] {
                    in_degree[j] -= 1;
                    if in_degree[j] == 0 { queue.push_back(j); }
                }
            }

            if sorted.len() != n {
                panic!("Cycle detected in phase {:?} — check After/Before constraints", phase);
            }

            self.phases.insert(phase, PhaseEntry { systems: sorted, is_render: phase.is_render() });
        }
    }

    /// Advance the sim by one display frame.
    ///
    /// The whole frame runs off one fixed clock: `frame_delta` is accumulated
    /// once (capped at 8 steps to prevent spiral-of-death on lag spikes), then
    /// for each step every Fixed phase runs in declaration order before the
    /// next step — so cross-phase invariants (events live one tick, spawns
    /// visible to the same step's collision, one integration per collision
    /// pass) hold on every frame shape. Render phases fire once afterward,
    /// receiving `frame_delta`, with alpha from the single accumulator.
    pub fn run_tick(&mut self, world: &mut World, resources: &mut Resources, frame_delta: f32) {
        let fixed_dt = self.fixed_dt;
        self.accumulator = (self.accumulator + frame_delta).min(fixed_dt * 8.0);
        while self.accumulator >= fixed_dt {
            for entry in self.phases.values_mut() {
                if entry.is_render { continue; }
                for system in &mut entry.systems {
                    system.run(world, resources, fixed_dt);
                }
            }
            self.accumulator -= fixed_dt;
        }

        // Expose the sub-step fraction to render systems before they run.
        if let Some(alpha) = resources.get_mut::<InterpolationAlpha>() {
            alpha.0 = self.accumulator / fixed_dt;
        }
        for entry in self.phases.values_mut() {
            if !entry.is_render { continue; }
            for system in &mut entry.systems {
                system.run(world, resources, frame_delta);
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
    // Each const ID creates a distinct type, enabling multiple instances in the same phase.
    struct LogSystem<const ID: usize> {
        name:  &'static str,
        log:   Arc<Mutex<Vec<&'static str>>>,
        delta: Arc<Mutex<Vec<f32>>>,
    }

    impl<const ID: usize> System for LogSystem<ID> {
        fn run(&mut self, _world: &mut World, _resources: &mut Resources, delta: f32) {
            self.log.lock().unwrap().push(self.name);
            self.delta.lock().unwrap().push(delta);
        }
    }

    fn make_system<const ID: usize>(
        name:  &'static str,
        log:   Arc<Mutex<Vec<&'static str>>>,
        delta: Arc<Mutex<Vec<f32>>>,
    ) -> LogSystem<ID> {
        LogSystem { name, log, delta }
    }

    #[test]
    fn first_default_last_ordering_respected() {
        let log   = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let delta = Arc::new(Mutex::new(Vec::<f32>::new()));

        let mut sched = Scheduler::new();
        sched.add(make_system::<0>("last", log.clone(), delta.clone()), Phase::Update, SystemOrder::Last);
        sched.add(make_system::<1>("first", log.clone(), delta.clone()), Phase::Update, SystemOrder::First);
        sched.add(make_system::<2>("default", log.clone(), delta.clone()), Phase::Update, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        // 1/60 s frame — enough for one 60 Hz step
        sched.run_tick(&mut world, &mut resources, 1.0 / 60.0);

        assert_eq!(*log.lock().unwrap(), vec!["first", "default", "last"]);
    }

    #[test]
    fn after_before_constraints_respected() {
        let log   = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let delta = Arc::new(Mutex::new(Vec::<f32>::new()));

        struct A;
        impl System for A {
            fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {}
        }

        let mut sched = Scheduler::new();
        // "after_a" must run after A; registered first to stress the sort
        sched.add(make_system::<0>("after_a", log.clone(), delta.clone()), Phase::Update, SystemOrder::after::<A>());
        sched.add(A, Phase::Update, SystemOrder::Default);
        sched.add(make_system::<1>("before_a", log.clone(), delta.clone()), Phase::Update, SystemOrder::before::<A>());
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
    fn after_first_system_is_honored() {
        let log   = Arc::new(Mutex::new(Vec::new()));
        let delta = Arc::new(Mutex::new(Vec::new()));

        struct FirstSys;
        impl System for FirstSys {
            fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {}
        }

        let mut sched = Scheduler::new();
        // Registered before FirstSys to stress that the target resolves
        // regardless of declaration order.
        sched.add(make_system::<0>("after_first", log.clone(), delta.clone()),
                  Phase::Update, SystemOrder::after::<FirstSys>());
        sched.add(FirstSys, Phase::Update, SystemOrder::First);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        sched.run_tick(&mut world, &mut resources, 1.0 / 60.0);

        assert_eq!(*log.lock().unwrap(), vec!["after_first"]);
    }

    #[test]
    #[should_panic(expected = "Cycle detected")]
    fn before_first_system_panics() {
        struct FirstSys;
        impl System for FirstSys {
            fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {}
        }
        struct BeforeFirst;
        impl System for BeforeFirst {
            fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {}
        }

        let mut sched = Scheduler::new();
        sched.add(FirstSys, Phase::Update, SystemOrder::First);
        sched.add(BeforeFirst, Phase::Update, SystemOrder::before::<FirstSys>());
        sched.build();
    }

    #[test]
    #[should_panic(expected = "never registered in this phase")]
    fn unknown_ordering_target_panics() {
        struct Ghost;
        impl System for Ghost {
            fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {}
        }
        struct Real;
        impl System for Real {
            fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {}
        }

        let mut sched = Scheduler::new();
        sched.add(Real, Phase::Update, SystemOrder::after::<Ghost>());
        sched.build();
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
        sched.add(make_system::<0>("step", log.clone(), delta.clone()),
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
        sched.add(make_system::<0>("s", log.clone(), delta.clone()),
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
        sched.add(make_system::<0>("r", log.clone(), delta.clone()),
                  Phase::Render, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        sched.run_tick(&mut world, &mut resources, 0.050);

        assert_eq!(log.lock().unwrap().len(), 1);
        assert!((delta.lock().unwrap()[0] - 0.050).abs() < 1e-6);
    }

    #[test]
    fn multi_step_frame_interleaves_phases() {
        // Two fixed phases, one system each. A 3-step frame must run them
        // (Update -> Collision) three times interleaved, NOT drain Update
        // three times and then Collision three times.
        let log   = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let delta = Arc::new(Mutex::new(Vec::<f32>::new()));

        let mut sched = Scheduler::new();
        sched.add(make_system::<0>("update",    log.clone(), delta.clone()),
                  Phase::Update,    SystemOrder::Default);
        sched.add(make_system::<1>("collision", log.clone(), delta.clone()),
                  Phase::Collision, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        // 3.5 steps worth of time -> exactly 3 steps (avoids fp boundary).
        sched.run_tick(&mut world, &mut resources, 3.5 / 60.0);

        assert_eq!(
            *log.lock().unwrap(),
            vec!["update", "collision", "update", "collision", "update", "collision"],
        );
    }

    #[test]
    fn spiral_of_death_capped_at_8_steps() {
        let log   = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let delta = Arc::new(Mutex::new(Vec::<f32>::new()));

        let mut sched = Scheduler::new();
        sched.add(make_system::<0>("s", log.clone(), delta.clone()),
                  Phase::Update, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        // 1 second lag spike — without cap this would fire 60 steps
        sched.run_tick(&mut world, &mut resources, 1.0);
        assert_eq!(log.lock().unwrap().len(), 8);
    }

    #[test]
    fn set_fixed_hz_changes_app_wide_step_count() {
        let log   = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let delta = Arc::new(Mutex::new(Vec::<f32>::new()));

        let mut sched = Scheduler::new();
        sched.set_fixed_hz(30.0);
        sched.add(make_system::<0>("s", log.clone(), delta.clone()),
                  Phase::Update, SystemOrder::Default);
        sched.build();

        let mut world     = World::new();
        let mut resources = Resources::new();
        // 2.5 steps worth of time at 30 Hz → fires exactly 2 steps
        sched.run_tick(&mut world, &mut resources, 2.5 / 30.0);
        assert_eq!(log.lock().unwrap().len(), 2);

        let deltas = delta.lock().unwrap();
        let expected = 1.0_f32 / 30.0;
        for &d in deltas.iter() {
            assert!((d - expected).abs() < 1e-6, "expected {expected}, got {d}");
        }
    }

    #[test]
    #[should_panic(expected = "duplicate system type")]
    fn duplicate_system_type_panics() {
        struct DuplicateSys;
        impl System for DuplicateSys {
            fn run(&mut self, _: &mut World, _: &mut Resources, _: f32) {}
        }

        let mut sched = Scheduler::new();
        sched.add(DuplicateSys, Phase::Update, SystemOrder::Default);
        sched.add(DuplicateSys, Phase::Update, SystemOrder::Default);
        sched.build();
    }
}