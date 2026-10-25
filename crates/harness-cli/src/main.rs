mod chat;
mod claude;
mod codex;
mod demo;
mod env_file;
mod events;
mod interactive;
mod jobs;
mod lock;
mod metrics;
mod providers;
mod schemas;
mod serve;
mod state;
mod workspace;

use harness_engine::RunState;

fn main() {
    if let Err(error) = env_file::load() {
        eprintln!("Falha ao carregar o ambiente: {error}");
        std::process::exit(1);
    }
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
        Some("chat") => {
            let mut workspace = None;
            let mut session = None;
            let mut list = false;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--workspace" => {
                        let Some(value) = args.next() else {
                            eprintln!(
                                "Uso: harness chat [--workspace <dir>] [--session <id> | --list]"
                            );
                            std::process::exit(2);
                        };
                        workspace = Some(std::path::PathBuf::from(value));
                    }
                    "--session" => {
                        let Some(value) = args.next() else {
                            eprintln!(
                                "Uso: harness chat [--workspace <dir>] [--session <id> | --list]"
                            );
                            std::process::exit(2);
                        };
                        session = Some(value);
                    }
                    "--list" => list = true,
                    _ => {
                        eprintln!(
                            "Uso: harness chat [--workspace <dir>] [--session <id> | --list]"
                        );
                        std::process::exit(2);
                    }
                }
            }
            if list && session.is_some() {
                eprintln!("Uso: harness chat --list não aceita --session");
                std::process::exit(2);
            }
            let result = if list {
                chat::list(workspace)
            } else {
                chat::run(workspace, session)
            };
            if let Err(error) = result {
                eprintln!("Falha no chat: {error}");
                std::process::exit(1);
            }
        }
        Some("serve") => {
            let mut bind = "127.0.0.1:8787".to_owned();
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--bind" => bind = args.next().unwrap_or_default(),
                    _ => {
                        eprintln!("Uso: harness serve [--bind <host:porta>]");
                        std::process::exit(2);
                    }
                }
            }
            if bind.is_empty() {
                eprintln!("Uso: harness serve [--bind <host:porta>]");
                std::process::exit(2);
            }
            if let Err(error) = serve::run(&bind) {
                eprintln!("Falha no servidor: {error}");
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
            Some("--from-session") => {
                let session_id = args.next().unwrap_or_default();
                if session_id.trim().is_empty() || args.next().is_some() {
                    eprintln!("Uso: harness run --from-session <session_id>");
                    std::process::exit(2);
                }
                let workspace = match std::env::current_dir() {
                    Ok(workspace) => workspace,
                    Err(error) => {
                        eprintln!("Falha ao obter o projeto atual: {error}");
                        std::process::exit(1);
                    }
                };
                let request = match chat::request_from_session(&workspace, &session_id) {
                    Ok(request) => request,
                    Err(error) => {
                        eprintln!("Falha ao promover a sessão: {error}");
                        std::process::exit(1);
                    }
                };
                println!(
                    "Sessão promovida para um novo run; a aprovação do contrato continua obrigatória."
                );
                if let Err(error) = interactive::run(&request) {
                    eprintln!("Falha na preparação da tarefa: {error}");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!(
                    "Uso: harness run < --demo | --pilot | --request <pedido> | --from-session <session_id> >"
                );
                std::process::exit(2);
            }
        },
        Some("resume") => match args.next().as_deref() {
            Some("--run") => {
                let run_id = args.next().unwrap_or_default();
                if run_id.trim().is_empty() || args.next().is_some() {
                    eprintln!("Uso: harness resume --run <run_id>");
                    std::process::exit(2);
                }
                if let Err(error) = interactive::resume(&run_id) {
                    eprintln!("Falha ao retomar o run: {error}");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Uso: harness resume --run <run_id>");
                std::process::exit(2);
            }
        },
        Some("dispute") => {
            let mut run_id = None;
            let mut sensor_id = None;
            let mut reason = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--run" => run_id = args.next(),
                    "--sensor" => sensor_id = args.next(),
                    "--reason" => {
                        reason = Some(args.collect::<Vec<_>>().join(" "));
                        break;
                    }
                    _ => {
                        eprintln!(
                            "Uso: harness dispute --run <run_id> --sensor <sensor_id> --reason <motivo>"
                        );
                        std::process::exit(2);
                    }
                }
            }
            let (Some(run_id), Some(sensor_id), Some(reason)) = (run_id, sensor_id, reason) else {
                eprintln!(
                    "Uso: harness dispute --run <run_id> --sensor <sensor_id> --reason <motivo>"
                );
                std::process::exit(2);
            };
            if run_id.trim().is_empty() || sensor_id.trim().is_empty() || reason.trim().is_empty() {
                eprintln!(
                    "Uso: harness dispute --run <run_id> --sensor <sensor_id> --reason <motivo>"
                );
                std::process::exit(2);
            }
            if let Err(error) = interactive::dispute(&run_id, &sensor_id, &reason) {
                eprintln!("Falha ao resolver a disputa: {error}");
                std::process::exit(1);
            }
        }
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
                "Uso: harness <states|schemas --write|--check [DIR]|chat [--workspace <dir>] [--session <id>|--list]|serve [--bind <host:porta>]|resume --run <run_id>|dispute --run <id> ...|state prune ...|--version|run ...>"
            );
            std::process::exit(2);
        }
    }
}
