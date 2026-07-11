//! Networking audit 2026-07-11, finding 17, path step 5: named WAN profiles
//! so client-feel claims have a headless test at recognizable real-world
//! conditions instead of only hand-picked individual numbers. Smoke test,
//! not a budget test: connect under each profile, send a small burst, and
//! confirm the connection survives and (eventually, since satellite's 600 ms
//! RTT plus loss means real retransmission delay) delivers most of it.
//!
//!   cargo test -p engine-net --test wan_profiles -- --ignored --nocapture

use engine_net::{Impairment, NetClient, NetServer, ServerEvent};
use std::time::{Duration, Instant};

#[test]
#[ignore = "WAN profile smoke test — real simulated latency, slow by design"]
fn named_wan_profiles_stay_connected_and_deliver() {
    const N: u32 = 50;
    for (name, impairment) in
        [("wifi", Impairment::wifi()), ("4g", Impairment::four_g()), ("satellite", Impairment::satellite())]
    {
        let mut server = NetServer::bind("127.0.0.1:0".parse().unwrap(), 1).expect("bind");
        let client = NetClient::connect_impaired(server.local_addr(), 1, impairment).expect("connect");

        let connect_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if server.poll().into_iter().any(|ev| matches!(ev, ServerEvent::Connected(_))) {
                break;
            }
            assert!(Instant::now() < connect_deadline, "{name}: client never connected");
            std::thread::sleep(Duration::from_millis(10));
        }

        for i in 0..N {
            client.send(i.to_le_bytes().to_vec());
        }

        // Generous deadline scaled to the profile's own RTT: reliable-stream
        // retransmission after simulated loss can take a few RTTs to recover.
        let deadline = Instant::now() + impairment.rtt * 8 + Duration::from_secs(3);
        let mut received = 0usize;
        let mut disconnected = false;
        while Instant::now() < deadline && received < N as usize {
            for ev in server.poll() {
                match ev {
                    ServerEvent::Message { .. } => received += 1,
                    ServerEvent::Disconnected(_) => disconnected = true,
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        println!("profile {name}: {received}/{N} delivered, rtt={:?}", impairment.rtt);
        assert!(!disconnected, "profile {name} disconnected mid-burst");
        assert!(
            received as f64 >= N as f64 * 0.5,
            "profile {name} delivered too few frames: {received}/{N}"
        );
    }
}
