//! `ohmforge` command-line runner (stub).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => println!("ohmforge {}", env!("CARGO_PKG_VERSION")),
        _ => {
            println!("ohmforge {} (pre-alpha scaffold)", env!("CARGO_PKG_VERSION"));
            println!("Usage: ohmforge [--version]");
        }
    }
}
