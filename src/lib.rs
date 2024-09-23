uniffi::setup_scaffolding!();

use std::io;
use std::time::Duration;
use std::time::Instant;

use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::autonat;
use libp2p::dcutr;
use libp2p::dns::ResolverConfig;
use libp2p::dns::ResolverOpts;
use libp2p::identify;
use libp2p::multiaddr::Protocol;
use libp2p::noise;
use libp2p::relay;
use libp2p::swarm::NetworkBehaviour;
use libp2p::swarm::SwarmEvent;
use libp2p::yamux;
use libp2p::{Multiaddr, PeerId, Stream, StreamProtocol};

use libp2p_stream as stream;
use rand::rngs::OsRng;
use rand::RngCore;

const ECHO_PROTOCOL: StreamProtocol = StreamProtocol::new("/echo");

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay_client: relay::client::Behaviour,
    autonat: autonat::v2::client::Behaviour,
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
            .expect("with_tcp")
            .with_dns_config(ResolverConfig::cloudflare(), ResolverOpts::default())
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .expect("with_relay_client")
            .with_behaviour(|keypair, relay_behaviour| Behaviour {
                relay_client: relay_behaviour,
                autonat: autonat::v2::client::Behaviour::new(
                    OsRng,
                    autonat::v2::client::Config::default(),
                ),
                identify: identify::Behaviour::new(identify::Config::new(
                    "/switchboard/0.0.1".to_string(),
                    keypair.public(),
                )),
                dcutr: dcutr::Behaviour::new(keypair.public().to_peer_id()),
                stream: stream::Behaviour::new(),
            })
            .expect("with_behaviour")
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(10)))
            .build();

        swarm
            .listen_on("/ip4/0.0.0.0/tcp/0".parse().unwrap())
            .expect("listen_on");

        // Wait to listen on all interfaces.
        let sleep = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(sleep);

        // TODO: Could enumerate interfaces and then ensure we get a listener for all of them
        loop {
            tokio::select! {
                event = swarm.next() => {
                    match event.expect("event") {
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

        let relay_address: Multiaddr = relay_address.parse().expect("parse");
        swarm.dial(relay_address.clone()).expect("dial");

        let mut learned_observed_addr = false;
        let mut told_relay_observed_addr = false;

        loop {
            match swarm.next().await.unwrap() {
                SwarmEvent::NewListenAddr { address, .. } => {
                    tracing::info!(%address, "Listening on address");
                }

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

        let remote: Multiaddr = url.parse().unwrap();
        let Protocol::P2p(peer_id) = remote.iter().last().unwrap() else {
            panic!("no peerid in addr");
        };

        swarm.dial(remote).unwrap();

        let control = swarm.behaviour().stream.new_control();

        tokio::spawn(async move { connection_handler(peer_id, control).await });

        while let Some(event) = swarm.next().await {
            match event {
                libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                    let listen_address = address.with_p2p(*swarm.local_peer_id()).unwrap();
                    tracing::info!(%listen_address, "new listen address");
                }

                event => tracing::info!(?event, "got event"),
            }
        }
    });
}

async fn connection_handler(peer: PeerId, mut control: stream::Control) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await; // Wait a second between echos.

        let stream = match control.open_stream(peer, ECHO_PROTOCOL).await {
            Ok(stream) => stream,
            Err(error @ stream::OpenStreamError::UnsupportedProtocol(_)) => {
                tracing::info!(%peer, %error);
                return;
            }
            Err(error) => {
                // Other errors may be temporary.
                // In production, something like an exponential backoff / circuit-breaker may be more appropriate.
                tracing::debug!(%peer, %error);
                continue;
            }
        };

        if let Err(e) = send(stream).await {
            tracing::warn!(%peer, "Echo protocol failed: {e}");
            continue;
        }

        tracing::info!(%peer, "Echo complete!")
    }
}

async fn send(mut stream: Stream) -> io::Result<()> {
    let num_bytes = rand::random::<usize>() % 1000;

    let mut bytes = vec![0; num_bytes];
    rand::thread_rng().fill_bytes(&mut bytes);

    stream.write_all(&bytes).await?;

    let mut buf = vec![0; num_bytes];
    stream.read_exact(&mut buf).await?;

    if bytes != buf {
        return Err(io::Error::new(io::ErrorKind::Other, "incorrect echo"));
    }

    stream.close().await?;

    Ok(())
}
