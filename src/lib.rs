uniffi::setup_scaffolding!();

use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};

use iroh_net::{key::SecretKey, relay::RelayMode, Endpoint, NodeAddr, NodeId};
use tracing::info;

// An example ALPN that we are using to communicate over the `Endpoint`
const EXAMPLE_ALPN: &[u8] = b"n0/iroh/examples/magic/0";

#[uniffi::export]
pub fn run(
    node_id: String,
    // relay_url: String,
    direct_addresses: String,
    bandwidth: f64,
    duration: f64,
    buffer_size: u64,
) {
    let duration = Duration::try_from_secs_f64(duration).unwrap();
    let buffer_size = buffer_size as usize;

    let node_id: NodeId = node_id.parse().unwrap();
    let direct_addresses = direct_addresses
        .split(',')
        .map(|x| x.trim().parse().unwrap())
        .collect::<Vec<SocketAddr>>();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let secret_key = SecretKey::generate();
    println!("secret key: {secret_key}");

    rt.block_on(async move {
        tracing_subscriber::fmt::init();

        // Build a `Endpoint`, which uses PublicKeys as node identifiers, uses QUIC for directly connecting to other nodes, and uses the relay protocol and relay servers to holepunch direct connections between nodes when there are NATs or firewalls preventing direct connections. If no direct connection can be made, packets are relayed over the relay servers.
        let endpoint = Endpoint::builder()
            // The secret key is used to authenticate with other nodes. The PublicKey portion of this secret key is how we identify nodes, often referred to as the `node_id` in our codebase.
            .secret_key(secret_key)
            // set the ALPN protocols this endpoint will accept on incoming connections
            .alpns(vec![EXAMPLE_ALPN.to_vec()])
            // `RelayMode::Default` means that we will use the default relay servers to holepunch and relay.
            // Use `RelayMode::Custom` to pass in a `RelayMap` with custom relay urls.
            // Use `RelayMode::Disable` to disable holepunching and relaying over HTTPS
            // If you want to experiment with relaying using your own relay server, you must pass in the same custom relay url to both the `listen` code AND the `connect` code
            .relay_mode(RelayMode::Default)
            // you can choose a port to bind to, but passing in `0` will bind the socket to a random available port
            .bind()
            .await
            .unwrap();

        let me = endpoint.node_id();
        println!("node id: {me}");
        println!("node listening addresses:");
        for local_endpoint in endpoint.direct_addresses().next().await.unwrap() {
            println!("\t{}", local_endpoint.addr)
        }

        let relay_url = endpoint.home_relay().unwrap();
        // .expect("should be connected to a relay server, try calling `endpoint.local_endpoints()` or `endpoint.connect()` first, to ensure the endpoint has actually attempted a connection before checking for the connected relay server");

        println!("node relay server url: {relay_url}\n");
        // Build a `NodeAddr` from the node_id, relay url, and UDP addresses.
        let addr = NodeAddr::from_parts(node_id, Some(relay_url), direct_addresses);

        // Attempt to connect, over the given ALPN.
        // Returns a Quinn connection.
        let conn = endpoint.connect(addr, EXAMPLE_ALPN).await.unwrap();
        info!("connected");

        // Use the Quinn API to send and recv content.
        let (mut send, mut recv) = conn.open_bi().await.unwrap();

        let start = Instant::now();

        let bytes_per_sec = (bandwidth / 8.0) * (1024.0 * 1024.0);
        let sends_per_sec = bytes_per_sec / buffer_size as f64;

        let mut interval =
            tokio::time::interval(Duration::try_from_secs_f64(1.0 / sends_per_sec).unwrap());

        println!("starting speed test");

        while start.elapsed() < duration {
            interval.tick().await;

            let bytes = vec![0; buffer_size];

            send.write_all(&bytes).await.unwrap();
        }

        let bytes = vec![1; buffer_size];
        send.write_all(&bytes).await.unwrap();
        send.finish().unwrap();
        let _ = send.stopped().await;

        println!("speed test done");

        let _ = endpoint.close(0u8.into(), b"bye").await;
    });
}
