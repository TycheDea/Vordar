use super::*;

/// NetClientState with no live connection: the net thread's connect
/// attempt fails in the background while the benched paths only read
/// and write the state fields.
pub fn state_for_bench(own_id: Option<u32>, predict: bool) -> NetClientState {
    let server_addr = "127.0.0.1:9".parse().unwrap();
    let mut state = NetClientState::new(
        Some(NetClient::connect(server_addr, PROTOCOL_VERSION).expect("bench NetClient")),
        server_addr,
        "bench".into(),
        [0u8; 32],
        predict,
        Duration::ZERO,
    );
    state.own_id = own_id;
    state
}

/// server-id → local-entity mapping (the enters path builds this normally).
pub fn map_entity(state: &mut NetClientState, id: u32, entity: Entity) {
    state.entities.insert(id, entity);
}

/// Seeds the client's cached prefab name table directly — bypasses the
/// `ServerMsg::PrefabTable` wire round trip so benches can build `enters`
/// with `u16` refs against a known table.
pub fn set_prefab_table(state: &mut NetClientState, names: Vec<String>) {
    state.prefab_names = names;
}

pub fn push_pending(state: &mut NetClientState, seq: u32, dir: Vec2, dt: f32) {
    state.pending.push_back(PendingIntent { seq, dir, dt, leap: None });
}

pub fn apply_aoi_delta(
    world: &mut World,
    resources: &mut Resources,
    tick: u64,
    enters: Vec<EntityState>,
    leaves: Vec<u32>,
) {
    super::apply::apply_aoi_delta(world, resources, tick, enters, leaves);
}

pub fn apply_states(
    world: &mut World,
    resources: &mut Resources,
    tick: u64,
    last_processed_seq: u32,
    states: Vec<EntityPos>,
) {
    super::apply::apply_states(world, resources, tick, last_processed_seq, states);
}

pub fn reconcile_own(
    world: &mut World,
    resources: &mut Resources,
    entity: Entity,
    server_pos: Vec3,
    last_processed_seq: u32,
) {
    super::prediction::reconcile_own(world, resources, entity, server_pos, last_processed_seq);
}
