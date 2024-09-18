uniffi::setup_scaffolding!();

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use quinn::{crypto::rustls::QuicClientConfig, ClientConfig, TransportConfig};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

#[uniffi::export]
pub fn run(url: String, bandwidth: f64, duration: u64, buffer_size: u64) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        println!("connecting");
        let addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let endpoint = quinn::Endpoint::client(addr).unwrap();

        let addr = tokio::net::lookup_host(url).await.unwrap().next().unwrap();

        let connection = endpoint
            .connect_with(configure_client(), addr, "speedtest")
            .unwrap()
            .await
            .unwrap();

        let (mut send_stream, _recv_stream) = connection.open_bi().await.unwrap();

        let bytes_per_sec = (bandwidth / 8.0) * (1024.0 * 1024.0);
        let sends_per_sec = bytes_per_sec / buffer_size as f64;

        println!("starting speed test");

        let start = Instant::now();

        let mut interval =
            tokio::time::interval(Duration::try_from_secs_f64(1.0 / sends_per_sec).unwrap());

        while start.elapsed() <= Duration::from_secs(duration) {
            interval.tick().await;

            let data = vec![0; buffer_size as usize];
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
