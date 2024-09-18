uniffi::setup_scaffolding!();

use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use tokio::net::UdpSocket;

#[uniffi::export]
pub fn run(url: String, bandwidth: f64, duration: u64, buffer_size: u64) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        let addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let socket = UdpSocket::bind(addr).await.unwrap();

        let server_addr = tokio::net::lookup_host(url).await.unwrap().next().unwrap();
        println!("sending to {}", server_addr);

        socket.connect(server_addr).await.unwrap();

        let bytes_per_sec = (bandwidth / 8.0) * (1024.0 * 1024.0);
        let sends_per_sec = bytes_per_sec / buffer_size as f64;

        println!("starting speed test");

        let start = Instant::now();

        let mut interval =
            tokio::time::interval(Duration::try_from_secs_f64(1.0 / sends_per_sec).unwrap());

        while start.elapsed() <= Duration::from_secs(duration) {
            interval.tick().await;

            let data = vec![0; buffer_size as usize];
            // println!("sending {:?}", data);

            socket.writable().await.unwrap();
            let n = socket.send(&data).await.unwrap();
            println!("sent {}", n);
        }

        let data = [1; 32];
        socket.send(&data).await.unwrap();

        println!("finished speed test");
    });
}
