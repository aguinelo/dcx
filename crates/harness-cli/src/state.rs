use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub struct PruneReport {
    pub removed: Vec<PathBuf>,
    pub kept: usize,
    pub dry_run: bool,
}

pub fn prune_runs(root: &Path, older_than: Duration, dry_run: bool) -> Result<PruneReport, String> {
    let runs = root.join("runs");
    if !runs.exists() {
        return Ok(PruneReport {
            removed: vec![],
            kept: 0,
            dry_run,
        });
    }
    let now = SystemTime::now();
    let mut removed = Vec::new();
    let mut kept = 0;
    let entries =
        fs::read_dir(&runs).map_err(|error| format!("cannot read {}: {error}", runs.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read run entry: {error}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let age = now.duration_since(modified).unwrap_or_default();
        if age >= older_than {
            if !dry_run {
                fs::remove_dir_all(&path)
                    .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
            }
            removed.push(path);
        } else {
            kept += 1;
        }
    }
    Ok(PruneReport {
        removed,
        kept,
        dry_run,
    })
}

pub fn run_prune(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut days: Option<u64> = None;
    let mut dry_run = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--older-than-days" => {
                let value = args.next().ok_or("--older-than-days exige um número")?;
                days = Some(
                    value
                        .parse()
                        .map_err(|_| format!("valor inválido para --older-than-days: {value}"))?,
                );
            }
            "--dry-run" => dry_run = true,
            other => return Err(format!("argumento desconhecido: {other}")),
        }
    }
    let days = days.ok_or("informe --older-than-days <N>")?;
    let root = crate::interactive::state_root();
    let report = prune_runs(
        &root,
        Duration::from_secs(days.saturating_mul(86_400)),
        dry_run,
    )?;
    println!(
        "{} run(s) {}; {} preservado(s) em {}",
        report.removed.len(),
        if report.dry_run {
            "seriam removidos"
        } else {
            "removidos"
        },
        report.kept,
        root.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("harness-state-test-{}-{nanos}", std::process::id()));
        fs::create_dir_all(path.join("runs/run-old")).expect("old run");
        fs::create_dir_all(path.join("runs/run-new")).expect("new run");
        path
    }

    #[test]
    fn dry_run_reports_without_removing() {
        let root = temp_root();
        let report = prune_runs(&root, Duration::ZERO, true).expect("prune");
        assert_eq!(report.removed.len(), 2);
        assert!(root.join("runs/run-old").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prune_removes_old_runs_and_keeps_recent_ones() {
        let root = temp_root();
        let old = fs::File::open(root.join("runs/run-old")).expect("open old");
        old.set_modified(SystemTime::now() - Duration::from_secs(86_400 * 30))
            .expect("age old run");
        let report = prune_runs(&root, Duration::from_secs(86_400), false).expect("prune");
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.kept, 1);
        assert!(!root.join("runs/run-old").exists());
        assert!(root.join("runs/run-new").exists());
        let _ = fs::remove_dir_all(root);
    }
}
