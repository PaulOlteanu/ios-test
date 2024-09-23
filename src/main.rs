use std::io;
use std::time::Duration;
use std::time::Instant;

use futures::AsyncWriteExt;
use futures::{AsyncReadExt, StreamExt};
use libp2p::autonat;
use libp2p::dcutr;
use libp2p::dns::ResolverConfig;
use libp2p::dns::ResolverOpts;
use libp2p::identify;
use libp2p::identity::Keypair;
use libp2p::noise;
use libp2p::relay;
use libp2p::swarm::NetworkBehaviour;
use libp2p::swarm::SwarmEvent;
use libp2p::yamux;
use libp2p::Multiaddr;
use libp2p::{Stream, StreamProtocol};
use libp2p_stream as stream;
use rand::rngs::OsRng;
use tracing::info;

const ECHO_PROTOCOL: StreamProtocol = StreamProtocol::new("/echo");

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay_client: relay::client::Behaviour,
    autonat: autonat::v2::client::Behaviour,
    identify: identify::Behaviour,
    dcutr: dcutr::Behaviour,
    stream: libp2p_stream::Behaviour,
}

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
        .listen_on("/ip4/0.0.0.0/tcp/8000".parse().unwrap())
        .unwrap();

    // Wait to listen on all interfaces.
    let sleep = tokio::time::sleep(Duration::from_secs(1));
    tokio::pin!(sleep);

    // TODO: Could enumerate interfaces and then ensure we get a listener for all of them
    loop {
        tokio::select! {
            event = swarm.next() => {
                match event.expect("event") {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        let listen_address = address.with_p2p(*swarm.local_peer_id()).unwrap();
                        tracing::info!(%listen_address, "Listening on address");
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

    let relay_address = std::env::args().nth(1).unwrap();
    let relay_address: Multiaddr = relay_address.parse().expect("parse");
    swarm.dial(relay_address.clone()).expect("dial");

    let mut learned_observed_addr = false;
    let mut told_relay_observed_addr = false;

    loop {
        match swarm.next().await.unwrap() {
            SwarmEvent::NewListenAddr { address, .. } => {
                let listen_address = address.with_p2p(*swarm.local_peer_id()).unwrap();
                tracing::info!(%listen_address, "Listening on address");
            }

            SwarmEvent::Dialing { .. } => {}
            SwarmEvent::ConnectionEstablished { .. } => {}
            SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Sent { .. })) => {
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

    let mut incoming_streams = swarm
        .behaviour()
        .stream
        .new_control()
        .accept(ECHO_PROTOCOL)
        .unwrap();

    tokio::spawn(async move {
        while let Some((peer_id, stream)) = incoming_streams.next().await {
            info!(%peer_id, "new stream");
            tokio::spawn(async move {
                let _ = recv(stream).await;
            });
        }
    });

    while let Some(event) = swarm.next().await {
        match event {
            libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                let listen_address = address.with_p2p(*swarm.local_peer_id()).unwrap();
                tracing::info!(%listen_address, "Listening on address");
            }

            event => tracing::info!(?event, "got event"),
        }
    }
}

async fn recv(mut stream: Stream) -> io::Result<usize> {
    let mut total = 0;

    let mut buf = [0u8; 2048];

    loop {
        let read = stream.read(&mut buf).await?;
        if read == 0 {
            return Ok(total);
        }

        total += read;
        stream.write_all(&buf[..read]).await?;
    }
}
