use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use crate::jobs;

struct InputHandle {
    stdin: ChildStdin,
    workspace: std::path::PathBuf,
}

type InputMap = HashMap<String, Arc<Mutex<InputHandle>>>;
type ProcessMap = HashMap<String, Arc<Mutex<std::process::Child>>>;
type ActiveSet = HashSet<String>;

fn inputs() -> &'static Mutex<InputMap> {
    static INPUTS: OnceLock<Mutex<InputMap>> = OnceLock::new();
    INPUTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn processes() -> &'static Mutex<ProcessMap> {
    static PROCESSES: OnceLock<Mutex<ProcessMap>> = OnceLock::new();
    PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_runs() -> &'static Mutex<ActiveSet> {
    static ACTIVE: OnceLock<Mutex<ActiveSet>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

fn recovery_lock() -> &'static Mutex<()> {
    static RECOVERY: OnceLock<Mutex<()>> = OnceLock::new();
    RECOVERY.get_or_init(|| Mutex::new(()))
}

fn is_running(run_id: &str) -> bool {
    active_runs()
        .lock()
        .map(|set| set.contains(run_id))
        .unwrap_or(false)
}

pub fn start(workspace: &Path, run_id: &str, session_id: &str) -> Result<(), String> {
    let workspace = workspace.to_path_buf();
    let run_id = run_id.to_owned();
    let session_id = session_id.to_owned();
    {
        let mut active = active_runs()
            .lock()
            .map_err(|_| "lock de workers ativos indisponível".to_owned())?;
        if !active.insert(run_id.clone()) {
            return Ok(());
        }
    }
    if let Err(error) = jobs::set_status(&workspace, &run_id, "running") {
        active_runs()
            .lock()
            .ok()
            .map(|mut active| active.remove(&run_id));
        return Err(error);
    }
    if let Err(error) = jobs::append_event(&workspace, &run_id, "running", serde_json::json!({})) {
        active_runs()
            .lock()
            .ok()
            .map(|mut active| active.remove(&run_id));
        return Err(error);
    }
    thread::spawn(move || {
        let result = execute(&workspace, &run_id, &session_id);
        if let Err(error) = result {
            let _ = jobs::append_event(
                &workspace,
                &run_id,
                "error",
                serde_json::json!({"message": error}),
            );
            let _ = jobs::set_status(&workspace, &run_id, "failed");
        }
        active_runs()
            .lock()
            .ok()
            .map(|mut active| active.remove(&run_id));
    });
    Ok(())
}

fn execute(workspace: &Path, run_id: &str, session_id: &str) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let saved_run = workspace
        .join(".harness")
        .join("runs")
        .join(run_id)
        .join("contract_bundle.json")
        .is_file();
    let mut command = Command::new(executable);
    if saved_run {
        command.args(["resume", "--run", run_id]);
    } else {
        command
            .args(["run", "--from-session", session_id])
            .env("HARNESS_RUN_ID", run_id);
    }
    command
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| format!("não foi possível iniciar worker: {error}"))?;
    if let Err(error) = jobs::set_worker_pid(workspace, run_id, Some(child.id()), false) {
        let _ = child.kill();
        return Err(error);
    }
    let stdin = child.stdin.take().ok_or("worker sem stdin")?;
    let stdout = child.stdout.take().ok_or("worker sem stdout")?;
    let stderr = child.stderr.take().ok_or("worker sem stderr")?;
    let child = Arc::new(Mutex::new(child));
    inputs()
        .lock()
        .map_err(|_| "lock de inputs indisponível".to_owned())?
        .insert(
            run_id.to_owned(),
            Arc::new(Mutex::new(InputHandle {
                stdin,
                workspace: workspace.to_path_buf(),
            })),
        );
    processes()
        .lock()
        .map_err(|_| "lock de processos indisponível".to_owned())?
        .insert(run_id.to_owned(), child.clone());
    let workspace_for_stdout = workspace.to_path_buf();
    let run_for_stdout = run_id.to_owned();
    let stdout_thread = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = jobs::append_event(
                &workspace_for_stdout,
                &run_for_stdout,
                "output",
                serde_json::json!({"text": line}),
            );
            if line.contains("Pergunta bloqueante:")
                || line.contains("Aprovar contrato?")
                || line.contains("O que precisa mudar no contrato?")
            {
                let _ = jobs::set_status(&workspace_for_stdout, &run_for_stdout, "awaiting_input");
                let _ = jobs::append_event(
                    &workspace_for_stdout,
                    &run_for_stdout,
                    "awaiting_input",
                    serde_json::json!({"text": line}),
                );
            }
        }
    });
    let workspace_for_stderr = workspace.to_path_buf();
    let run_for_stderr = run_id.to_owned();
    let stderr_thread = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = jobs::append_event(
                &workspace_for_stderr,
                &run_for_stderr,
                "diagnostic",
                serde_json::json!({"text": line}),
            );
        }
    });
    let status = child
        .lock()
        .map_err(|_| "processo do worker indisponível".to_owned())?
        .wait()
        .map_err(|error| error.to_string())?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    inputs().lock().ok().map(|mut map| map.remove(run_id));
    processes().lock().ok().map(|mut map| map.remove(run_id));
    jobs::set_worker_pid(workspace, run_id, None, false)?;
    let cancelled = jobs::load(workspace, run_id)
        .map(|job| job.status == "cancelled")
        .unwrap_or(false);
    if cancelled {
        jobs::append_event(workspace, run_id, "cancelled", serde_json::json!({}))?;
    } else if status.success() {
        jobs::set_status(workspace, run_id, "done")?;
        jobs::append_event(workspace, run_id, "done", serde_json::json!({}))?;
    } else {
        jobs::set_status(workspace, run_id, "failed")?;
        jobs::append_event(
            workspace,
            run_id,
            "failed",
            serde_json::json!({"exit_status": status.code()}),
        )?;
    }
    Ok(())
}

pub fn cancel(workspace: &Path, run_id: &str) -> Result<(), String> {
    let process = processes()
        .lock()
        .map_err(|_| "lock de processos indisponível".to_owned())?
        .get(run_id)
        .cloned()
        .ok_or_else(|| format!("run {run_id} não está em execução"))?;
    process
        .lock()
        .map_err(|_| "processo do worker indisponível".to_owned())?
        .kill()
        .map_err(|error| format!("não foi possível cancelar run: {error}"))?;
    jobs::set_status(workspace, run_id, "cancelled")?;
    jobs::append_event(workspace, run_id, "cancel_requested", serde_json::json!({}))
}

pub fn recover(workspace: &Path) {
    let Ok(_recovery_guard) = recovery_lock().lock() else {
        return;
    };
    let root = workspace.join(".harness").join("api-runs");
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(job) =
            serde_json::from_slice::<jobs::RunJob>(&std::fs::read(&path).unwrap_or_default())
        else {
            continue;
        };
        if job.status == "queued" {
            let _ = start(workspace, &job.run_id, &job.session_id);
        } else if (job.status == "running" || job.status == "awaiting_input")
            && !is_running(&job.run_id)
        {
            if job.restart_count >= 3 {
                let _ = jobs::set_status(workspace, &job.run_id, "failed");
                let _ = jobs::append_event(
                    workspace,
                    &job.run_id,
                    "recovery_stopped",
                    serde_json::json!({"reason": "limite de 3 retomadas após reinício atingido"}),
                );
                continue;
            }
            if stop_orphan_worker(&job).is_err() {
                let _ = jobs::set_status(workspace, &job.run_id, "failed");
                let _ = jobs::append_event(
                    workspace,
                    &job.run_id,
                    "recovery_failed",
                    serde_json::json!({"reason": "não foi possível encerrar o worker anterior"}),
                );
                continue;
            }
            let _ = jobs::set_worker_pid(workspace, &job.run_id, None, true);
            let _ = jobs::append_event(
                workspace,
                &job.run_id,
                "recovered",
                serde_json::json!({
                    "restart_count": job.restart_count.saturating_add(1),
                    "mode": if workspace.join(".harness").join("runs").join(&job.run_id).join("contract_bundle.json").is_file() { "resume" } else { "restart_session" }
                }),
            );
            let _ = start(workspace, &job.run_id, &job.session_id);
        }
    }
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    let pid_text = pid.to_string();
    let Ok(output) = Command::new("ps")
        .args(["-p", &pid_text, "-o", "stat="])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stat = String::from_utf8_lossy(&output.stdout);
    let stat = stat.trim();
    !stat.is_empty() && !stat.starts_with('Z')
}

fn stop_orphan_worker(job: &jobs::RunJob) -> Result<(), String> {
    let Some(pid) = job.worker_pid else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        let pid_text = pid.to_string();
        let output = Command::new("ps")
            .args(["-p", &pid_text, "-o", "command="])
            .output()
            .map_err(|error| format!("não foi possível verificar worker antigo: {error}"))?;
        let command = String::from_utf8_lossy(&output.stdout);
        if !output.status.success()
            || !(command.contains(&job.session_id) || command.contains(&job.run_id))
        {
            return Ok(());
        }
        let group = format!("-{pid}");
        let _ = Command::new("kill").args(["-TERM", &group]).status();
        for _ in 0..30 {
            if !process_is_running(pid) {
                return Ok(());
            }
            thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = Command::new("kill").args(["-KILL", &group]).status();
        if process_is_running(pid) {
            return Err(format!("worker antigo {pid} continua ativo"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err("retomada automática após reinício ainda não é suportada nesta plataforma".to_owned())
    }
}

pub fn send_input(run_id: &str, input: &str) -> Result<(), String> {
    let handle = inputs()
        .lock()
        .map_err(|_| "lock de inputs indisponível".to_owned())?
        .get(run_id)
        .cloned()
        .ok_or_else(|| format!("run {run_id} não está aguardando entrada"))?;
    let mut handle = handle
        .lock()
        .map_err(|_| "stdin do worker indisponível".to_owned())?;
    writeln!(handle.stdin, "{input}")
        .map_err(|error| format!("não foi possível enviar entrada: {error}"))?;
    handle
        .stdin
        .flush()
        .map_err(|error| format!("não foi possível enviar entrada: {error}"))?;
    jobs::set_status(&handle.workspace, run_id, "running")?;
    jobs::append_event(
        &handle.workspace,
        run_id,
        "input_received",
        serde_json::json!({"text": input}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_input_requires_a_running_process() {
        assert!(send_input("missing", "s").is_err());
    }

    #[test]
    fn recovery_stops_after_three_restarts() {
        let workspace = std::env::temp_dir().join(format!(
            "harness-worker-recovery-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        let job = jobs::enqueue(&workspace, "chat-1", "pedido").expect("enfileirar");
        jobs::set_status(&workspace, &job.run_id, "running").expect("marcar running");
        for _ in 0..3 {
            jobs::set_worker_pid(&workspace, &job.run_id, None, true).expect("contar reinício");
        }

        recover(&workspace);

        let loaded = jobs::load(&workspace, &job.run_id).expect("carregar");
        assert_eq!(loaded.status, "failed");
        let events = jobs::events(&workspace, &job.run_id).expect("eventos");
        assert!(events.iter().any(|event| event.event == "recovery_stopped"));
        std::fs::remove_dir_all(workspace).expect("limpar");
    }
}
