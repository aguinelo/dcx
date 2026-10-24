use std::fs;
#[cfg(test)]
use std::path::PathBuf;
use std::path::{Component, Path};

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

pub fn copy_project(source: &Path, destination: &Path) -> Result<(), String> {
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
    let mut total = 0_u64;
    copy_directory(source, destination, &mut total)
}

fn copy_directory(source: &Path, destination: &Path, total: &mut u64) -> Result<(), String> {
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
            copy_directory(&path, &target, total)?;
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
    let mut applied = Vec::new();
    for path in paths {
        let relative = Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err(format!("refusing to apply unsafe path: {path}"));
        }
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
            if target
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                fs::remove_file(&target).map_err(|error| {
                    format!("cannot replace symlink {}: {error}", target.display())
                })?;
            }
            fs::copy(&source, &target).map_err(|error| {
                format!(
                    "cannot apply {} to {}: {error}",
                    source.display(),
                    target.display()
                )
            })?;
            applied.push(AppliedChange::Updated(path.clone()));
        } else if target.exists() {
            fs::remove_file(&target)
                .map_err(|error| format!("cannot remove {}: {error}", target.display()))?;
            applied.push(AppliedChange::Deleted(path.clone()));
        }
    }
    Ok(applied)
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

        copy_project(&source, &destination).expect("copiar projeto");

        assert!(destination.join("main.py").is_file());
        assert!(!destination.join("target").exists());
        assert!(!destination.join(".git").exists());
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
