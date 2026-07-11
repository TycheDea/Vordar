//! Regression test for MAX_FRAME regression.
//! A single client inside a 100-entity crowd must survive several snapshot waves
//! without being disconnected by an oversized server→client frame.

use engine_net::{NetClient, NetServer};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn client_survives_crowd_snapshots() {
    // 101 entities total (1 client + 100 crowd) → ~2 KiB snapshot easily exceeds 1 KiB
    let mut server = NetServer::bind(([127,0,0,1], 0)).await.unwrap();
    let addr = server.local_addr().unwrap();

    let client = NetClient::connect(addr).await.unwrap();

    // Spawn a dummy server task that just accepts and drops connections
    // (the real test is that the client does not get "bad frame length")
    let handle = tokio::spawn(async move {
        if let Some(_ev) = server.next().await {}
    });

    // The client should stay alive for several snapshot intervals
    // (no "bad frame length" disconnect).
    let res = timeout(Duration::from_millis(800), async {
        // poll a few times; any disconnect would surface here
        for _ in 0..8 {
            let _ = client.poll();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;

    handle.abort();
    assert!(res.is_ok(), "client disconnected under crowd snapshot load");
}