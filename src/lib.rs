uniffi::setup_scaffolding!();

use std::time::Duration;
use std::time::Instant;

use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::dcutr;
use libp2p::identify;
use libp2p::multiaddr::Protocol;
use libp2p::noise;
use libp2p::relay;
use libp2p::swarm::NetworkBehaviour;
use libp2p::swarm::SwarmEvent;
use libp2p::yamux;
use libp2p::{Multiaddr, PeerId, Stream, StreamProtocol};

use libp2p_stream as stream;

const ECHO_PROTOCOL: StreamProtocol = StreamProtocol::new("/echo");

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay_client: relay::client::Behaviour,
    identify: identify::Behaviour,
    dcutr: dcutr::Behaviour,
    stream: libp2p_stream::Behaviour,
}

#[uniffi::export]
pub fn run(url: String, relay_address: String, bandwidth: f64, duration: u64, buffer_size: u64) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        tracing_subscriber::fmt().init();

        let mut swarm = libp2p::SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default().nodelay(true),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )
            .unwrap()
            .with_quic()
            .with_dns()
            .unwrap()
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .unwrap()
            .with_behaviour(|keypair, relay_behaviour| Behaviour {
                relay_client: relay_behaviour,
                identify: identify::Behaviour::new(identify::Config::new(
                    "/TODO/0.0.1".to_string(),
                    keypair.public(),
                )),
                dcutr: dcutr::Behaviour::new(keypair.public().to_peer_id()),
                stream: stream::Behaviour::new(),
            })
            .unwrap()
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(10)))
            .build();

        swarm
            .listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())
            .unwrap();

        swarm
            .listen_on("/ip4/0.0.0.0/tcp/0".parse().unwrap())
            .unwrap();

        // Wait to listen on all interfaces.
        (async {
            let sleep = tokio::time::sleep(Duration::from_secs(1));
            tokio::pin!(sleep);

            loop {
                tokio::select! {
                    event = swarm.next() => {
                        match event.unwrap() {
                            SwarmEvent::NewListenAddr { address, .. } => {
                                tracing::info!(%address, "Listening on address");
                            }
                            event => panic!("{event:?}"),
                        }
                    }
                    _ = &mut sleep => {
                        // Likely listening on all interfaces now, thus continuing by breaking the loop.
                        break;
                    }
                }
            }
        })
        .await;

        let relay_address: Multiaddr = relay_address.parse().unwrap();
        swarm.dial(relay_address.clone()).unwrap();

        (async {
            let mut learned_observed_addr = false;
            let mut told_relay_observed_addr = false;

            loop {
                match swarm.next().await.unwrap() {
                    SwarmEvent::NewListenAddr { .. } => {}
                    SwarmEvent::Dialing { .. } => {}
                    SwarmEvent::ConnectionEstablished { .. } => {}
                    SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Sent {
                        ..
                    })) => {
                        tracing::info!("Told relay its public address");
                        told_relay_observed_addr = true;
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
                        info: identify::Info { observed_addr, .. },
                        ..
                    })) => {
                        tracing::info!(address=%observed_addr, "Relay told us our observed address");
                        learned_observed_addr = true;
                    }
                    event => panic!("{event:?}"),
                }

                if learned_observed_addr && told_relay_observed_addr {
                    break;
                }
            }
        }).await;

        let remote: Multiaddr = url.parse().unwrap();
        let Protocol::P2p(peer_id) = remote.iter().last().unwrap() else {
            panic!();
        };

        swarm.dial(remote).unwrap();

        let control = swarm.behaviour().stream.new_control();

        tokio::spawn(async move {
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
        });

        connection_handler(
            peer_id,
            control,
            bandwidth,
            Duration::from_secs(duration),
            buffer_size as usize,
        )
        .await;
    });
}

/// A very simple, `async fn`-based connection handler for our custom echo protocol.
async fn connection_handler(
    peer: PeerId,
    mut control: libp2p_stream::Control,
    bandwidth: f64,
    duration: Duration,
    buffer_size: usize,
) {
    let start = Instant::now();

    let bytes_per_sec = (bandwidth / 8.0) * (1024.0 * 1024.0);
    let sends_per_sec = bytes_per_sec / buffer_size as f64;

    let mut interval =
        tokio::time::interval(Duration::try_from_secs_f64(1.0 / sends_per_sec).unwrap());

    let mut stream = match control.open_stream(peer, ECHO_PROTOCOL).await {
        Ok(stream) => stream,
        Err(error @ libp2p_stream::OpenStreamError::UnsupportedProtocol(_)) => {
            tracing::warn!(%peer, %error, "failed to open stream");
            return;
        }
        Err(error) => {
            tracing::warn!(%peer, %error, "failed to open stream");
            return;
        }
    };

    println!("starting speed test");

    while start.elapsed() < duration {
        interval.tick().await;

        let bytes = vec![0; buffer_size];

        stream.write_all(&bytes).await.unwrap();
    }

    stream.close().await.unwrap();

    println!("speed test done");
}
