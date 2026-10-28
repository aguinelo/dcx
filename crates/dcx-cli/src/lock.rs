use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct ProjectLock {
    path: PathBuf,
}

impl ProjectLock {
    pub fn acquire(project: &Path) -> Result<Self, String> {
        let directory = project.join(".dcx");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
        let path = directory.join("lock");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(
                    file,
                    "pid={} started={}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_secs())
                        .unwrap_or_default()
                );
                Ok(Self { path })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let owner = fs::read_to_string(&path).unwrap_or_default();
                if let Some(pid) = owner_pid(&owner) {
                    if !process_alive(pid) {
                        let _ = fs::remove_file(&path);
                        return Self::acquire(project);
                    }
                }
                Err(format!(
                    "another dcx run holds {} ({})",
                    path.display(),
                    owner.trim()
                ))
            }
            Err(error) => Err(format!("cannot lock {}: {error}", path.display())),
        }
    }
}

fn owner_pid(contents: &str) -> Option<u32> {
    contents
        .split_whitespace()
        .find_map(|field| field.strip_prefix("pid="))
        .and_then(|pid| pid.parse().ok())
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    true
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let project = std::env::temp_dir().join(format!("dcx-lock-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&project).expect("project");
        {
            let _lock = ProjectLock::acquire(&project).expect("first lock");
            assert!(ProjectLock::acquire(&project).is_err());
        }
        let _lock = ProjectLock::acquire(&project).expect("lock after release");
        drop(_lock);

        fs::write(project.join(".dcx/lock"), "pid=4294967295 started=0\n").expect("stale lock");
        let _lock = ProjectLock::acquire(&project).expect("stale lock is recovered");
        drop(_lock);
        let _ = fs::remove_dir_all(project);
    }
}
