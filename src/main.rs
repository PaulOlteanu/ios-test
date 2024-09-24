use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{debug_handler, Json, Router};
use serde::{Deserialize, Serialize};
use str0m::change::SdpOffer;
use str0m::net::{Protocol, Receive, Transmit};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, UdpSocket};
use tokio::sync::{mpsc, Mutex};

#[derive(Serialize, Deserialize, Debug)]
struct Candidates {
    candidates: Vec<String>,
}

struct AppState {
    rtc: Rtc,
    candidate: Candidate,
    ready_send: mpsc::UnboundedSender<()>,
    new_candidate_send: mpsc::UnboundedSender<()>,
}

type WrappedState = Arc<Mutex<AppState>>;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // let socket = UdpSocket::bind("0.0.0.0:8080").await.unwrap();
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("0.0.0.0:8080".parse().unwrap()).unwrap();
    let listener = socket.listen(1024).unwrap();

    let (in_send, mut in_recv) = mpsc::unbounded_channel::<Recv>();
    let (out_send, mut out_recv) = mpsc::unbounded_channel::<Transmit>();

    tokio::spawn(async move {
        let (mut connection, addr) = listener.accept().await.unwrap();
        let (mut recv, mut send) = connection.split();
        loop {
            let mut buf = vec![0; 33000];
            tokio::select! {
                Ok(n) = recv.read(&mut buf) => {
                    buf.truncate(n);
                    let recv = Recv {
                        source: addr,
                        contents: buf,
                    };

                    in_send.send(recv).unwrap();
                }

                Some(t) = out_recv.recv() => {
                    let mut buf = Cursor::new(Vec::from(t.contents));
                    send.write_all_buf(&mut buf).await.unwrap();
                }
            }
        }
        // while let next =
    });

    let mut rtc = RtcConfig::new().set_ice_lite(true).build();

    let addr = SocketAddr::from(([129, 146, 216, 83], 8080));
    let candidate = Candidate::host(addr, "tcp").unwrap();

    rtc.add_local_candidate(candidate.clone());

    let (ready_send, mut ready_recv) = mpsc::unbounded_channel();
    let (new_candidate_send, mut new_candidate_recv) = mpsc::unbounded_channel();

    let state = AppState {
        rtc,
        ready_send,
        new_candidate_send,
        candidate,
    };
    let state = Arc::new(Mutex::new(state));

    let app = Router::new()
        .route("/offer", post(offer_handler))
        .route("/candidate", post(candidate_handler))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    ready_recv.recv().await.unwrap();

    let mut received = 0;
    let mut start = None;

    loop {
        let timeout = {
            let mut state = state.lock().await;

            match state.rtc.poll_output().unwrap() {
                // Stop polling when we get the timeout.
                Output::Timeout(v) => v,

                // Transmit this data to the remote peer. Typically via
                // a UDP socket. The destination IP comes from the ICE
                // agent. It might change during the session.
                Output::Transmit(v) => {
                    out_send.send(v).unwrap();
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
                            println!("data channel opened {:?} {name}", id);
                        }

                        Event::ChannelData(data) => {
                            if start.is_none() {
                                start = Some(Instant::now());
                            }

                            received += data.data.len();

                            if data.data.first() == Some(&1) {
                                let elapsed = start.unwrap().elapsed();

                                let throughput = (((received * 8) / (1024 * 1024)) as f64)
                                    / elapsed.as_secs_f64();
                                println!(
                                    "received {} bytes over {:?}. throughput = {} mbps",
                                    received, elapsed, throughput
                                );
                                break;
                            }
                        }

                        Event::ChannelClose(id) => {
                            println!("data channel closed {:?}", id);
                        }
                        _ => {}
                    }

                    continue;
                }
            }
        };

        tokio::select! {
            Some(recv) = in_recv.recv() => {
                let destination = addr;

                let input = Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Tcp,
                        source: recv.source,
                        destination,
                        contents: recv.contents.as_slice().try_into().unwrap(),
                    }
                );

                let mut state = state.lock().await;
                state.rtc.handle_input(input).unwrap();
            }

            _ = new_candidate_recv.recv() => {
                continue;
            }

            _ = tokio::time::sleep_until(timeout.into()) => {
                let input = Input::Timeout(Instant::now());

                let mut state = state.lock().await;
                state.rtc.handle_input(input).unwrap();
            }
        };
    }
}

struct Recv {
    source: SocketAddr,
    contents: Vec<u8>,
}

#[debug_handler]
async fn offer_handler(State(state): State<WrappedState>, body: String) -> impl IntoResponse {
    println!("received offer {}", body);
    let mut state = state.lock().await;

    let offer = SdpOffer::from_sdp_string(&body).unwrap();
    let answer = state.rtc.sdp_api().accept_offer(offer).unwrap();

    state.ready_send.send(()).unwrap();

    dbg!(answer.to_sdp_string())
}

#[debug_handler]
async fn candidate_handler(State(state): State<WrappedState>, body: String) -> impl IntoResponse {
    println!("received candidate {}", body);
    let mut state = state.lock().await;
    let candidate = Candidate::from_sdp_string(&body).unwrap();

    state.rtc.add_remote_candidate(candidate);
    state.new_candidate_send.send(()).unwrap();
}
