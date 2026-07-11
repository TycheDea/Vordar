//! Regression test for the MAX_FRAME split (networking audit 2026-07-11, finding 1).
//!
//! One client inside a 100-entity crowd: the server emits snapshot-sized frames
//! (~2.2 KiB — 100 entities at ~22 bytes each) that exceed the 1 KiB inbound cap.
//! Under the old shared 1 KiB `MAX_FRAME`, the client's reader rejected the first
//! such frame as "bad frame length" and the connection died. With the split caps
//! the client must receive every wave and stay connected.

use engine_net::{ClientEvent, NetClient, NetServer, ServerEvent, MAX_FRAME_IN, MAX_FRAME_OUT};
use std::time::{Duration, Instant};

const CROWD_ENTITIES: usize = 100;
const BYTES_PER_ENTITY: usize = 22; // ≥5B varint id + 12B position + up to 5B hp
const WAVES: usize = 5;

#[test]
fn client_survives_crowd_snapshot_waves() {
    let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
    let mut client = NetClient::connect(server.local_addr(), 1).expect("connect");

    // Wait for the server to see the connection.
    let deadline = Instant::now() + Duration::from_secs(5);
    let conn = loop {
        let connected = server.poll().into_iter().find_map(|ev| match ev {
            ServerEvent::Connected(id) => Some(id),
            _ => None,
        });
        if let Some(id) = connected {
            break id;
        }
        assert!(Instant::now() < deadline, "client never connected");
        std::thread::sleep(Duration::from_millis(10));
    };

    // A full-crowd snapshot: legal outbound, but over the inbound cap — the
    // exact shape that disconnected clients under the shared 1 KiB cap.
    let snapshot = vec![0xAB; CROWD_ENTITIES * BYTES_PER_ENTITY];
    assert!(snapshot.len() > MAX_FRAME_IN, "snapshot must exceed the inbound cap for this test to bite");
    assert!(snapshot.len() <= MAX_FRAME_OUT, "snapshot must be a legal outbound frame");

    for _ in 0..WAVES {
        server.send(conn, snapshot.clone());
    }

    // The client must receive every wave without a Disconnected event.
    let mut received = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while received < WAVES {
        for ev in client.poll() {
            match ev {
                ClientEvent::Message(data) => {
                    assert_eq!(data.len(), snapshot.len(), "snapshot arrived truncated");
                    received += 1;
                }
                ClientEvent::Disconnected => {
                    panic!("client disconnected during snapshot waves ({received}/{WAVES} received)")
                }
                ClientEvent::Connected => {}
            }
        }
        assert!(Instant::now() < deadline, "timed out with {received}/{WAVES} waves received");
        std::thread::sleep(Duration::from_millis(10));
    }

    // The server side must not have seen the client drop either.
    for ev in server.poll() {
        if let ServerEvent::Disconnected(id) = ev {
            assert_ne!(id, conn, "server saw the client disconnect");
        }
    }
}
