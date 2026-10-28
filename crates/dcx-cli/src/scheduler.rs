use std::path::Path;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{chat, routines};

pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        thread::Builder::new()
            .name("dcx-routine-scheduler".to_owned())
            .spawn(|| {
                loop {
                    tick();
                    thread::sleep(Duration::from_secs(15));
                }
            })
            .expect("não foi possível iniciar scheduler de rotinas");
    });
}

fn tick() {
    for workspace in routines::scheduled_workspaces() {
        match routines::claim_due(&workspace, now_ms()) {
            Ok(due) => {
                for routine in due {
                    let workspace = workspace.clone();
                    thread::spawn(move || {
                        let result = execute(&workspace, &routine);
                        if let Err(error) = routines::finish_run(&workspace, &routine.id, result) {
                            eprintln!("Não foi possível atualizar rotina {}: {error}", routine.id);
                        }
                    });
                }
            }
            Err(error) => eprintln!(
                "Não foi possível verificar rotinas em {}: {error}",
                workspace.display()
            ),
        }
    }
}

fn execute(workspace: &Path, routine: &routines::Routine) -> Result<Option<f64>, String> {
    crate::env_file::load_for_workspace(workspace)?;
    let mut session = chat::open_session(workspace, None)?;
    let message = format!("Rotina agendada: {}\n\n{}", routine.title, routine.prompt);
    let reply = chat::send_message_with_events(workspace, &mut session, &message, &mut |_| {})?;
    Ok(reply.cost_usd)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
