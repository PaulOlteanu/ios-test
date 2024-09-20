fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS") == Ok("ios".to_owned()) {
        println!("cargo:rustc-link-lib=framework=SystemConfiguration");
    }
}
