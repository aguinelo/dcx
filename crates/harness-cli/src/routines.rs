use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_TITLE_CHARS: usize = 100;
const MAX_PROMPT_CHARS: usize = 12_000;
const MAX_ROUTINES: usize = 100;
const ALLOWED_INTERVALS_HOURS: [u16; 5] = [1, 6, 12, 24, 168];
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Routine {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    #[serde(default)]
    pub interval_hours: Option<u16>,
    #[serde(default)]
    pub next_run_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub last_run_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub last_run_status: Option<String>,
    #[serde(default)]
    pub last_run_error: Option<String>,
    #[serde(default)]
    pub last_cost_usd: Option<f64>,
}

pub fn list(workspace: &Path) -> Result<Vec<Routine>, String> {
    let _guard = store_lock()
        .lock()
        .map_err(|_| "lock de rotinas indisponível".to_owned())?;
    list_unlocked(workspace)
}

fn list_unlocked(workspace: &Path) -> Result<Vec<Routine>, String> {
    let path = routines_path(workspace);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return Ok(Vec::new());
    };
    if !metadata.file_type().is_file() {
        return Err("arquivo de rotinas precisa ser um arquivo regular".to_owned());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("não foi possível ler rotinas: {error}"))?;
    let mut routines: Vec<Routine> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("arquivo de rotinas inválido: {error}"))?;
    routines.sort_by_key(|routine| routine.title.to_lowercase());
    Ok(routines)
}

pub fn save(
    workspace: &Path,
    id: Option<&str>,
    title: &str,
    prompt: &str,
    interval_hours: Option<u16>,
) -> Result<Routine, String> {
    let title = title.trim();
    let prompt = prompt.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!(
            "nome da rotina deve ter entre 1 e {MAX_TITLE_CHARS} caracteres"
        ));
    }
    if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(format!(
            "instruções devem ter entre 1 e {MAX_PROMPT_CHARS} caracteres"
        ));
    }
    if interval_hours.is_some_and(|hours| !ALLOWED_INTERVALS_HOURS.contains(&hours)) {
        return Err("intervalo deve ser 1, 6, 12, 24 ou 168 horas".to_owned());
    }

    let _guard = store_lock()
        .lock()
        .map_err(|_| "lock de rotinas indisponível".to_owned())?;
    let mut routines = list_unlocked(workspace)?;
    let now = now_ms();
    if let Some(id) = id {
        validate_id(id)?;
        let Some(existing) = routines.iter_mut().find(|routine| routine.id == id) else {
            return Err("rotina não encontrada".to_owned());
        };
        existing.title = title.to_owned();
        existing.prompt = prompt.to_owned();
        existing.updated_at_unix_ms = now;
        if existing.interval_hours != interval_hours {
            existing.interval_hours = interval_hours;
            existing.next_run_at_unix_ms = interval_hours.map(|hours| next_run(now, hours));
            if interval_hours.is_none() {
                existing.last_run_status = None;
                existing.last_run_error = None;
            }
        }
        let saved = existing.clone();
        write(workspace, &routines)?;
        return Ok(saved);
    }

    if routines.len() >= MAX_ROUTINES {
        return Err(format!(
            "workspace pode ter no máximo {MAX_ROUTINES} rotinas"
        ));
    }

    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let id = format!("routine-{now}-{}-{sequence}", std::process::id());
    let routine = Routine {
        id,
        title: title.to_owned(),
        prompt: prompt.to_owned(),
        created_at_unix_ms: now,
        updated_at_unix_ms: now,
        interval_hours,
        next_run_at_unix_ms: interval_hours.map(|hours| next_run(now, hours)),
        last_run_at_unix_ms: None,
        last_run_status: None,
        last_run_error: None,
        last_cost_usd: None,
    };
    routines.push(routine.clone());
    write(workspace, &routines)?;
    Ok(routine)
}

pub fn delete(workspace: &Path, id: &str) -> Result<(), String> {
    validate_id(id)?;
    let _guard = store_lock()
        .lock()
        .map_err(|_| "lock de rotinas indisponível".to_owned())?;
    let mut routines = list_unlocked(workspace)?;
    routines.retain(|routine| routine.id != id);
    write(workspace, &routines)
}

pub fn claim_due(workspace: &Path, now: u64) -> Result<Vec<Routine>, String> {
    let _guard = store_lock()
        .lock()
        .map_err(|_| "lock de rotinas indisponível".to_owned())?;
    let mut routines = list_unlocked(workspace)?;
    let mut due = Vec::new();
    for routine in &mut routines {
        let (Some(interval), Some(next)) = (routine.interval_hours, routine.next_run_at_unix_ms)
        else {
            continue;
        };
        if next <= now {
            routine.next_run_at_unix_ms = Some(next_run(now, interval));
            routine.last_run_at_unix_ms = Some(now);
            routine.last_run_status = Some("running".to_owned());
            routine.last_run_error = None;
            routine.last_cost_usd = None;
            due.push(routine.clone());
        }
    }
    if !due.is_empty() {
        write(workspace, &routines)?;
    }
    Ok(due)
}

pub fn finish_run(
    workspace: &Path,
    id: &str,
    result: Result<Option<f64>, String>,
) -> Result<(), String> {
    validate_id(id)?;
    let _guard = store_lock()
        .lock()
        .map_err(|_| "lock de rotinas indisponível".to_owned())?;
    let mut routines = list_unlocked(workspace)?;
    let routine = routines
        .iter_mut()
        .find(|routine| routine.id == id)
        .ok_or_else(|| "rotina não encontrada".to_owned())?;
    match result {
        Ok(cost) => {
            routine.last_run_status = Some("done".to_owned());
            routine.last_run_error = None;
            routine.last_cost_usd = cost;
        }
        Err(error) => {
            routine.last_run_status = Some("failed".to_owned());
            routine.last_run_error = Some(error.chars().take(500).collect());
            routine.last_cost_usd = None;
        }
    }
    write(workspace, &routines)
}

pub fn scheduled_workspaces() -> Vec<PathBuf> {
    let registry = registry_path();
    let Ok(bytes) = fs::read(registry) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn register_workspace(workspace: &Path) -> Result<(), String> {
    let workspace = fs::canonicalize(workspace)
        .map_err(|error| format!("não foi possível resolver workspace do scheduler: {error}"))?;
    let _guard = registry_lock()
        .lock()
        .map_err(|_| "lock do registry de rotinas indisponível".to_owned())?;
    let mut workspaces = scheduled_workspaces();
    if !workspaces.contains(&workspace) {
        workspaces.push(workspace);
        let path = registry_path();
        let parent = path.parent().expect("registry tem diretório pai");
        fs::create_dir_all(parent)
            .map_err(|error| format!("não foi possível criar state do scheduler: {error}"))?;
        let temporary = parent.join("routine-workspaces.json.tmp");
        let bytes = serde_json::to_vec_pretty(&workspaces)
            .map_err(|error| format!("não foi possível serializar workspaces: {error}"))?;
        fs::write(&temporary, bytes)
            .map_err(|error| format!("não foi possível salvar workspaces agendados: {error}"))?;
        fs::rename(temporary, path)
            .map_err(|error| format!("não foi possível publicar workspaces agendados: {error}"))?;
    }
    Ok(())
}

fn next_run(now: u64, interval_hours: u16) -> u64 {
    now.saturating_add(u64::from(interval_hours) * 60 * 60 * 1000)
}

fn store_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn registry_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn registry_path() -> PathBuf {
    let root = if let Some(directory) = std::env::var_os("HARNESS_STATE_DIR") {
        PathBuf::from(directory)
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".local/state/harness")
    } else {
        std::env::temp_dir().join("harness-state")
    };
    root.join("routine-workspaces.json")
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err("id de rotina inválido".to_owned());
    }
    Ok(())
}

fn write(workspace: &Path, routines: &[Routine]) -> Result<(), String> {
    let path = routines_path(workspace);
    let directory = path.parent().expect("caminho de rotinas tem diretório pai");
    fs::create_dir_all(directory)
        .map_err(|error| format!("não foi possível criar diretório de rotinas: {error}"))?;
    let temporary = directory.join("routines.json.tmp");
    let bytes = serde_json::to_vec_pretty(routines)
        .map_err(|error| format!("não foi possível serializar rotinas: {error}"))?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("não foi possível salvar rotinas: {error}"))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("não foi possível publicar rotinas: {error}"))
}

fn routines_path(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("routines.json")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace() -> PathBuf {
        std::env::temp_dir().join(format!(
            "harness-routines-{}-{}-{}",
            std::process::id(),
            now_ms(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn routines_can_be_created_updated_and_deleted() {
        let workspace = temp_workspace();
        let created =
            save(&workspace, None, "Revisar PR", "Revise o PR atual.", None).expect("criar");
        let another = save(
            &workspace,
            None,
            "Revisar issues",
            "Liste as issues abertas.",
            None,
        )
        .expect("criar segunda rotina");
        assert_ne!(created.id, another.id);
        assert_eq!(list(&workspace).expect("listar").len(), 2);
        let updated = save(
            &workspace,
            Some(&created.id),
            "Revisar diff",
            "Analise o diff.",
            None,
        )
        .expect("atualizar");
        assert_eq!(updated.created_at_unix_ms, created.created_at_unix_ms);
        assert_eq!(updated.title, "Revisar diff");
        delete(&workspace, &created.id).expect("remover");
        assert_eq!(list(&workspace).expect("listar uma rotina").len(), 1);
        delete(&workspace, &another.id).expect("remover segunda rotina");
        assert!(list(&workspace).expect("listar vazio").is_empty());
        fs::remove_dir_all(workspace).expect("limpar workspace temporário");
    }

    #[test]
    fn routines_reject_empty_content_and_unsafe_ids() {
        let workspace = temp_workspace();
        assert!(save(&workspace, None, " ", "prompt", None).is_err());
        assert!(save(&workspace, None, "Nome", " ", None).is_err());
        assert!(save(&workspace, None, "Nome", "prompt", Some(2)).is_err());
        assert!(delete(&workspace, "../outside").is_err());
    }

    #[test]
    fn existing_routine_files_without_schedule_fields_still_load() {
        let workspace = temp_workspace();
        let path = routines_path(&workspace);
        fs::create_dir_all(path.parent().expect("pai")).expect("criar harness");
        fs::write(
            &path,
            r#"[{"id":"routine-old","title":"Antiga","prompt":"Prompt antigo","created_at_unix_ms":1,"updated_at_unix_ms":2}]"#,
        )
        .expect("escrever rotina antiga");
        let routines = list(&workspace).expect("carregar rotina legada");
        assert_eq!(routines[0].interval_hours, None);
        assert_eq!(routines[0].title, "Antiga");
        fs::remove_dir_all(workspace).expect("limpar workspace temporário");
    }

    #[test]
    fn scheduled_routines_are_claimed_once_and_record_completion() {
        let workspace = temp_workspace();
        let created = save(&workspace, None, "Resumo", "Resuma atualizações.", Some(1))
            .expect("criar agendamento");
        let due_at = created.next_run_at_unix_ms.expect("próxima execução");
        assert!(
            claim_due(&workspace, due_at - 1)
                .expect("ainda não venceu")
                .is_empty()
        );
        let claimed = claim_due(&workspace, due_at).expect("claim");
        assert_eq!(claimed.len(), 1);
        assert!(
            claim_due(&workspace, due_at)
                .expect("claim único")
                .is_empty()
        );
        finish_run(&workspace, &created.id, Ok(Some(0.0123))).expect("concluir");
        let saved = list(&workspace).expect("ler estado").remove(0);
        assert_eq!(saved.last_run_status.as_deref(), Some("done"));
        assert_eq!(saved.last_cost_usd, Some(0.0123));
        assert!(saved.next_run_at_unix_ms.unwrap() > due_at);
        fs::remove_dir_all(workspace).expect("limpar workspace temporário");
    }
}
