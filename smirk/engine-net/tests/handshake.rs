//! Regression test for the networking audit 2026-07-11, finding 16: a
//! protocol version mismatch used to be a silent close — the server returned
//! an `Err` and dropped the connection with no reason ever reaching the
//! client. This asserts the server now sends a `Ctrl::Reject` frame carrying
//! the reason, and the client surfaces it as `ClientEvent::Rejected` instead
//! of just a bare `Disconnected`.

use engine_net::{ClientEvent, NetClient, NetServer, ServerEvent};
use std::time::{Duration, Instant};

#[test]
fn version_mismatch_is_rejected_with_a_reason_not_a_silent_close() {
    let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
    let addr = server.local_addr();
    // Client speaks protocol version 2; the server was bound with version 1.
    let mut client = NetClient::connect(addr, 2).expect("connect() call itself");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut rejection: Option<String> = None;
    let mut server_connected = false;
    loop {
        for ev in client.poll() {
            match ev {
                ClientEvent::Rejected(reason) => rejection = Some(reason),
                ClientEvent::Connected => panic!("client connected despite a version mismatch"),
                ClientEvent::Disconnected | ClientEvent::Message(_) => {}
            }
        }
        if server.poll().into_iter().any(|ev| matches!(ev, ServerEvent::Connected(_))) {
            server_connected = true;
        }
        if rejection.is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "client never received a Rejected event for the version mismatch");
        std::thread::sleep(Duration::from_millis(10));
    }

    let reason = rejection.expect("Rejected event carries a reason");
    assert!(
        reason.contains("version mismatch"),
        "rejection reason should explain the version mismatch, got: {reason}"
    );
    assert!(!server_connected, "server must never register a connection whose handshake failed");
}
