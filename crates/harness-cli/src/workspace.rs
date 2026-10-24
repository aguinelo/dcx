use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const SKIPPED: &[&str] = &[
    ".git",
    ".harness",
    "target",
    "node_modules",
    ".venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppliedChange {
    Updated(String),
    Deleted(String),
}

pub fn copy_project(
    source: &Path,
    destination: &Path,
    excluded: &[std::path::PathBuf],
) -> Result<(), String> {
    if destination.exists() {
        fs::remove_dir_all(destination).map_err(|error| {
            format!(
                "cannot reset execution workspace {}: {error}",
                destination.display()
            )
        })?;
    }
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "cannot create execution workspace {}: {error}",
            destination.display()
        )
    })?;
    let excluded = excluded
        .iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect::<Vec<_>>();
    let mut total = 0_u64;
    copy_directory(source, destination, &mut total, &excluded)
}

fn copy_directory(
    source: &Path,
    destination: &Path,
    total: &mut u64,
    excluded: &[std::path::PathBuf],
) -> Result<(), String> {
    let entries = fs::read_dir(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read project entry: {error}"))?;
        let name = entry.file_name();
        if SKIPPED
            .iter()
            .any(|skipped| name.to_string_lossy() == *skipped)
        {
            continue;
        }
        let path = entry.path();
        if !excluded.is_empty() {
            let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if excluded
                .iter()
                .any(|excluded| canonical.starts_with(excluded))
            {
                continue;
            }
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let target = destination.join(&name);
        if metadata.is_dir() {
            fs::create_dir_all(&target).map_err(|error| {
                format!("cannot create directory {}: {error}", target.display())
            })?;
            copy_directory(&path, &target, total, excluded)?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_FILE_BYTES {
                return Err(format!(
                    "project file exceeds the 16 MiB copy limit: {}",
                    path.display()
                ));
            }
            *total = total.saturating_add(metadata.len());
            if *total > MAX_TOTAL_BYTES {
                return Err("project exceeds the 256 MiB copy limit".to_owned());
            }
            fs::copy(&path, &target).map_err(|error| {
                format!(
                    "cannot copy {} to {}: {error}",
                    path.display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

pub fn apply_changes(
    original: &Path,
    executed: &Path,
    paths: &[String],
) -> Result<Vec<AppliedChange>, String> {
    let mut relatives = Vec::with_capacity(paths.len());
    for path in paths {
        let relative = Path::new(path);
        if path.trim().is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err(format!("refusing to apply unsafe path: {path}"));
        }
        relatives.push(relative.to_path_buf());
    }

    let backup = backup_directory();
    fs::create_dir_all(&backup).map_err(|error| format!("cannot create apply backup: {error}"))?;
    let mut originals = Vec::with_capacity(relatives.len());
    for relative in &relatives {
        let target = original.join(relative);
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let _ = fs::remove_dir_all(&backup);
                return Err(format!(
                    "refusing to replace symlink while applying {}",
                    relative.display()
                ));
            }
            Ok(metadata) if metadata.is_dir() => {
                let _ = fs::remove_dir_all(&backup);
                return Err(format!(
                    "refusing to replace directory while applying {}",
                    relative.display()
                ));
            }
            Ok(_) => {
                let saved = backup.join(relative);
                if let Some(parent) = saved.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("cannot create apply backup directory: {error}")
                    })?;
                }
                fs::copy(&target, &saved)
                    .map_err(|error| format!("cannot back up {}: {error}", target.display()))?;
                originals.push((relative.clone(), true));
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::NotADirectory =>
            {
                originals.push((relative.clone(), false));
            }
            Err(error) => {
                let _ = fs::remove_dir_all(&backup);
                return Err(format!("cannot inspect {}: {error}", target.display()));
            }
        }
    }

    let mut applied = Vec::new();
    for relative in &relatives {
        match apply_one(original, executed, relative) {
            Ok(change) => applied.push(change),
            Err(error) => {
                let rollback = restore_backup(original, &backup, &originals);
                let _ = fs::remove_dir_all(&backup);
                return Err(match rollback {
                    Ok(()) => format!("{error}; all applied files were rolled back"),
                    Err(rollback_error) => {
                        format!("{error}; rollback also failed: {rollback_error}")
                    }
                });
            }
        }
    }
    let _ = fs::remove_dir_all(&backup);
    Ok(applied)
}

fn apply_one(original: &Path, executed: &Path, relative: &Path) -> Result<AppliedChange, String> {
    let source = executed.join(relative);
    let target = original.join(relative);
    let source_metadata = fs::symlink_metadata(&source).ok();
    if source_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        fs::copy(&source, &target).map_err(|error| {
            format!(
                "cannot apply {} to {}: {error}",
                source.display(),
                target.display()
            )
        })?;
        Ok(AppliedChange::Updated(
            relative.to_string_lossy().replace('\\', "/"),
        ))
    } else if target.exists() {
        fs::remove_file(&target)
            .map_err(|error| format!("cannot remove {}: {error}", target.display()))?;
        Ok(AppliedChange::Deleted(
            relative.to_string_lossy().replace('\\', "/"),
        ))
    } else {
        Ok(AppliedChange::Updated(
            relative.to_string_lossy().replace('\\', "/"),
        ))
    }
}

fn restore_backup(
    original: &Path,
    backup: &Path,
    originals: &[(PathBuf, bool)],
) -> Result<(), String> {
    for (relative, existed) in originals {
        let target = original.join(relative);
        if *existed {
            let saved = backup.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot recreate {}: {error}", parent.display()))?;
            }
            fs::copy(&saved, &target)
                .map_err(|error| format!("cannot restore {}: {error}", target.display()))?;
        } else {
            match fs::remove_file(&target) {
                Ok(()) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        || error.kind() == std::io::ErrorKind::NotADirectory => {}
                Err(error) => {
                    return Err(format!("cannot remove {}: {error}", target.display()));
                }
            }
        }
    }
    Ok(())
}

fn backup_directory() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "harness-apply-backup-{}-{nanos}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("relógio")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "harness-workspace-{}-{nanos}-{label}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("criar diretório");
        path
    }

    #[test]
    fn copy_skips_build_and_vcs_directories() {
        let source = unique_dir("copy-source");
        let destination = unique_dir("copy-destination");
        fs::write(source.join("main.py"), "print('ok')\n").expect("arquivo");
        fs::create_dir_all(source.join("target")).expect("target");
        fs::write(source.join("target/debug.bin"), "bin").expect("binário");
        fs::create_dir_all(source.join(".git")).expect("git");
        fs::write(source.join(".git/HEAD"), "ref").expect("head");
        let state = source.join("state");
        fs::create_dir_all(&state).expect("state");
        fs::write(state.join("run.json"), "{}").expect("state file");

        copy_project(&source, &destination, &[state]).expect("copiar projeto");

        assert!(destination.join("main.py").is_file());
        assert!(!destination.join("target").exists());
        assert!(!destination.join(".git").exists());
        assert!(!destination.join("state").exists());
    }

    #[test]
    fn failed_apply_rolls_back_files_already_copied() {
        let original = unique_dir("apply-rollback-original");
        let executed = unique_dir("apply-rollback-executed");
        fs::write(original.join("keep.py"), "old\n").expect("antigo");
        fs::write(executed.join("keep.py"), "new\n").expect("novo");
        fs::write(original.join("blocked"), "not a directory\n").expect("bloqueio");
        fs::create_dir_all(executed.join("blocked")).expect("dir no executado");
        fs::write(executed.join("blocked/new.py"), "new\n").expect("novo arquivo");

        let error = apply_changes(
            &original,
            &executed,
            &["keep.py".into(), "blocked/new.py".into()],
        )
        .expect_err("aplicação deve falhar no segundo arquivo");

        assert!(error.contains("rolled back"), "erro inesperado: {error}");
        assert_eq!(
            fs::read_to_string(original.join("keep.py")).expect("conteúdo"),
            "old\n"
        );
        assert!(!original.join("blocked/new.py").exists());
    }

    #[test]
    fn apply_updates_and_deletes_without_leaving_the_project() {
        let original = unique_dir("apply-original");
        let executed = unique_dir("apply-executed");
        fs::write(original.join("keep.py"), "old\n").expect("antigo");
        fs::write(executed.join("keep.py"), "new\n").expect("novo");
        fs::write(executed.join("added.py"), "added\n").expect("adicionado");
        fs::write(original.join("removed.py"), "removed\n").expect("removido");

        let applied = apply_changes(
            &original,
            &executed,
            &["keep.py".into(), "added.py".into(), "removed.py".into()],
        )
        .expect("aplicar mudanças");

        assert_eq!(
            fs::read_to_string(original.join("keep.py")).expect("conteúdo"),
            "new\n"
        );
        assert!(original.join("added.py").is_file());
        assert!(!original.join("removed.py").exists());
        assert_eq!(applied.len(), 3);
        assert!(apply_changes(&original, &executed, &["../escape.py".into()]).is_err());
    }
}
