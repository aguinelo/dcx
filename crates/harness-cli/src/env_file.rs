use std::path::{Path, PathBuf};

pub fn load() -> Result<(), String> {
    if let Some(path) = std::env::var_os("HARNESS_ENV_FILE") {
        return load_required(Path::new(&path));
    }
    load_optional(Path::new(".env"))
}

pub fn load_for_workspace(workspace: &Path) -> Result<(), String> {
    if std::env::var_os("HARNESS_ENV_FILE").is_some() {
        return Ok(());
    }
    load_optional(&workspace.join(".env"))
}

pub fn api_key(workspace: &Path) -> Option<String> {
    let path = environment_path(workspace)?;
    let entries = dotenvy::from_path_iter(&path).ok()?;
    for entry in entries.flatten() {
        if entry.0 == "ANTHROPIC_API_KEY" {
            return Some(entry.1);
        }
    }
    None
}

fn environment_path(workspace: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HARNESS_ENV_FILE") {
        return Some(PathBuf::from(path));
    }
    let workspace_env = workspace.join(".env");
    if workspace_env.is_file() {
        return Some(workspace_env);
    }
    let current_env = PathBuf::from(".env");
    current_env.is_file().then_some(current_env)
}

fn load_optional(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Ok(());
    }
    load_required(path)
}

fn load_required(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!(
            "arquivo de ambiente não encontrado: {}",
            path.display()
        ));
    }
    dotenvy::from_path(PathBuf::from(path))
        .map(|_| ())
        .map_err(|error| format!("não foi possível carregar {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_explicit_file_is_an_error() {
        assert!(load_required(Path::new("/nonexistent/harness-env-file")).is_err());
    }

    #[test]
    fn missing_optional_file_is_ignored() {
        assert!(load_optional(Path::new("/nonexistent/harness-env-file")).is_ok());
    }
}
