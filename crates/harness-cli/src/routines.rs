use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_TITLE_CHARS: usize = 100;
const MAX_PROMPT_CHARS: usize = 12_000;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Routine {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

pub fn list(workspace: &Path) -> Result<Vec<Routine>, String> {
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

    let mut routines = list(workspace)?;
    let now = now_ms();
    if let Some(id) = id {
        validate_id(id)?;
        let Some(existing) = routines.iter_mut().find(|routine| routine.id == id) else {
            return Err("rotina não encontrada".to_owned());
        };
        existing.title = title.to_owned();
        existing.prompt = prompt.to_owned();
        existing.updated_at_unix_ms = now;
        let saved = existing.clone();
        write(workspace, &routines)?;
        return Ok(saved);
    }

    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let id = format!("routine-{now}-{}-{sequence}", std::process::id());
    let routine = Routine {
        id,
        title: title.to_owned(),
        prompt: prompt.to_owned(),
        created_at_unix_ms: now,
        updated_at_unix_ms: now,
    };
    routines.push(routine.clone());
    write(workspace, &routines)?;
    Ok(routine)
}

pub fn delete(workspace: &Path, id: &str) -> Result<(), String> {
    validate_id(id)?;
    let mut routines = list(workspace)?;
    routines.retain(|routine| routine.id != id);
    write(workspace, &routines)
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
            "harness-routines-{}-{}",
            std::process::id(),
            now_ms()
        ))
    }

    #[test]
    fn routines_can_be_created_updated_and_deleted() {
        let workspace = temp_workspace();
        let created = save(&workspace, None, "Revisar PR", "Revise o PR atual.").expect("criar");
        let another = save(
            &workspace,
            None,
            "Revisar issues",
            "Liste as issues abertas.",
        )
        .expect("criar segunda rotina");
        assert_ne!(created.id, another.id);
        assert_eq!(list(&workspace).expect("listar").len(), 2);
        let updated = save(
            &workspace,
            Some(&created.id),
            "Revisar diff",
            "Analise o diff.",
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
        assert!(save(&workspace, None, " ", "prompt").is_err());
        assert!(save(&workspace, None, "Nome", " ").is_err());
        assert!(delete(&workspace, "../outside").is_err());
    }
}
