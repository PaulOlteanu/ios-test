use std::net::{SocketAddr, UdpSocket};
use std::{collections::HashMap, time::Instant};

struct Data {
    start: Instant,
    amount: usize,
}

fn main() {
    let addr = SocketAddr::from(([0, 0, 0, 0], 8000));
    let socket = UdpSocket::bind(addr).unwrap();
    let mut buf = vec![0; 16384];

    let mut in_progress: HashMap<SocketAddr, Data> = HashMap::new();

    loop {
        let (n, addr) = socket.recv_from(&mut buf).unwrap();
        if n == 0 {
            continue;
        }

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
    }
}
