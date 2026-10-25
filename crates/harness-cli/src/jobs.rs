use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const JOB_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunJob {
    pub schema_version: u32,
    pub run_id: String,
    pub session_id: String,
    pub workspace: PathBuf,
    pub request: String,
    pub status: String,
    pub created_at_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobEvent {
    pub event: String,
    pub data: serde_json::Value,
    pub created_at_unix_ms: u64,
}

pub fn enqueue(workspace: &Path, session_id: &str, request: &str) -> Result<RunJob, String> {
    let run_id = format!("api-{}-{}", now_ms(), std::process::id());
    let job = RunJob {
        schema_version: JOB_SCHEMA_VERSION,
        run_id: run_id.clone(),
        session_id: session_id.to_owned(),
        workspace: workspace.to_path_buf(),
        request: request.to_owned(),
        status: "queued".to_owned(),
        created_at_unix_ms: now_ms(),
    };
    let directory = jobs_root(workspace);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("não foi possível criar {}: {error}", directory.display()))?;
    let path = directory.join(format!("{run_id}.json"));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&job).map_err(|error| format!("job inválido: {error}"))?,
    )
    .map_err(|error| format!("não foi possível salvar job {}: {error}", path.display()))?;
    append_event(
        workspace,
        &run_id,
        "queued",
        serde_json::json!({"run_id": run_id}),
    )?;
    Ok(job)
}

pub fn append_event(
    workspace: &Path,
    run_id: &str,
    event: &str,
    data: serde_json::Value,
) -> Result<(), String> {
    let path = jobs_root(workspace).join(format!("{run_id}.events.jsonl"));
    let record = JobEvent {
        event: event.to_owned(),
        data,
        created_at_unix_ms: now_ms(),
    };
    let line =
        serde_json::to_string(&record).map_err(|error| format!("evento inválido: {error}"))?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("não foi possível abrir eventos {}: {error}", path.display()))?;
    writeln!(file, "{line}").map_err(|error| format!("não foi possível gravar evento: {error}"))
}

pub fn events(workspace: &Path, run_id: &str) -> Result<Vec<JobEvent>, String> {
    let path = jobs_root(workspace).join(format!("{run_id}.events.jsonl"));
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("não foi possível ler eventos {}: {error}", path.display()))?;
    text.lines()
        .map(|line| serde_json::from_str(line).map_err(|error| format!("evento inválido: {error}")))
        .collect()
}

pub fn load(workspace: &Path, run_id: &str) -> Result<RunJob, String> {
    if run_id.is_empty()
        || run_id.len() > 128
        || !run_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err(format!("run_id inválido: {run_id}"));
    }
    let path = jobs_root(workspace).join(format!("{run_id}.json"));
    let bytes = fs::read(&path).map_err(|error| format!("job {run_id} não encontrado: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("job {} inválido: {error}", path.display()))
}

fn jobs_root(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("api-runs")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_and_load_job() {
        let workspace = std::env::temp_dir().join(format!("harness-job-{}", now_ms()));
        let job = enqueue(&workspace, "chat-1", "pedido").expect("enfileirar");
        let loaded = load(&workspace, &job.run_id).expect("carregar");
        assert_eq!(loaded.status, "queued");
        assert_eq!(loaded.session_id, "chat-1");
        fs::remove_dir_all(workspace).expect("limpar");
    }
}
