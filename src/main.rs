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
    offer_send: mpsc::UnboundedSender<()>,
    new_candidate_send: mpsc::UnboundedSender<()>,
}

type WrappedState = Arc<Mutex<AppState>>;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut registry = Registry::new();

    // Use the default set of Interceptors
    registry = register_default_interceptors(registry, &mut m)?;

    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();

    // Prepare the configuration
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    let peer_connection = Arc::new(api.new_peer_connection(config).await?);
}

#[debug_handler]
async fn offer_handler(State(state): State<WrappedState>, body: String) -> impl IntoResponse {
    println!("received offer {}", body);
    let mut state = state.lock().await;

    let offer = SdpOffer::from_sdp_string(&body).unwrap();
    let answer = state.rtc.sdp_api().accept_offer(offer).unwrap();

    answer.to_sdp_string()
}

#[debug_handler]
async fn get_candidate_handler(State(state): State<WrappedState>) -> impl IntoResponse {
    println!("request to get candidate");
    let state = state.lock().await;

    state.ready_send.send(()).unwrap();

    state.candidate.to_sdp_string()
}

#[debug_handler]
async fn candidate_handler(State(state): State<WrappedState>, body: String) -> impl IntoResponse {
    println!("received candidate {}", body);
    let mut state = state.lock().await;
    let candidate = Candidate::from_sdp_string(&body).unwrap();

    state.rtc.add_remote_candidate(candidate);
    state.new_candidate_send.send(()).unwrap();
}
