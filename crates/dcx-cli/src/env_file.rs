use std::path::{Path, PathBuf};

pub fn load() -> Result<(), String> {
    if let Some(path) = std::env::var_os("DCX_ENV_FILE") {
        return load_required(Path::new(&path));
    }
    load_optional(Path::new(".env"))
}

pub fn load_for_workspace(workspace: &Path) -> Result<(), String> {
    if std::env::var_os("DCX_ENV_FILE").is_some() {
        return Ok(());
    }
    load_optional(&workspace.join(".env"))
}

pub fn value(workspace: &Path, name: &str) -> Option<String> {
    if let Some(path) = environment_path(workspace) {
        if let Ok(entries) = dotenvy::from_path_iter(&path) {
            for entry in entries.flatten() {
                if entry.0 == name {
                    return Some(entry.1);
                }
            }
        }
    }
    std::env::var(name).ok()
}

/// Writes `updates` into `workspace/.env`, replacing existing `KEY=` lines in
/// place and appending keys that aren't present yet. Other lines (comments,
/// unrelated keys) are preserved as-is.
pub fn set_values(workspace: &Path, updates: &[(&str, String)]) -> Result<(), String> {
    let path = workspace.join(".env");
    let original = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = original.lines().map(str::to_owned).collect();
    let mut remaining: Vec<(&str, String)> = updates.to_vec();

    for line in lines.iter_mut() {
        let key = line.split('=').next().unwrap_or("").trim();
        if let Some(index) = remaining.iter().position(|(name, _)| *name == key) {
            let (name, value) = remaining.remove(index);
            *line = format!("{name}={value}");
        }
    }
    for (name, value) in remaining {
        lines.push(format!("{name}={value}"));
    }

    let mut content = lines.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    std::fs::write(&path, content)
        .map_err(|error| format!("não foi possível gravar {}: {error}", path.display()))
}

fn environment_path(workspace: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DCX_ENV_FILE") {
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
        assert!(load_required(Path::new("/nonexistent/dcx-env-file")).is_err());
    }

    #[test]
    fn missing_optional_file_is_ignored() {
        assert!(load_optional(Path::new("/nonexistent/dcx-env-file")).is_ok());
    }
}
