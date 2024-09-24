use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{debug_handler, Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpSocket, UdpSocket};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::ice::network_type::NetworkType;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

#[derive(Serialize, Deserialize, Debug)]
struct Candidates {
    candidates: Vec<String>,
}

#[derive(Clone)]
struct AppState {
    peer_connection: Arc<RTCPeerConnection>,
    queued_candidates: Arc<AsyncMutex<Vec<RTCIceCandidateInit>>>,
}

struct Data {
    start: Instant,
    total: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let mut m = MediaEngine::default();
    m.register_default_codecs().unwrap();
    let mut registry = Registry::new();

    // Use the default set of Interceptors
    registry = register_default_interceptors(registry, &mut m).unwrap();

    let mut settings = SettingEngine::default();
    settings.set_network_types(vec![NetworkType::Tcp4]);
    let d = Duration::from_secs(30);
    settings.set_ice_timeouts(Some(d), Some(d), Some(d));

    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .with_setting_engine(settings)
        .build();

    // Prepare the configuration
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    let data_channels = Arc::new(Mutex::new(HashMap::new()));

    let peer_connection = Arc::new(api.new_peer_connection(config).await.unwrap());

    peer_connection.on_data_channel(Box::new(move |d| {
        println!("received data channel");

        let data = Data {
            start: Instant::now(),
            total: 0,
        };

        data_channels
            .lock()
            .unwrap()
            .insert(d.label().to_owned(), data);

        let label = d.label().to_owned();

        let dcs = data_channels.clone();

        Box::pin(async move {
            d.on_message(Box::new(move |msg| {
                println!("received message");

                if !msg.data.is_empty() && msg.data[0] == 1 {
                    println!("finished");
                    let elapsed = dcs.lock().unwrap().get(&label).unwrap().start.elapsed();
                    let received = dcs.lock().unwrap().get(&label).unwrap().total;

                    let throughput =
                        (((received * 8) / (1024 * 1024)) as f64) / elapsed.as_secs_f64();
                    println!(
                        "received {} bytes over {:?}. throughput = {} mbps",
                        received, elapsed, throughput
                    );
                } else {
                    dcs.lock().unwrap().get_mut(&label).unwrap().total += msg.data.len();
                }

                Box::pin(async {})
            }))
        })
    }));

    let queued_candidates = Arc::new(AsyncMutex::new(Vec::new()));
    let state = AppState {
        peer_connection,
        queued_candidates,
    };

    let app = Router::new()
        .route("/offer", post(offer_handler))
        .route("/candidate", post(candidate_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

#[debug_handler]
async fn offer_handler(State(state): State<AppState>, body: String) -> impl IntoResponse {
    println!("received offer {}", body);
    let description = RTCSessionDescription::offer(body).unwrap();
    state
        .peer_connection
        .set_remote_description(description)
        .await
        .unwrap();

    let answer = state.peer_connection.create_answer(None).await.unwrap();
    state
        .peer_connection
        .set_local_description(answer)
        .await
        .unwrap();

    let mut gather_complete = state.peer_connection.gathering_complete_promise().await;
    let _ = gather_complete.recv().await;

    let local_desc = state.peer_connection.local_description().await.unwrap();
    println!("sending answer {}", local_desc.sdp);
    local_desc.sdp
}

#[debug_handler]
async fn candidate_handler(State(state): State<AppState>, body: String) -> impl IntoResponse {
    println!("received candidate {}", body);
    let candidate = RTCIceCandidateInit {
        candidate: body,
        ..Default::default()
    };

    if state.peer_connection.remote_description().await.is_some() {
        for candidate in state.queued_candidates.lock().await.drain(..) {
            state
                .peer_connection
                .add_ice_candidate(candidate)
                .await
                .unwrap();
        }

        state
            .peer_connection
            .add_ice_candidate(candidate)
            .await
            .unwrap();
    } else {
        state.queued_candidates.lock().await.push(candidate);
    }
}
