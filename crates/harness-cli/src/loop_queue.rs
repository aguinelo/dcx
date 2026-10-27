use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_TASKS: usize = 25;
const MAX_REQUEST_CHARS: usize = 12_000;

#[derive(Deserialize)]
struct Manifest {
    tasks: Vec<ManifestTask>,
}

#[derive(Deserialize)]
struct ManifestTask {
    id: String,
    request: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LoopTask {
    id: String,
    request: String,
    status: String,
    run_id: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LoopState {
    schema_version: u32,
    id: String,
    workspace: PathBuf,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
    current_task: usize,
    tasks: Vec<LoopTask>,
}

pub fn start(manifest_path: &Path) -> Result<(), String> {
    let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
    let bytes = fs::read(manifest_path)
        .map_err(|error| format!("não foi possível ler {}: {error}", manifest_path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("manifesto da fila inválido: {error}"))?;
    validate_manifest(&manifest)?;
    let now = now_ms();
    let id = format!("loop-{now}-{}", std::process::id());
    let tasks = manifest
        .tasks
        .into_iter()
        .map(|task| LoopTask {
            id: task.id,
            request: task.request.trim().to_owned(),
            status: "queued".to_owned(),
            run_id: None,
            error: None,
        })
        .collect();
    let state = LoopState {
        schema_version: 1,
        id: id.clone(),
        workspace,
        created_at_unix_ms: now,
        updated_at_unix_ms: now,
        current_task: 0,
        tasks,
    };
    save(&state)?;
    println!("Fila finita criada: {id} ({} tarefas)", state.tasks.len());
    execute(state)
}

pub fn resume(id: &str) -> Result<(), String> {
    validate_id(id)?;
    let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
    let path = state_path(&workspace, id);
    let bytes = fs::read(&path)
        .map_err(|error| format!("fila {id} não encontrada em {}: {error}", path.display()))?;
    let state: LoopState = serde_json::from_slice(&bytes)
        .map_err(|error| format!("estado da fila inválido: {error}"))?;
    if state.workspace != workspace {
        return Err("a fila só pode ser retomada no workspace em que foi criada".to_owned());
    }
    execute(state)
}

fn execute(mut state: LoopState) -> Result<(), String> {
    while state.current_task < state.tasks.len() {
        let index = state.current_task;
        let task_id = state.tasks[index].id.clone();
        println!(
            "\nFila {} — tarefa {}/{}: {}",
            state.id,
            index + 1,
            state.tasks.len(),
            task_id
        );
        state.tasks[index].status = "running".to_owned();
        state.tasks[index].error = None;
        state.updated_at_unix_ms = now_ms();
        save(&state)?;

        let task_request = state.tasks[index].request.clone();
        let existing_run = state.tasks[index].run_id.clone().filter(|run_id| {
            state
                .workspace
                .join(".harness")
                .join("runs")
                .join(run_id)
                .join("contract_bundle.json")
                .is_file()
        });
        let result = if let Some(run_id) = existing_run {
            println!("Retomando o run {run_id} da tarefa {task_id}");
            crate::interactive::resume(&run_id).map_err(|error| error.to_string())
        } else {
            crate::interactive::run_tracked(&task_request, |run_id| {
                state.tasks[index].run_id = Some(run_id.to_owned());
                state.updated_at_unix_ms = now_ms();
                save(&state)?;
                println!("Run {} associado à tarefa {task_id}", run_id);
                Ok(())
            })
            .map_err(|error| error.to_string())
        };

        match result {
            Ok(()) => {
                state.tasks[index].status = "done".to_owned();
                state.current_task += 1;
                state.updated_at_unix_ms = now_ms();
                save(&state)?;
            }
            Err(error) => {
                state.tasks[index].status = "paused".to_owned();
                state.tasks[index].error = Some(error.chars().take(1000).collect());
                state.updated_at_unix_ms = now_ms();
                save(&state)?;
                return Err(format!(
                    "fila pausada na tarefa {}. Resolva o bloqueio e retome com `harness loop resume --id {}`",
                    state.tasks[index].id, state.id
                ));
            }
        }
    }
    println!(
        "Fila {} concluída: {} tarefa(s).",
        state.id,
        state.tasks.len()
    );
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.tasks.is_empty() || manifest.tasks.len() > MAX_TASKS {
        return Err(format!("a fila precisa ter entre 1 e {MAX_TASKS} tarefas"));
    }
    let mut ids = std::collections::HashSet::new();
    for task in &manifest.tasks {
        validate_id(&task.id)?;
        if !ids.insert(&task.id) {
            return Err(format!("id de tarefa duplicado: {}", task.id));
        }
        if task.request.trim().is_empty() || task.request.chars().count() > MAX_REQUEST_CHARS {
            return Err(format!(
                "pedido da tarefa {} deve ter entre 1 e {MAX_REQUEST_CHARS} caracteres",
                task.id
            ));
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
    {
        return Err(format!("id inválido: {id}"));
    }
    Ok(())
}

fn save(state: &LoopState) -> Result<(), String> {
    let path = state_path(&state.workspace, &state.id);
    let parent = path.parent().ok_or("estado da fila sem diretório pai")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(".{}.tmp", state.id));
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

fn state_path(workspace: &Path, id: &str) -> PathBuf {
    workspace
        .join(".harness")
        .join("loops")
        .join(format!("{id}.json"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
