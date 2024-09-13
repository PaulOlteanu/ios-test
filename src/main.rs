use std::{net::SocketAddr, sync::Arc, time::Instant};

use quinn::{Endpoint, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

#[tokio::main]
async fn main() {
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    let (config, _) = configure_server();

    let endpoint = Endpoint::server(config, addr).unwrap();

    loop {
        while let Some(incoming) = endpoint.accept().await {
            tokio::spawn(async move {
                let connection = incoming.accept().unwrap().await.unwrap();
                println!("got connection");

                let (send_stream, mut recv_stream) = connection.accept_bi().await.unwrap();

                let start = Instant::now();
                let mut total_sent = 0;
                let mut buf = vec![9; 4096];
                while let Ok(Some(n)) = recv_stream.read(&mut buf).await {
                    println!("received data chunk len={}", n);
                    total_sent += n;
                }

                println!("connection finished");
                println!("{} bytes sent in {:?}", total_sent, start.elapsed());
            });
        }
    }
}

pub(crate) fn configure_server() -> (ServerConfig, CertificateDer<'static>) {
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "hostname".into()]).unwrap();
    let cert_der = CertificateDer::from(cert.cert);
    let priv_key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

    let mut server_config =
        ServerConfig::with_single_cert(vec![cert_der.clone()], priv_key.into()).unwrap();
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.max_concurrent_uni_streams(0_u8.into());
    transport_config.max_idle_timeout(None); // TODO: IS THIS SUS?

    (server_config, cert_der)
}
