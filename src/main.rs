use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{debug_handler, Json, Router};
use serde::{Deserialize, Serialize};
use str0m::change::SdpOffer;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
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

    let socket = UdpSocket::bind("0.0.0.0:8000").await.unwrap();

    let mut rtc = Rtc::new();

    // let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    let addr = SocketAddr::from(([129, 146, 216, 83], 8000));
    let candidate = Candidate::host(addr, "udp").unwrap();

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
        .route(
            "/candidate",
            get(get_candidate_handler).post(candidate_handler),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
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
                    socket.send_to(&v.contents, v.destination).await.unwrap();
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

        let mut buf = vec![0; 4096];

        let input = tokio::select! {
            input = socket.recv_from(&mut buf) => {
                let (n, source) = input.unwrap();
                buf.truncate(n);

                let destination = addr;

                Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination,
                        contents: buf.as_slice().try_into().unwrap(),
                    }
                )
            }

            _ = new_candidate_recv.recv() => {
                continue;
            }

            _ = tokio::time::sleep_until(timeout.into()) => {
                Input::Timeout(Instant::now())
            }
        };

        let mut state = state.lock().await;
        // Input is either a Timeout or Receive of data. Both drive the state forward.
        state.rtc.handle_input(input).unwrap();
    }
}

#[debug_handler]
async fn offer_handler(State(state): State<WrappedState>, body: String) -> impl IntoResponse {
    let mut state = state.lock().await;

    let offer = SdpOffer::from_sdp_string(&body).unwrap();
    let answer = state.rtc.sdp_api().accept_offer(offer).unwrap();

    answer.to_sdp_string()
}

#[debug_handler]
async fn get_candidate_handler(State(state): State<WrappedState>) -> impl IntoResponse {
    let state = state.lock().await;

    state.ready_send.send(()).unwrap();

    state.candidate.to_sdp_string()
}

#[debug_handler]
async fn candidate_handler(State(state): State<WrappedState>, body: String) -> impl IntoResponse {
    let mut state = state.lock().await;
    let candidate = Candidate::from_sdp_string(&body).unwrap();

    state.rtc.add_remote_candidate(candidate);
    state.new_candidate_send.send(()).unwrap();
}
