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
use std::sync::{mpsc, Mutex};
use std::thread::JoinHandle;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS characters (
    id     INTEGER PRIMARY KEY,
    name   TEXT NOT NULL UNIQUE,
    zone   TEXT NOT NULL DEFAULT 'start',
    pos_x  REAL NOT NULL,
    pos_y  REAL NOT NULL,
    pos_z  REAL NOT NULL,
    health INTEGER NOT NULL
);
";

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterRecord {
    pub zone: String,
    pub pos: Vec3,
    pub health: i32,
}

enum DbRequest {
    LoadOrCreate {
        conn: ConnId,
        name: String,
        defaults: CharacterRecord,
        /// Where the result goes — the requesting zone's handle.
        reply: mpsc::Sender<DbLoaded>,
    },
    Save { name: String, zone: String, pos: Vec3, health: i32 },
}

/// A finished load: the character `name` plays on connection `conn`.
pub struct DbLoaded {
    pub conn: ConnId,
    pub name: String,
    pub record: CharacterRecord,
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
        let db = Connection::open(path)?;
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
        db.execute_batch(SCHEMA)?;
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
    /// Load `name`'s character, inserting `defaults` first if it doesn't
    /// exist. The result arrives via `poll` on THIS handle.
    pub fn load_or_create(&self, conn: ConnId, name: String, defaults: CharacterRecord) {
        let _ = self.tx.send(DbRequest::LoadOrCreate {
            conn,
            name,
            defaults,
            reply: self.reply_tx.clone(),
        });
    }

    /// Persist zone + position + health. Fire-and-forget; failures are
    /// logged by the worker.
    pub fn save(&self, name: String, zone: String, pos: Vec3, health: i32) {
        let _ = self.tx.send(DbRequest::Save { name, zone, pos, health });
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
                DbRequest::LoadOrCreate { conn, name, defaults, reply } => {
                    match load_or_create(&tx, &name, defaults) {
                        Ok(record) => loaded.push((reply, DbLoaded { conn, name, record })),
                        Err(e) => log::error!("db: load '{name}' failed: {e}"),
                    }
                }
                DbRequest::Save { name, zone, pos, health } => {
                    let result = tx.execute(
                        "UPDATE characters SET zone = ?1, pos_x = ?2, pos_y = ?3, pos_z = ?4, health = ?5 WHERE name = ?6",
                        rusqlite::params![zone, pos.x as f64, pos.y as f64, pos.z as f64, health, name],
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

fn load_or_create(db: &Connection, name: &str, defaults: CharacterRecord) -> rusqlite::Result<CharacterRecord> {
    // The schema default supplies zone = 'start' — fresh characters always
    // begin in the start zone regardless of where they logged in.
    db.execute(
        "INSERT OR IGNORE INTO characters (name, pos_x, pos_y, pos_z, health) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![name, defaults.pos.x as f64, defaults.pos.y as f64, defaults.pos.z as f64, defaults.health],
    )?;
    db.query_row(
        "SELECT zone, pos_x, pos_y, pos_z, health FROM characters WHERE name = ?1",
        [name],
        |row| {
            Ok(CharacterRecord {
                zone: row.get(0)?,
                pos: Vec3::new(row.get::<_, f64>(1)? as f32, row.get::<_, f64>(2)? as f32, row.get::<_, f64>(3)? as f32),
                health: row.get(4)?,
            })
        },
    )
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
        CharacterRecord { zone: "start".into(), pos: Vec3::ZERO, health: 100 }
    }

    fn wait_loaded(handle: &DbHandle) -> DbLoaded {
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
        let defaults = CharacterRecord { zone: "start".into(), pos: Vec3::new(3.0, 0.0, -2.0), health: 100 };
        handle.load_or_create(1, "alice".into(), defaults.clone());
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
            handle.load_or_create(1, "bob".into(), defaults());
            wait_loaded(&handle);
            handle.save("bob".into(), "east".into(), saved_pos, 40);
            // Drop (handle first, then worker) flushes the queued save.
        }
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let handle = worker.handle();
        handle.load_or_create(2, "bob".into(), defaults());
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
        handle.save("ghost".into(), "east".into(), Vec3::ONE, 1);
        handle.load_or_create(1, "ghost".into(), defaults());
        // The save hit no row; the later create uses defaults.
        assert_eq!(wait_loaded(&handle).record, defaults());
    }

    #[test]
    fn replies_route_to_the_requesting_handle() {
        let path = temp_db("route");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let a = worker.handle();
        let b = worker.handle();
        a.load_or_create(1, "ann".into(), defaults());
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
        a.load_or_create(1, "carl".into(), defaults());
        wait_loaded(&a);
        let moved = Vec3::new(-16.0, 0.0, 0.0);
        a.save("carl".into(), "east".into(), moved, 80);
        b.load_or_create(7, "carl".into(), defaults());
        let loaded = wait_loaded(&b);
        assert_eq!(loaded.record.zone, "east");
        assert_eq!(loaded.record.pos, moved);
        assert_eq!(loaded.record.health, 80);
    }

    /// Regression test for finding 13 of the networking audit: `db.rs` had
    /// no `journal_mode` PRAGMA at all, so SQLite defaulted to a rollback
    /// journal (fsync per commit, writers block readers). `journal_mode` is
    /// recorded in the database file header, not per-connection, so a fresh,
    /// independent connection opened after the worker started must also see
    /// "wal".
    #[test]
    fn spawn_enables_wal_journal_mode() {
        let path = temp_db("wal");
        let worker = DbWorker::spawn(path.to_str().unwrap()).unwrap();
        let check = Connection::open(&path).unwrap();
        let mode: String = check.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        drop(worker);
    }

    /// Regression test for finding 13's batched-transaction worker loop: a
    /// wave of heterogeneous requests (three saves for existing characters
    /// plus a load-or-create for a new one) enqueued back-to-back, before
    /// the worker thread can drain them one at a time, must all land — not
    /// just the first request the old per-request-autocommit loop happened
    /// to see first.
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
            handle.load_or_create(0, n.into(), defaults());
            wait_loaded(&handle);
        }

        // Fire the burst without waiting between sends.
        handle.save("ann".into(), "east".into(), Vec3::new(1.0, 0.0, 0.0), 10);
        handle.save("bob".into(), "east".into(), Vec3::new(2.0, 0.0, 0.0), 20);
        handle.save("cleo".into(), "east".into(), Vec3::new(3.0, 0.0, 0.0), 30);
        handle.load_or_create(9, "dana".into(), defaults());
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
            handle.load_or_create(1, name.into(), defaults());
            let loaded = wait_loaded(&handle);
            assert_eq!(loaded.record.zone, "east", "{name} zone not saved");
            assert_eq!(loaded.record.pos, pos, "{name} pos not saved");
            assert_eq!(loaded.record.health, health, "{name} health not saved");
        }
    }
}
