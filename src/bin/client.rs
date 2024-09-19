fn main() {
    // ios_test::run("150.136.219.122:8000".to_owned(), 10.0, 5, 2048);
    // ios_test::run("129.146.216.83:8000".to_owned(), 10.0, 5, 2048);
    ios_test::run(
        "/ip4/127.0.0.1/tcp/8000/p2p/Qmc9ubuon2MSBj2j58jh4i5sRR8eB9wMPp8QP2DezF3X8j".to_owned(),
        "/ip4/129.146.216.83/tcp/4001".to_owned(),
        10.0,
        5,
        2048,
    );
}
