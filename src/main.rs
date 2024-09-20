use std::io;
use std::time::Duration;
use std::time::Instant;

use futures::{AsyncReadExt, StreamExt};
use libp2p::identity::Keypair;
use libp2p::{Stream, StreamProtocol};

const ECHO_PROTOCOL: StreamProtocol = StreamProtocol::new("/echo");

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let mut bytes = std::fs::read("private.pk8").unwrap();
    let keypair = Keypair::rsa_from_pkcs8(&mut bytes).unwrap();

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default().nodelay(true),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .unwrap()
        .with_quic()
        .with_behaviour(|_| libp2p_stream::Behaviour::default())
        .unwrap()
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(10)))
        .build();

    swarm
        .listen_on("/ip4/0.0.0.0/tcp/8000".parse().unwrap())
        .unwrap();

    swarm
        .listen_on("/ip4/0.0.0.0/udp/8000/quic-v1".parse().unwrap())
        .unwrap();

    let mut incoming_streams = swarm
        .behaviour()
        .new_control()
        .accept(ECHO_PROTOCOL)
        .unwrap();

    tokio::spawn(async move {
        while let Some((peer, stream)) = incoming_streams.next().await {
            tokio::spawn(async move {
                let _ = recv(stream).await;
            });
        }
    });

    loop {
        let event = swarm.next().await.expect("never terminates");

        match event {
            libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                let listen_address = address.with_p2p(*swarm.local_peer_id()).unwrap();
                tracing::info!(%listen_address);
            }

            event => tracing::info!(?event, "got event"),
        }
    }
}

async fn recv(mut stream: Stream) -> io::Result<usize> {
    let start = Instant::now();

    let mut total = 0;
    let mut total_messages = 0;

    let mut previous_amount = 0;
    let mut previous_time = Instant::now();

    let mut buf = [0u8; 16384];
    loop {
        let read = stream.read(&mut buf).await?;
        total_messages += 1;
        total += read;

        if total_messages % 100 == 0 {
            let elapsed = previous_time.elapsed();

            let read = total - previous_amount;
            let throughput = ((read * 8) as f64 / (1024 * 1024) as f64) / elapsed.as_secs_f64();

            println!(
                "received {} bytes over {:?}. throughput = {} mbps",
                total, elapsed, throughput
            );

            previous_amount = total;
            previous_time = Instant::now();
        }

        if read == 0 || buf[0] == 1 {
            let elapsed = start.elapsed();

            let throughput = (((total * 8) / (1024 * 1024)) as f64) / elapsed.as_secs_f64();
            println!(
                "FINAL: received {} bytes over {:?}. throughput = {} mbps",
                total, elapsed, throughput
            );

            return Ok(total);
        }
    }
}
