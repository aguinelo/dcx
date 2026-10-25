use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
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

fn inputs() -> &'static Mutex<InputMap> {
    static INPUTS: OnceLock<Mutex<InputMap>> = OnceLock::new();
    INPUTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn processes() -> &'static Mutex<ProcessMap> {
    static PROCESSES: OnceLock<Mutex<ProcessMap>> = OnceLock::new();
    PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_running(run_id: &str) -> bool {
    processes()
        .lock()
        .map(|map| map.contains_key(run_id))
        .unwrap_or(false)
}

pub fn start(workspace: &Path, run_id: &str, session_id: &str) -> Result<(), String> {
    let workspace = workspace.to_path_buf();
    let run_id = run_id.to_owned();
    let session_id = session_id.to_owned();
    jobs::set_status(&workspace, &run_id, "running")?;
    jobs::append_event(&workspace, &run_id, "running", serde_json::json!({}))?;
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
    });
    Ok(())
}

fn execute(workspace: &Path, run_id: &str, session_id: &str) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(executable)
        .args(["run", "--from-session", session_id])
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("não foi possível iniciar worker: {error}"))?;
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
            let _ = jobs::set_status(workspace, &job.run_id, "failed");
            let _ = jobs::append_event(
                workspace,
                &job.run_id,
                "recovered_as_failed",
                serde_json::json!({"reason": "daemon reiniciado durante a execução"}),
            );
        }
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
    #[test]
    fn worker_input_requires_a_running_process() {
        assert!(super::send_input("missing", "s").is_err());
    }
}
