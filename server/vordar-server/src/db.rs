// db — character persistence on a dedicated worker thread.
//
// The sim never blocks on SQLite: requests go over an mpsc channel to a
// worker thread that owns the rusqlite Connection (Send, not Sync), and
// load results come back on per-handle reply channels polled from the Input
// phase — the same thread+channel shape engine-net uses. ONE worker serves
// every zone App through cloned `DbHandle`s: the single FIFO request channel
// means a save enqueued by zone A (disconnect or portal transfer) lands
// before a relogin-load enqueued by zone B afterwards, so an instant
// reconnect always sees its own save — across zones, by construction.
// Replies carry their own sender, so ConnIds (per-NetServer counters that
// collide across zones) never need disambiguating.
//
// All SQL lives in this module. Auth is deferred (development runs as a
// single-player pack): characters are keyed by plain name; an accounts
// table later is `ALTER TABLE characters ADD COLUMN account_id` — no rewrite.

use engine_net::ConnId;
use glam::Vec3;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{mpsc, Mutex};
use std::thread::JoinHandle;
use vordar_protocol::AccountToken;

// Append-only schema history: entry i brings a database from user_version i
// to i+1. Entry 0 is the original baseline schema — deliberately kept as
// `CREATE TABLE IF NOT EXISTS` so every pre-versioning database in the wild
// (user_version == 0, table already present) adopts the ladder losslessly;
// later entries use plain DDL. Never edit an already-shipped entry — append
// a new one instead, the same discipline as any other migration ladder.
const MIGRATIONS: &[&str] = &["
CREATE TABLE IF NOT EXISTS characters (
    id     INTEGER PRIMARY KEY,
    name   TEXT NOT NULL UNIQUE,
    zone   TEXT NOT NULL DEFAULT 'start',
    pos_x  REAL NOT NULL,
    pos_y  REAL NOT NULL,
    pos_z  REAL NOT NULL,
    health INTEGER NOT NULL
);
", "
ALTER TABLE characters ADD COLUMN cooldowns TEXT NOT NULL DEFAULT '{}';
", "
CREATE TABLE accounts (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, token_hash BLOB);
ALTER TABLE characters ADD COLUMN account_id INTEGER REFERENCES accounts(id);
INSERT INTO accounts (name) SELECT name FROM characters;
UPDATE characters SET account_id = (SELECT id FROM accounts WHERE accounts.name = characters.name);
"];

/// Bring `db` up to the latest schema version. Each pending migration runs in
/// its own transaction with the `user_version` bump committed atomically
/// alongside its DDL (`user_version` is header state, so it commits with the
/// transaction). A `user_version` beyond the known ladder means the file was
/// written by a newer build; refuse it rather than run against an unknown
/// schema.
fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 0 || version as usize > MIGRATIONS.len() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(format!(
                "database schema version {version} is newer than this build supports (known migrations: 0..={})",
                MIGRATIONS.len()
            )),
        ));
    }
    for (i, ddl) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = db.transaction()?;
        tx.execute_batch(ddl)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterRecord {
    pub zone: String,
    pub pos: Vec3,
    pub health: i32,
    /// Cooldown remainders (skill id → remaining microseconds) as of the
    /// moment this record was saved — never absolute `ready_at` stamps,
    /// which only mean something against the server clock that produced
    /// them. Only skills still cooling down are present.
    pub cooldowns: HashMap<String, u64>,
}

enum DbRequest {
    /// Verify `token` against the `accounts` row for `name` before loading or
    /// creating the character — see `login()`.
    Login {
        conn: ConnId,
        name: String,
        token: AccountToken,
        defaults: CharacterRecord,
        reply: mpsc::Sender<DbLoaded>,
    },
    Save { name: String, record: CharacterRecord },
}

/// The result of a worker-side login attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum DbLoginOutcome {
    /// Credentials verified (or the name was claimed just now) — the
    /// character's record, loaded or freshly created.
    Granted(CharacterRecord),
    /// The account exists and is claimed by a different token. No character
    /// row was touched.
    BadToken,
}

/// A finished load: the character `name` plays on connection `conn`.
pub struct DbLoaded {
    pub conn: ConnId,
    pub name: String,
    pub outcome: DbLoginOutcome,
}

/// Owns the worker thread. Mint one `DbHandle` per zone App via `handle()`.
/// Drop joins the worker (draining queued saves), so the owner must outlive
/// every handle — handles keep the request channel open.
pub struct DbWorker {
    tx: Option<mpsc::Sender<DbRequest>>,
    handle: Option<JoinHandle<()>>,
}

/// A zone App's connection to the shared worker: cloned request sender plus
/// a private reply channel, so loads come back only to the zone that asked.
pub struct DbHandle {
    tx: mpsc::Sender<DbRequest>,
    reply_tx: mpsc::Sender<DbLoaded>,
    /// Mutex only to make the handle Sync for the resource map — the zone's
    /// sim thread is the sole reader, so it never contends.
    reply_rx: Mutex<mpsc::Receiver<DbLoaded>>,
}

impl DbWorker {
    /// Open the database (creating the schema) and start the worker thread.
    /// Opening happens on the calling thread so startup failure is synchronous.
    pub fn spawn(path: &str) -> rusqlite::Result<DbWorker> {
        let mut db = Connection::open(path)?;
        // WAL lets a save's writer transaction run without blocking a
        // concurrent load's reader; NORMAL syncs at WAL checkpoints instead
        // of every commit; busy_timeout absorbs a checkpoint briefly holding
        // the file instead of returning SQLITE_BUSY. `:memory:` databases
        // (tests, throwaway runs) have no WAL file, so SQLite silently keeps
        // the in-memory journal — harmless.
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        migrate(&mut db)?;
        let (req_tx, req_rx) = mpsc::channel::<DbRequest>();
        let handle = std::thread::Builder::new()
            .name("vordar-db".into())
            .spawn(move || worker(db, req_rx))
            .expect("spawn db worker thread");
        Ok(DbWorker { tx: Some(req_tx), handle: Some(handle) })
    }

    pub fn handle(&self) -> DbHandle {
        let (reply_tx, reply_rx) = mpsc::channel();
        DbHandle {
            tx: self.tx.as_ref().unwrap().clone(),
            reply_tx,
            reply_rx: Mutex::new(reply_rx),
        }
    }
}

impl DbHandle {
    /// Mint a sibling handle for the same worker: shares `self`'s request
    /// channel (saves and loads still flow to, and are ordered by, the one
    /// `DbWorker`) but owns a fresh, private reply channel. Deliberately not
    /// `impl Clone` — a clone that silently shared the reply channel would be
    /// a semantic trap: a load issued through the fork must reply only to the
    /// fork, never to `self`, so a supervisor rebuilding a panicked zone's
    /// App can mint a new handle without in-flight replies addressed to the
    /// dead App's reply channel leaking into the rebuilt one.
    pub fn fork(&self) -> DbHandle {
        let (reply_tx, reply_rx) = mpsc::channel();
        DbHandle {
            tx: self.tx.clone(),
            reply_tx,
            reply_rx: Mutex::new(reply_rx),
        }
    }

    /// Verify `token` against the `accounts` row for `name` (creating and
    /// claiming it on first use, claiming a legacy unclaimed row, denying a
    /// mismatch) and, on success, load or create the character. The result
    /// arrives via `poll` on THIS handle.
    pub fn login(&self, conn: ConnId, name: String, token: AccountToken, defaults: CharacterRecord) {
        let _ = self.tx.send(DbRequest::Login {
            conn,
            name,
            token,
            defaults,
            reply: self.reply_tx.clone(),
        });
    }

    /// Persist a full character record (zone, position, health, and cooldown
    /// remainders). Fire-and-forget; failures are logged by the worker.
    pub fn save(&self, name: String, record: CharacterRecord) {
        let _ = self.tx.send(DbRequest::Save { name, record });
    }

    /// Completed loads since the last poll.
    pub fn poll(&self) -> Vec<DbLoaded> {
        self.reply_rx.lock().unwrap().try_iter().collect()
    }
}

impl Drop for DbWorker {
    // Dropping the sender closes the channel once every handle is gone; the
    // worker drains whatever is queued before exiting, so pending saves are
    // flushed on graceful shutdown.
    fn drop(&mut self) {
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker(mut db: Connection, rx: mpsc::Receiver<DbRequest>) {
    loop {
        // Block for the first request of a wave, then drain whatever else is
        // already queued without blocking — a burst (staggered autosaves,
        // several logins) becomes one transaction instead of one autocommit
        // statement — and, in the old rollback-journal mode, one fsync — per
        // request.
        let Ok(first) = rx.recv() else { return };
        let mut batch = vec![first];
        batch.extend(rx.try_iter());

        let tx = match db.transaction() {
            Ok(tx) => tx,
            Err(e) => {
                log::error!("db: failed to start transaction: {e}");
                continue;
            }
        };

        // Replies are collected and sent only after the batch commits, so a
        // load result is never handed out before it is durable.
        let mut loaded: Vec<(mpsc::Sender<DbLoaded>, DbLoaded)> = Vec::new();
        for req in batch {
            match req {
                DbRequest::Login { conn, name, token, defaults, reply } => {
                    match login(&tx, &name, &token, defaults) {
                        Ok(outcome) => loaded.push((reply, DbLoaded { conn, name, outcome })),
                        Err(e) => log::error!("db: login '{name}' failed: {e}"),
                    }
                }
                DbRequest::Save { name, record } => {
                    let cooldowns_text = ron::to_string(&record.cooldowns).unwrap_or_else(|e| {
                        log::error!("db: failed to encode cooldowns for '{name}': {e}");
                        "{}".into()
                    });
                    let result = tx.execute(
                        "UPDATE characters SET zone = ?1, pos_x = ?2, pos_y = ?3, pos_z = ?4, health = ?5, cooldowns = ?6 WHERE name = ?7",
                        rusqlite::params![
                            record.zone,
                            record.pos.x as f64,
                            record.pos.y as f64,
                            record.pos.z as f64,
                            record.health,
                            cooldowns_text,
                            name
                        ],
                    );
                    if let Err(e) = result {
                        log::error!("db: save '{name}' failed: {e}");
                    }
                }
            }
        }

        if let Err(e) = tx.commit() {
            log::error!("db: transaction commit failed: {e}");
            continue;
        }
        for (reply, msg) in loaded {
            let _ = reply.send(msg);
        }
    }
}

fn load_or_create(
    db: &Connection,
    name: &str,
    defaults: CharacterRecord,
    account_id: i64,
) -> rusqlite::Result<CharacterRecord> {
    // The schema default supplies zone = 'start' — fresh characters always
    // begin in the start zone regardless of where they logged in.
    // `account_id` is set on a genuine INSERT; an existing row keeps
    // whatever it already had (`INSERT OR IGNORE` no-ops on conflict).
    db.execute(
        "INSERT OR IGNORE INTO characters (name, pos_x, pos_y, pos_z, health, account_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![name, defaults.pos.x as f64, defaults.pos.y as f64, defaults.pos.z as f64, defaults.health, account_id],
    )?;
    db.query_row(
        "SELECT zone, pos_x, pos_y, pos_z, health, cooldowns FROM characters WHERE name = ?1",
        [name],
        |row| {
            let cooldowns_text: String = row.get(5)?;
            let cooldowns = ron::from_str(&cooldowns_text).unwrap_or_else(|e| {
                log::error!("db: failed to parse cooldowns for '{name}': {e}");
                HashMap::new()
            });
            Ok(CharacterRecord {
                zone: row.get(0)?,
                pos: Vec3::new(row.get::<_, f64>(1)? as f32, row.get::<_, f64>(2)? as f32, row.get::<_, f64>(3)? as f32),
                health: row.get(4)?,
                cooldowns,
            })
        },
    )
}

/// Verify `token` against the `accounts` row for `name`: missing → create it
/// claimed with `sha256(token)`; present but unclaimed (`token_hash` NULL —
/// a legacy row, or one the migration backfilled) → claim it; present and
/// claimed → grant only on a matching hash, denying (no character touched)
/// otherwise. On success, load or create the character linked to the
/// account, self-healing `account_id` for any row left without one (a
/// defensive no-op once every login path sets it on INSERT).
fn login(db: &Connection, name: &str, token: &AccountToken, defaults: CharacterRecord) -> rusqlite::Result<DbLoginOutcome> {
    let hash = Sha256::digest(token).to_vec();

    let existing: Option<(i64, Option<Vec<u8>>)> = match db.query_row(
        "SELECT id, token_hash FROM accounts WHERE name = ?1",
        [name],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(row) => Some(row),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e),
    };

    let account_id = match existing {
        None => {
            db.execute(
                "INSERT INTO accounts (name, token_hash) VALUES (?1, ?2)",
                rusqlite::params![name, hash],
            )?;
            db.last_insert_rowid()
        }
        Some((id, None)) => {
            db.execute("UPDATE accounts SET token_hash = ?1 WHERE id = ?2", rusqlite::params![hash, id])?;
            id
        }
        Some((id, Some(claimed))) => {
            if claimed != hash {
                return Ok(DbLoginOutcome::BadToken);
            }
            id
        }
    };

    let record = load_or_create(db, name, defaults, account_id)?;
    db.execute(
        "UPDATE characters SET account_id = ?1 WHERE name = ?2 AND account_id IS NULL",
        rusqlite::params![account_id, name],
    )?;
    Ok(DbLoginOutcome::Granted(record))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("vordar-db-test-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn defaults() -> CharacterRecord {
        CharacterRecord { zone: "start".into(), pos: Vec3::ZERO, health: 100, cooldowns: HashMap::new() }
    }

    /// The old always-a-record shape, for the many pre-existing tests that
    /// only care about the granted path (every `login()` call they make
    /// either claims a fresh name or repeats the same token, so the outcome
    /// is always `Granted`). Panics on `BadToken` — that outcome is only
    /// exercised by the token-mismatch tests below, via `wait_login`.
    struct Loaded {
        conn: ConnId,
        name: String,
        record: CharacterRecord,
    }

    fn wait_loaded(handle: &DbHandle) -> Loaded {
        let loaded = wait_login(handle);
        match loaded.outcome {
            DbLoginOutcome::Granted(record) => Loaded { conn: loaded.conn, name: loaded.name, record },
            DbLoginOutcome::BadToken => panic!("wait_loaded: unexpected BadToken from '{}'", loaded.name),
        }
    }

    /// Outcome-aware wait for `DbHandle::login` tests, which need to see
    /// `BadToken` rather than have it treated as a bug.
    fn wait_login(handle: &DbHandle) -> DbLoaded {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(loaded) = handle.poll().pop() {
                return loaded;
            }
            assert!(std::time::Instant::now() < deadline, "db load timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn fresh_name_gets_defaults() {
        let path = temp_db("fresh");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        let defaults = CharacterRecord { zone: "start".into(), pos: Vec3::new(3.0, 0.0, -2.0), health: 100, cooldowns: HashMap::new() };
        handle.login(1, "alice".into(), [0u8; 32], defaults.clone());
        let loaded = wait_loaded(&handle);
        assert_eq!(loaded.conn, 1);
        assert_eq!(loaded.name, "alice");
        assert_eq!(loaded.record, defaults);
    }

    #[test]
    fn save_then_reload_roundtrips_across_reopen() {
        let path = temp_db("reload");
        let saved_pos = Vec3::new(7.5, 0.0, -4.25);
        {
            let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
            let handle = worker.handle();
            handle.login(1, "bob".into(), [0u8; 32], defaults());
            wait_loaded(&handle);
            handle.save("bob".into(), CharacterRecord { zone: "east".into(), pos: saved_pos, health: 40, cooldowns: HashMap::new() });
            // Drop (handle first, then worker) flushes the queued save.
        }
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        handle.login(2, "bob".into(), [0u8; 32], defaults());
        let loaded = wait_loaded(&handle);
        assert_eq!(loaded.record.zone, "east");
        assert_eq!(loaded.record.pos, saved_pos);
        assert_eq!(loaded.record.health, 40);
    }

    #[test]
    fn save_for_unknown_name_is_harmless() {
        let path = temp_db("unknown");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        handle.save("ghost".into(), CharacterRecord { zone: "east".into(), pos: Vec3::ONE, health: 1, cooldowns: HashMap::new() });
        handle.login(1, "ghost".into(), [0u8; 32], defaults());
        // The save hit no row; the later create uses defaults.
        assert_eq!(wait_loaded(&handle).record, defaults());
    }

    #[test]
    fn replies_route_to_the_requesting_handle() {
        let path = temp_db("route");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let a = worker.handle();
        let b = worker.handle();
        a.login(1, "ann".into(), [0u8; 32], defaults());
        let loaded = wait_loaded(&a);
        assert_eq!(loaded.name, "ann");
        // b never sees a's load, even after it has completed.
        assert!(b.poll().is_empty());
    }

    #[test]
    fn save_via_one_handle_visible_to_load_via_another() {
        // The cross-zone transfer ordering: zone A saves, zone B loads later
        // through the same FIFO request channel — B must see A's save.
        let path = temp_db("fifo");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let a = worker.handle();
        let b = worker.handle();
        a.login(1, "carl".into(), [0u8; 32], defaults());
        wait_loaded(&a);
        let moved = Vec3::new(-16.0, 0.0, 0.0);
        a.save("carl".into(), CharacterRecord { zone: "east".into(), pos: moved, health: 80, cooldowns: HashMap::new() });
        b.login(7, "carl".into(), [0u8; 32], defaults());
        let loaded = wait_loaded(&b);
        assert_eq!(loaded.record.zone, "east");
        assert_eq!(loaded.record.pos, moved);
        assert_eq!(loaded.record.health, 80);
    }

    /// `journal_mode` must be WAL, not SQLite's rollback-journal default
    /// (fsync per commit, writers block readers). It is recorded in the
    /// database file header, not per-connection, so a fresh, independent
    /// connection opened after the worker started must also see "wal".
    #[test]
    fn spawn_enables_wal_journal_mode() {
        let path = temp_db("wal");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let check = Connection::open(&path).unwrap();
        let mode: String = check.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        drop(worker);
    }

    /// A fresh database has no schema history at all — `spawn` must stamp
    /// `user_version` to the latest migration it applied, not leave it at
    /// SQLite's default of 0. Read through an independent connection since
    /// `user_version` is header state, not per-connection.
    #[test]
    fn fresh_db_stamps_user_version_to_latest_migration() {
        let path = temp_db("migrate-fresh");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let check = Connection::open(&path).unwrap();
        let version: i64 = check.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64, "fresh db should be stamped to the latest known migration");
        drop(worker);
    }

    /// A legacy database predating the migration ladder looks like this —
    /// the characters table present, data in it, no `user_version` stamp
    /// (defaults to 0). Opening it through `DbWorker` must adopt the ladder
    /// losslessly (the row's data survives) and stamp the version, not leave
    /// the file's history ambiguous forever.
    #[test]
    fn legacy_db_without_version_stamp_adopts_the_ladder() {
        let path = temp_db("migrate-legacy");
        {
            let raw = Connection::open(&path).unwrap();
            raw.execute_batch(
                "CREATE TABLE characters (
                    id     INTEGER PRIMARY KEY,
                    name   TEXT NOT NULL UNIQUE,
                    zone   TEXT NOT NULL DEFAULT 'start',
                    pos_x  REAL NOT NULL,
                    pos_y  REAL NOT NULL,
                    pos_z  REAL NOT NULL,
                    health INTEGER NOT NULL
                );",
            )
            .unwrap();
            raw.execute(
                "INSERT INTO characters (name, zone, pos_x, pos_y, pos_z, health) VALUES ('legacy', 'east', 1.0, 2.0, 3.0, 55)",
                [],
            )
            .unwrap();
        }

        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        handle.login(1, "legacy".into(), [0u8; 32], defaults());
        let loaded = wait_loaded(&handle);
        assert_eq!(loaded.record.zone, "east");
        assert_eq!(loaded.record.pos, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(loaded.record.health, 55);

        let check = Connection::open(&path).unwrap();
        let version: i64 = check.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64, "legacy adoption should stamp the version too");
        // handle is dropped before worker (reverse declaration order) so its
        // sender clone is gone before the worker thread is joined — the same
        // ordering `save_then_reload_roundtrips_across_reopen` relies on.
    }

    /// A database stamped with a `user_version` beyond this build's known
    /// migration ladder was written by a newer build. Running against it
    /// blind risks corrupting an unknown schema; `spawn` must refuse it.
    #[test]
    fn newer_schema_version_is_refused_not_silently_run() {
        let path = temp_db("migrate-future");
        {
            let raw = Connection::open(&path).unwrap();
            raw.pragma_update(None, "user_version", 99i64).unwrap();
        }
        let result = DbWorker::spawn(path.to_str().unwrap());
        assert!(result.is_err(), "a database from a newer build must be refused, not silently run");
    }

    /// A supervisor rebuilding a panicked zone's App needs a fresh `DbHandle`
    /// without reaching back to the main-thread `DbWorker`. `fork()` must
    /// share the worker's request channel (same end-to-end persistence) but
    /// keep its own reply channel private — a load issued through the fork
    /// must never be visible on the handle it was forked from.
    #[test]
    fn fork_routes_replies_only_to_the_fork() {
        let path = temp_db("fork");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let h1 = worker.handle();
        let h2 = h1.fork();

        h2.login(1, "forked".into(), [0u8; 32], defaults());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let loaded = loop {
            assert!(h1.poll().is_empty(), "fork's reply leaked into the original handle");
            if let Some(loaded) = h2.poll().pop() {
                break loaded;
            }
            assert!(std::time::Instant::now() < deadline, "fork load timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(loaded.name, "forked");
        match loaded.outcome {
            DbLoginOutcome::Granted(record) => assert_eq!(record, defaults()),
            DbLoginOutcome::BadToken => panic!("a fresh name's first login must always grant"),
        }
        assert!(h1.poll().is_empty(), "fork's reply leaked into the original handle after completion");

        let moved = Vec3::new(5.0, 0.0, -1.0);
        h2.save("forked".into(), CharacterRecord { zone: "east".into(), pos: moved, health: 77, cooldowns: HashMap::new() });
        drop(h1);
        drop(h2);
        drop(worker);

        let check = Connection::open(&path).unwrap();
        let (zone, pos_x, pos_y, pos_z, health): (String, f64, f64, f64, i32) = check
            .query_row(
                "SELECT zone, pos_x, pos_y, pos_z, health FROM characters WHERE name = 'forked'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(zone, "east");
        assert_eq!(pos_x, moved.x as f64);
        assert_eq!(pos_y, moved.y as f64);
        assert_eq!(pos_z, moved.z as f64);
        assert_eq!(health, 77);
    }

    /// A wave of heterogeneous requests (three saves for existing characters
    /// plus a load-or-create for a new one) enqueued back-to-back, before
    /// the worker thread can drain them one at a time, must all land in a
    /// single batched transaction — not just the first request seen.
    #[test]
    fn a_burst_of_saves_and_loads_all_land_from_one_wave() {
        let path = temp_db("batch");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        for n in ["ann", "bob", "cleo"] {
            // Seeded one at a time: `poll`/`wait_loaded` return whatever
            // replies have arrived as a batch, and `wait_loaded` only keeps
            // the last — waiting after each send keeps this seeding loop
            // (not the fix under test) from losing replies.
            handle.login(0, n.into(), [0u8; 32], defaults());
            wait_loaded(&handle);
        }

        // Fire the burst without waiting between sends.
        handle.save("ann".into(), CharacterRecord { zone: "east".into(), pos: Vec3::new(1.0, 0.0, 0.0), health: 10, cooldowns: HashMap::new() });
        handle.save("bob".into(), CharacterRecord { zone: "east".into(), pos: Vec3::new(2.0, 0.0, 0.0), health: 20, cooldowns: HashMap::new() });
        handle.save("cleo".into(), CharacterRecord { zone: "east".into(), pos: Vec3::new(3.0, 0.0, 0.0), health: 30, cooldowns: HashMap::new() });
        handle.login(9, "dana".into(), [0u8; 32], defaults());
        let dana = wait_loaded(&handle);
        assert_eq!(dana.name, "dana");
        assert_eq!(dana.record, defaults());

        drop(handle);
        drop(worker);
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        for (name, pos, health) in [
            ("ann", Vec3::new(1.0, 0.0, 0.0), 10),
            ("bob", Vec3::new(2.0, 0.0, 0.0), 20),
            ("cleo", Vec3::new(3.0, 0.0, 0.0), 30),
        ] {
            handle.login(1, name.into(), [0u8; 32], defaults());
            let loaded = wait_loaded(&handle);
            assert_eq!(loaded.record.zone, "east", "{name} zone not saved");
            assert_eq!(loaded.record.pos, pos, "{name} pos not saved");
            assert_eq!(loaded.record.health, health, "{name} health not saved");
        }
    }

    /// Cooldowns are persisted as remainders in the `cooldowns` column
    /// (RON-encoded `HashMap<String, u64>`). A save carrying a non-empty
    /// cooldowns map must survive a full close/reopen of the database file,
    /// the same round-trip `save_then_reload_roundtrips_across_reopen`
    /// proves for position/health.
    #[test]
    fn cooldowns_persist_across_reopen() {
        let path = temp_db("cooldowns");
        let mut cooldowns = HashMap::new();
        cooldowns.insert("onslaught".to_string(), 4_500_000u64);
        cooldowns.insert("rend".to_string(), 200_000u64);
        {
            let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
            let handle = worker.handle();
            handle.login(1, "dara".into(), [0u8; 32], defaults());
            wait_loaded(&handle);
            handle.save(
                "dara".into(),
                CharacterRecord {
                    zone: "start".into(),
                    pos: Vec3::new(2.0, 0.0, 3.0),
                    health: 80,
                    cooldowns: cooldowns.clone(),
                },
            );
            // Drop (handle first, then worker) flushes the queued save.
        }
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        handle.login(2, "dara".into(), [0u8; 32], defaults());
        let loaded = wait_loaded(&handle);
        assert_eq!(loaded.record.cooldowns, cooldowns, "cooldown remainders must survive a reopen");
    }

    /// A fresh name's first `login` has nothing to compare against, so it
    /// claims the account (stores `sha256(token)`) and grants — same as
    /// `load_or_create` would, just through the verified path.
    #[test]
    fn fresh_name_login_claims_the_account_and_grants() {
        let path = temp_db("login-fresh");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        handle.login(1, "erin".into(), [7u8; 32], defaults());
        let loaded = wait_login(&handle);
        assert_eq!(loaded.conn, 1);
        assert_eq!(loaded.name, "erin");
        match loaded.outcome {
            DbLoginOutcome::Granted(record) => assert_eq!(record, defaults()),
            DbLoginOutcome::BadToken => panic!("a fresh name must claim the account, not be denied"),
        }
    }

    /// A second login presenting the SAME token the account was claimed with
    /// must keep granting — the account is claimed, not locked to one login.
    #[test]
    fn same_token_relogin_is_granted() {
        let path = temp_db("login-same-token");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        let token = [3u8; 32];
        handle.login(1, "finn".into(), token, defaults());
        wait_login(&handle);
        handle.login(2, "finn".into(), token, defaults());
        let loaded = wait_login(&handle);
        assert_eq!(loaded.conn, 2);
        match loaded.outcome {
            DbLoginOutcome::Granted(_) => {}
            DbLoginOutcome::BadToken => panic!("the same token must still be granted on relogin"),
        }
    }

    /// A DIFFERENT token than the one that claimed the name must be denied,
    /// and the character row must be left exactly as it was — a mismatch
    /// never touches character state.
    #[test]
    fn mismatched_token_is_denied_and_character_untouched() {
        let path = temp_db("login-mismatch");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        handle.login(1, "gwen".into(), [1u8; 32], defaults());
        wait_login(&handle);
        handle.save(
            "gwen".into(),
            CharacterRecord { zone: "east".into(), pos: Vec3::new(5.0, 0.0, 0.0), health: 42, cooldowns: HashMap::new() },
        );
        handle.login(2, "gwen".into(), [2u8; 32], defaults());
        let loaded = wait_login(&handle);
        assert_eq!(loaded.conn, 2);
        match loaded.outcome {
            DbLoginOutcome::BadToken => {}
            DbLoginOutcome::Granted(_) => panic!("a mismatched token must be denied"),
        }
        let check = Connection::open(&path).unwrap();
        let (zone, health): (String, i32) = check
            .query_row("SELECT zone, health FROM characters WHERE name = 'gwen'", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!(zone, "east", "a denied login must not touch the character row");
        assert_eq!(health, 42, "a denied login must not touch the character row");
    }

    /// A database predating the accounts feature (characters table only, no
    /// accounts) migrated on `spawn` must land exactly one unclaimed account
    /// per character, linked via `account_id`, at `user_version == 3` — and
    /// that legacy character's first `login` claims the account for the
    /// presented token.
    #[test]
    fn legacy_characters_get_unclaimed_linked_accounts_and_first_login_claims() {
        let path = temp_db("login-legacy");
        {
            let raw = Connection::open(&path).unwrap();
            raw.execute_batch(
                "CREATE TABLE characters (
                    id     INTEGER PRIMARY KEY,
                    name   TEXT NOT NULL UNIQUE,
                    zone   TEXT NOT NULL DEFAULT 'start',
                    pos_x  REAL NOT NULL,
                    pos_y  REAL NOT NULL,
                    pos_z  REAL NOT NULL,
                    health INTEGER NOT NULL
                );",
            )
            .unwrap();
            raw.execute(
                "INSERT INTO characters (name, zone, pos_x, pos_y, pos_z, health) VALUES ('holt', 'east', 1.0, 2.0, 3.0, 55)",
                [],
            )
            .unwrap();
        }

        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        {
            let check = Connection::open(&path).unwrap();
            let version: i64 = check.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
            assert_eq!(version, MIGRATIONS.len() as i64);

            let account_count: i64 =
                check.query_row("SELECT COUNT(*) FROM accounts WHERE name = 'holt'", [], |row| row.get(0)).unwrap();
            assert_eq!(account_count, 1, "migration must create exactly one account per character");

            let token_hash: Option<Vec<u8>> =
                check.query_row("SELECT token_hash FROM accounts WHERE name = 'holt'", [], |row| row.get(0)).unwrap();
            assert!(token_hash.is_none(), "a backfilled account starts unclaimed");

            let linked: i64 = check
                .query_row(
                    "SELECT COUNT(*) FROM characters c JOIN accounts a ON c.account_id = a.id WHERE c.name = 'holt'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(linked, 1, "the legacy character must be linked to its account");
        }

        let handle = worker.handle();
        handle.login(1, "holt".into(), [9u8; 32], defaults());
        let loaded = wait_login(&handle);
        match loaded.outcome {
            DbLoginOutcome::Granted(record) => {
                assert_eq!(record.zone, "east");
                assert_eq!(record.pos, Vec3::new(1.0, 2.0, 3.0));
                assert_eq!(record.health, 55);
            }
            DbLoginOutcome::BadToken => panic!("a legacy unclaimed account's first login must claim it"),
        }

        let check = Connection::open(&path).unwrap();
        let claimed: Option<Vec<u8>> =
            check.query_row("SELECT token_hash FROM accounts WHERE name = 'holt'", [], |row| row.get(0)).unwrap();
        assert!(claimed.is_some(), "the account must be claimed after its first login");
    }
}
