mod claude;
mod codex;
mod demo;
mod events;
mod interactive;
mod providers;
mod schemas;
mod state;
mod workspace;

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
            Some("--job-02") if args.next().is_none() => {
                if let Err(error) = demo::run_job02() {
                    eprintln!("Falha no job-02: {error}");
                    std::process::exit(1);
                }
            }
            Some("--request") => {
                let request = args.collect::<Vec<_>>().join(" ");
                if request.trim().is_empty() {
                    eprintln!("Uso: harness run --request <pedido>");
                    std::process::exit(2);
                }
                if let Err(error) = interactive::run(&request) {
                    eprintln!("Falha na preparação da tarefa: {error}");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Uso: harness run < --demo | --pilot | --request <pedido> >");
                std::process::exit(2);
            }
        },
        Some("state") => match args.next().as_deref() {
            Some("prune") => {
                if let Err(error) = state::run_prune(args) {
                    eprintln!("Falha ao limpar o estado: {error}");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Uso: harness state prune --older-than-days <N> [--dry-run]");
                std::process::exit(2);
            }
        },
        Some("--version" | "-V") if args.next().is_none() => {
            println!("harness {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!(
                "Uso: harness <states|schemas --write|--check [DIR]|state prune --older-than-days <N> [--dry-run]|--version|run --demo|--pilot|--job-02|--request <pedido>>"
            );
            std::process::exit(2);
        }
    }
}
