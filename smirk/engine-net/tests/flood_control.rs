//! Two hostile-bot scenarios the server must bound:
//!   - a single connection flooding app frames far faster than the sim's poll
//!     cadence must not have every frame forwarded into `ServerEvent`s (the
//!     reader-side token bucket bounds it) and must not be disconnected just
//!     for being fast — that's `WRITER_QUEUE_CAP`'s job (tested in server.rs),
//!     a different failure mode from flooding.
//!   - opening more connections than `MAX_CONNECTIONS_PER_IP` from one source
//!     address must have the excess refused, not accepted.

use engine_net::{ClientEvent, ConnId, NetClient, NetServer, ServerEvent};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn wait_for_connect(server: &mut NetServer) -> ConnId {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(id) = server.poll().into_iter().find_map(|ev| match ev {
            ServerEvent::Connected(id) => Some(id),
            _ => None,
        }) {
            return id;
        }
        assert!(Instant::now() < deadline, "client never connected");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn flooding_client_is_rate_limited_not_disconnected() {
    let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
    let mut client = NetClient::connect(server.local_addr(), 1).expect("connect");
    let conn = wait_for_connect(&mut server);

    // Flood: far more app frames than the token bucket's capacity plus any
    // refill that could plausibly land during the poll window below.
    const FLOOD: usize = 10_000;
    for _ in 0..FLOOD {
        client.send(vec![0xCDu8; 8]);
    }

    // The steady refill rate (NetServer::MSG_REFILL_PER_SEC = 120/s) caps
    // what a 5 s window can admit at ~128 + 120*5 = 728 messages — an order
    // of magnitude below FLOOD/2, so this bound can't be satisfied by
    // legitimate throughput alone; it requires the bucket to be dropping frames.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = 0usize;
    let mut disconnected = false;
    while Instant::now() < deadline {
        for ev in server.poll() {
            match ev {
                ServerEvent::Message { conn: c, .. } if c == conn => received += 1,
                ServerEvent::Disconnected(c) if c == conn => disconnected = true,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(!disconnected, "flooding disconnected the connection — rate limiting should drop frames, not kick");
    assert!(
        received < FLOOD / 2,
        "token bucket did not bound the flood: {received}/{FLOOD} frames reached the sim"
    );
    let rejects = server.metrics().rejects.load(Ordering::Relaxed);
    assert!(rejects > 0, "flood should have tripped the reject counter at least once");

    let _ = client.poll();
}

#[test]
fn extra_connections_from_one_ip_are_refused() {
    let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
    let addr = server.local_addr();

    const ATTEMPTS: usize = NetServer::MAX_CONNECTIONS_PER_IP + 4;
    let mut clients: Vec<NetClient> =
        (0..ATTEMPTS).map(|_| NetClient::connect(addr, 1).expect("connect() call itself")).collect();

    // Let every attempt resolve: accepted ones fire Connected on both sides;
    // refused ones fire Disconnected on the client without ever seeing Connected.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut server_connected = 0usize;
    let mut client_connected = [false; ATTEMPTS];
    loop {
        for ev in server.poll() {
            if let ServerEvent::Connected(_) = ev {
                server_connected += 1;
            }
        }
        for (i, c) in clients.iter_mut().enumerate() {
            for ev in c.poll() {
                if let ClientEvent::Connected = ev {
                    client_connected[i] = true;
                }
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        server_connected,
        NetServer::MAX_CONNECTIONS_PER_IP,
        "server accepted a different number of connections than MAX_CONNECTIONS_PER_IP allows"
    );
    let accepted_clients = client_connected.iter().filter(|&&c| c).count();
    assert_eq!(
        accepted_clients,
        NetServer::MAX_CONNECTIONS_PER_IP,
        "client-observed accept count should match the per-IP cap"
    );
}
