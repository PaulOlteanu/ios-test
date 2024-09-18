use std::{collections::HashMap, net::SocketAddr, time::Instant};

use tokio::net::UdpSocket;

struct Data {
    start: Instant,
    amount: usize,
}

#[tokio::main]
async fn main() {
    let addr = SocketAddr::from(([0, 0, 0, 0], 8000));
    let socket = UdpSocket::bind(addr).await.unwrap();
    let mut buf = vec![0; 16384];

    let mut in_progress: HashMap<SocketAddr, Data> = HashMap::new();

    loop {
        let (n, addr) = socket.recv_buf_from(&mut buf).await.unwrap();
        if n == 0 {
            continue;
        }

        println!("received {} bytes from {}", n, addr);

        if buf[0] == 1 {
            if let Some(data) = in_progress.remove(&addr) {
                let amount = data.amount + n;
                let duration = data.start.elapsed();
                let throughput = (((amount * 8) / (1024 * 1024)) as f64) / duration.as_secs_f64();
                println!(
                    "received {} bytes over {:?} from {}. throughput = {} mbps",
                    amount, duration, addr, throughput
                );
            }
            continue;
        }

        let entry = in_progress.entry(addr).or_insert_with(|| {
            println!("adding new client {}", addr);
            Data {
                start: Instant::now(),
                amount: 0,
            }
        });
        entry.amount += n;

        // println!("received {} bytes from {}", x, y);
    }
}
