uniffi::setup_scaffolding!();

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use quinn::{crypto::rustls::QuicClientConfig, ClientConfig, TransportConfig, VarInt};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

#[uniffi::export]
pub fn run(ip: String, port: u16) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        let addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let endpoint = quinn::Endpoint::client(addr).unwrap();

        let addr = format!("{}:{}", ip, port).parse().unwrap();
        let connection = endpoint
            .connect_with(configure_client(), addr, "speedtest")
            .unwrap()
            .await
            .unwrap();

        let (mut send_stream, _recv_stream) = connection.open_bi().await.unwrap();

        // 131072 bytes a sec
        println!("starting speed test");
        let start = Instant::now();

        // need to do 64 writes a sec
        let mut interval = tokio::time::interval(Duration::try_from_secs_f64(1.0 / 64.0).unwrap());

        while start.elapsed() <= Duration::from_secs(5) {
            interval.tick().await;
            let data = vec![0; 2048];
            send_stream.write_all(&data).await.unwrap();
        }

        send_stream.finish().unwrap();
        send_stream.stopped().await.unwrap();
        println!("finished speed test");
    });
}

pub fn configure_client() -> ClientConfig {
    let mut client_config = ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(SkipServerVerification::new())
                .with_no_client_auth(),
        )
        .unwrap(),
    ));

    let mut transport_config = TransportConfig::default();
    transport_config.max_idle_timeout(None); // TODO: IS THIS SUS?
    client_config.transport_config(Arc::new(transport_config));

    client_config
}

#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
