uniffi::setup_scaffolding!();

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use str0m::{
    change::SdpAnswer,
    channel::{ChannelConfig, Reliability},
    net::{Protocol, Receive},
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
};
use stunclient::StunClient;
use tokio::net::UdpSocket;

#[derive(Serialize, Deserialize)]
struct Candidates {
    candidates: Vec<Candidate>,
}

#[uniffi::export]
pub fn run(host: String, port: u16, bandwidth: f64, duration: u64, buffer_size: u64) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        tracing_subscriber::fmt::init();

        let mut rtc = Rtc::new();
        let server_url = format!("http://{host}:{port}");
        let client = reqwest::Client::new();

        let addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let host_socket = UdpSocket::bind(addr).await.unwrap();
        let host_socket = Arc::new(host_socket);
        let host_addr =
            SocketAddr::from(([127, 0, 0, 1], host_socket.local_addr().unwrap().port()));

        let addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let srflx_socket = UdpSocket::bind(addr).await.unwrap();
        let srflx_socket = Arc::new(srflx_socket);
        let srflx_addr =
            SocketAddr::from(([127, 0, 0, 1], srflx_socket.local_addr().unwrap().port()));

        println!("getting srflx addr");

        let stun_addr = "stun.l.google.com:19302";
        let stun_addr = tokio::net::lookup_host(stun_addr)
            .await
            .unwrap()
            .next()
            .unwrap();

        let stun_client = StunClient::new(stun_addr);
        let srflx = stun_client
            .query_external_address_async(&srflx_socket)
            .await
            .unwrap();

        println!("got srflx addr {}", stun_addr);

        let mut candidates = Vec::new();
        let mut sockets = HashMap::new();

        let candidate = Candidate::host(host_addr, "udp").unwrap();
        rtc.add_local_candidate(candidate.clone());
        candidates.push(candidate);
        sockets.insert(host_addr, host_socket.clone());

        let base = SocketAddr::from(([0, 0, 0, 0], srflx_socket.local_addr().unwrap().port()));
        let candidate = Candidate::server_reflexive(srflx, base, "udp").unwrap();
        rtc.add_local_candidate(candidate.clone());
        candidates.push(candidate);
        sockets.insert(base, srflx_socket.clone());

        let mut change = rtc.sdp_api();
        let channel_config = ChannelConfig {
            label: "jklfds".to_owned(),
            // ordered: false,
            ordered: true,
            // reliability: Reliability::MaxRetransmits { retransmits: 0 },
            reliability: Reliability::Reliable,
            negotiated: None,
            protocol: "jklfds".to_owned(),
        };

        change.add_channel_with_config(channel_config);

        let (offer, pending) = change.apply().unwrap();

        let reply = client
            .post(format!("{server_url}/offer"))
            .body(offer.to_sdp_string())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        let answer = SdpAnswer::from_sdp_string(&reply).unwrap();
        rtc.sdp_api().accept_answer(pending, answer).unwrap();

        let candidates = Candidates { candidates };
        let candidates: Candidates = client
            .post(format!("{server_url}/candidate"))
            .json(&candidates)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        for candidate in candidates.candidates {
            rtc.add_remote_candidate(candidate);
        }

        let data_channel_id = Mutex::new(None);

        let bytes_per_sec = (bandwidth / 8.0) * (1024.0 * 1024.0);
        let sends_per_sec = bytes_per_sec / buffer_size as f64;

        println!("starting speed test");

        let mut interval =
            tokio::time::interval(Duration::try_from_secs_f64(1.0 / sends_per_sec).unwrap());

        let mut end = Instant::now() + Duration::from_secs(duration);
        let very_end = end + Duration::from_secs(duration + 30);

        let mut should_send = false;
        let mut done = false;
        let mut num_sent = 0;

        loop {
            let timeout = match rtc.poll_output().unwrap() {
                // Stop polling when we get the timeout.
                Output::Timeout(v) => v,

                // Transmit this data to the remote peer. Typically via
                // a UDP socket. The destination IP comes from the ICE
                // agent. It might change during the session.
                Output::Transmit(v) => {
                    if let Some(socket) = sockets.get(&v.source) {
                        socket.send_to(&v.contents, v.destination).await.unwrap();
                    }
                    continue;
                }

                // Events are mainly incoming media data from the remote
                // peer, but also data channel data and statistics.
                Output::Event(v) => {
                    // Abort if we disconnect.
                    if v == Event::IceConnectionStateChange(IceConnectionState::Disconnected) {
                        return;
                    }

                    match v {
                        Event::Connected => {
                            println!("connected");
                        }

                        Event::IceConnectionStateChange(state) => {
                            println!("Ice connection state change {:?}", state);
                        }

                        Event::ChannelOpen(id, name) => {
                            *data_channel_id.lock().unwrap() = Some(id);
                            should_send = true;
                            println!("data channel opened {:?} {name}", id);
                        }

                        Event::ChannelData(data) => {
                            println!("data channel data {:?}", data);
                        }

                        Event::ChannelClose(id) => {
                            println!("data channel closed {:?}", id);
                        }
                        _ => {}
                    }

                    continue;
                }
            };

            if done {
                end = Instant::now() + Duration::from_secs(1000);
            }

            let mut buf1 = vec![0; 2048];
            let mut buf2 = vec![0; 2048];

            let input = tokio::select! {
                input = host_socket.recv_from(&mut buf1) => {
                    let (n, source) = input.unwrap();
                    buf1.truncate(n);

                    let destination = host_addr;

                    Input::Receive(
                        Instant::now(),
                        Receive {
                            proto: Protocol::Udp,
                            source,
                            destination,
                            contents: buf1.as_slice().try_into().unwrap(),
                        }
                    )
                }

                input = srflx_socket.recv_from(&mut buf2) => {
                    let (n, source) = input.unwrap();
                    buf2.truncate(n);

                    let destination = srflx_addr;

                    Input::Receive(
                        Instant::now(),
                        Receive {
                            proto: Protocol::Udp,
                            source,
                            destination,
                            contents: buf2.as_slice().try_into().unwrap(),
                        }
                    )
                }

                _ = interval.tick() => {
                    if let Some(id) = *data_channel_id.lock().unwrap() {
                        if should_send {
                            let data = vec![0; buffer_size as usize];
                            rtc.channel(id).unwrap().write(false, &data).unwrap();
                            num_sent += 1;
                        }
                    }

                    continue;
                }

                _ = tokio::time::sleep_until(end.into()) => {
                    if let Some(id) = *data_channel_id.lock().unwrap() {
                        let data = vec![1; buffer_size as usize];
                        rtc.channel(id).unwrap().write(false, &data).unwrap();
                    }

                    println!("finished speed test. sent {} messages", num_sent);
                    done = true;
                    should_send = false;
                    continue;
                }

                _ = tokio::time::sleep_until(very_end.into()) => {
                    break;
                }

                _ = tokio::time::sleep_until(timeout.into()) => {
                    Input::Timeout(Instant::now())
                }
            };

            // Input is either a Timeout or Receive of data. Both drive the state forward.
            rtc.handle_input(input).unwrap();
        }
    });
}

use std::net::IpAddr;
use systemstat::{Platform, System};

pub fn select_host_address() -> IpAddr {
    let system = System::new();
    let networks = system.networks().unwrap();

    for net in networks.values() {
        for n in &net.addrs {
            if let systemstat::IpAddr::V4(v) = n.addr {
                if !v.is_loopback() && !v.is_link_local() && !v.is_broadcast() {
                    return IpAddr::V4(v);
                }
            }
        }
    }

    panic!("Found no usable network interface");
}
