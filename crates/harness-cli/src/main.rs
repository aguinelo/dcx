mod codex;
mod demo;
mod schemas;

use harness_engine::RunState;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("states") if args.next().is_none() => {
            for state in RunState::ALL {
                println!("{state}");
            }
        }
        Some("schemas") => {
            if let Err(error) = schemas::run(args) {
                eprintln!("Erro ao processar schemas: {error}");
                std::process::exit(1);
            }
        }
        Some("run") => match args.next().as_deref() {
            Some("--demo") if args.next().is_none() => {
                if let Err(error) = demo::run() {
                    eprintln!("Falha no demo: {error}");
                    std::process::exit(1);
                }
            }
            Some("--pilot") if args.next().is_none() => {
                if let Err(error) = demo::run_pilot() {
                    eprintln!("Falha no piloto: {error}");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Uso: harness run < --demo | --pilot >");
                std::process::exit(2);
            }
        },
        Some("--version" | "-V") if args.next().is_none() => {
            println!("harness {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!(
                "Uso: harness <states|schemas --write|--check [DIR]|--version|run --demo|--pilot>"
            );
            std::process::exit(2);
        }
    }
}
