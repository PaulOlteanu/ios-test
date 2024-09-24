fn main() {
    let node_id = "kec7t5o4nw6cnbbbkk3l7nbdgq3jfkkgqg6mtuk6ceg56lrlp5la".to_owned();
    let direct_addresses = "173.33.27.146:32947,173.33.27.146:54463,100.69.120.135:54463,172.17.0.1:54463,172.18.0.1:54463,192.168.0.123:54463,192.168.122.1:54463".to_owned();

    ios_test::run(node_id, direct_addresses, 10.0, 5.0, 2048);
}
