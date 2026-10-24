mod demo;

use harness_engine::RunState;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("states") if args.next().is_none() => {
            for state in RunState::ALL {
                println!("{state}");
            }
        }
        Some("run") if args.next().as_deref() == Some("--demo") && args.next().is_none() => {
            if let Err(error) = demo::run() {
                eprintln!("Falha no demo: {error}");
                std::process::exit(1);
            }
        }
        Some("--version" | "-V") if args.next().is_none() => {
            println!("harness {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!("Uso: harness <states|--version|run --demo>");
            std::process::exit(2);
        }
    }
}
