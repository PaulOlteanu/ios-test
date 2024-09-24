//! The smallest example showing how to use iroh-net and [`iroh_net::Endpoint`] to connect two devices.
//!
//! This example uses the default relay servers to attempt to holepunch, and will use that relay server to relay packets if the two devices cannot establish a direct UDP connection.
//! run this example from the project root:
//!     $ cargo run --example listen
use std::time::{Duration, Instant};

use futures::StreamExt;
use iroh_net::endpoint::ConnectionError;
use iroh_net::key::SecretKey;
use iroh_net::relay::RelayMode;
use iroh_net::Endpoint;
use tracing::{debug, info, warn};

// An example ALPN that we are using to communicate over the `Endpoint`
const EXAMPLE_ALPN: &[u8] = b"n0/iroh/examples/magic/0";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let secret_key = SecretKey::generate();
    println!("secret key: {secret_key}");

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

    let local_addrs = endpoint
        .direct_addresses()
        .next()
        .await
        .unwrap()
        .into_iter()
        .map(|endpoint| {
            let addr = endpoint.addr.to_string();
            println!("\t{addr}");
            addr
        })
        .collect::<Vec<_>>()
        .join(",");

    println!("local addrs: {}", local_addrs);

    let relay_url = endpoint.home_relay().unwrap();
    println!("node relay server url: {relay_url}");
    println!("\nin a separate terminal run:");

    println!(
        "\tcargo run --example connect -- --node-id {me} --addrs \"{local_addrs}\" --relay-url {relay_url}\n"
    );
    // accept incoming connections, returns a normal QUIC connection
    while let Some(incoming) = endpoint.accept().await {
        let mut connecting = match incoming.accept() {
            Ok(connecting) => connecting,
            Err(err) => {
                warn!("incoming connection failed: {err:#}");
                // we can carry on in these cases:
                // this can be caused by retransmitted datagrams
                continue;
            }
        };
        let alpn = connecting.alpn().await.unwrap();
        let conn = connecting.await.unwrap();
        let node_id = iroh_net::endpoint::get_remote_node_id(&conn).unwrap();
        info!(
            "new connection from {node_id} with ALPN {} (coming from {})",
            String::from_utf8_lossy(&alpn),
            conn.remote_address()
        );

        // spawn a task to handle reading and writing off of the connection
        tokio::spawn(async move {
            // accept a bi-directional QUIC connection
            // use the `quinn` APIs to send and recv content
            let (mut send, mut recv) = conn.accept_bi().await.unwrap();
            debug!("accepted bi stream, waiting for data...");

            let start = Instant::now();

            let mut total = 0;
            let mut total_messages = 0;

            let mut previous_amount = 0;
            let mut previous_time = Instant::now();

            let mut buf = [0u8; 16384];
            loop {
                match recv.read(&mut buf).await.unwrap() {
                    None | Some(0) => {
                        let elapsed = start.elapsed();

                        let throughput =
                            (((total * 8) / (1024 * 1024)) as f64) / elapsed.as_secs_f64();
                        println!(
                            "FINAL: received {} bytes over {:?}. throughput = {} mbps",
                            total, elapsed, throughput
                        );

                        break;
                    }

                    Some(n) => {
                        if buf[0] == 1 {
                            let elapsed = start.elapsed();

                            let throughput =
                                (((total * 8) / (1024 * 1024)) as f64) / elapsed.as_secs_f64();
                            println!(
                                "FINAL: received {} bytes over {:?}. throughput = {} mbps",
                                total, elapsed, throughput
                            );

                            break;
                        }

                        total_messages += 1;
                        total += n;

                        if total_messages % 100 == 0 {
                            let elapsed = previous_time.elapsed();

                            let read = total - previous_amount;
                            let throughput =
                                ((read * 8) as f64 / (1024 * 1024) as f64) / elapsed.as_secs_f64();

                            println!(
                                "received {} bytes over {:?}. throughput = {} mbps",
                                total, elapsed, throughput
                            );

                            previous_amount = total;
                            previous_time = Instant::now();
                        }
                    }
                }
            }
        });
    }
}
