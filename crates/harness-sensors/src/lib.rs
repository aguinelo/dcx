//! Executes acceptance sensors written by the acceptance author.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use harness_artifacts::{SensorImplementation, SensorResult, SensorSpec, SensorSuite};
use harness_engine::SensorRunner;

const MAX_EVIDENCE_CHARS: usize = 2000;

/// Runs Python sensor scripts in the task workspace.
///
/// Each script receives `HARNESS_WORKSPACE`, `HARNESS_SENSOR_ID` and
/// `HARNESS_CRITERION_ID`. Exit code 0 means pass; any other exit code or a
/// timeout means fail. The script must be self-contained and read only from the
/// workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxMode {
    Off,
    Auto,
    Required,
}

pub struct ScriptSensorRunner {
    python: OsString,
    scripts: BTreeMap<String, PathBuf>,
    sources: BTreeMap<String, String>,
    outputs: PathBuf,
    timeout: Duration,
    run_counter: u64,
    sandbox: SandboxMode,
}

impl ScriptSensorRunner {
    pub fn materialize(
        implementation: &SensorImplementation,
        suite: &SensorSuite,
        directory: &Path,
    ) -> Result<Self, String> {
        implementation
            .validate_for(suite)
            .map_err(|error| error.to_string())?;
        fs::create_dir_all(directory).map_err(|error| {
            format!(
                "cannot create sensor directory {}: {error}",
                directory.display()
            )
        })?;
        let outputs = directory.join("outputs");
        fs::create_dir_all(&outputs)
            .map_err(|error| format!("cannot create sensor output directory: {error}"))?;

        let mut scripts = BTreeMap::new();
        let mut sources = BTreeMap::new();
        for script in &implementation.scripts {
            let path = directory.join(&script.filename);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "cannot replace sensor {}: {error}",
                        script.sensor_id
                    ));
                }
            }
            fs::write(&path, &script.source)
                .map_err(|error| format!("cannot write sensor {}: {error}", script.sensor_id))?;
            make_read_only(&path)?;
            scripts.insert(script.sensor_id.clone(), path);
            sources.insert(script.sensor_id.clone(), script.source.clone());
        }

        Ok(Self {
            python: resolve_python(),
            scripts,
            sources,
            outputs,
            timeout: Duration::from_secs(60),
            run_counter: 0,
            sandbox: SandboxMode::Off,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_sandbox(mut self, sandbox: SandboxMode) -> Self {
        self.sandbox = sandbox;
        self
    }

    pub fn sandbox_from_env(mut self) -> Result<Self, String> {
        self.sandbox = match std::env::var("HARNESS_SENSOR_SANDBOX")
            .unwrap_or_else(|_| "off".to_owned())
            .to_ascii_lowercase()
            .as_str()
        {
            "off" => SandboxMode::Off,
            "auto" => SandboxMode::Auto,
            "required" => SandboxMode::Required,
            other => {
                return Err(format!(
                    "HARNESS_SENSOR_SANDBOX inválido: {other}; use off, auto ou required"
                ));
            }
        };
        Ok(self)
    }

    pub fn script_path(&self, sensor_id: &str) -> Option<&Path> {
        self.scripts.get(sensor_id).map(PathBuf::as_path)
    }

    fn evidence_paths(&mut self, sensor_id: &str) -> (PathBuf, PathBuf) {
        self.run_counter += 1;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let prefix = format!("{}-{}-{nanos}", safe_name(sensor_id), self.run_counter);
        (
            self.outputs.join(format!("{prefix}.out")),
            self.outputs.join(format!("{prefix}.err")),
        )
    }
}

impl SensorRunner for ScriptSensorRunner {
    fn run_sensor(
        &mut self,
        sensor: &SensorSpec,
        workspace: &Path,
    ) -> Result<SensorResult, String> {
        let script = self
            .scripts
            .get(&sensor.id)
            .ok_or_else(|| format!("no executable script for sensor {}", sensor.id))?
            .clone();
        let source = self
            .sources
            .get(&sensor.id)
            .ok_or_else(|| format!("no locked source for sensor {}", sensor.id))?;
        let current = fs::read_to_string(&script)
            .map_err(|error| format!("cannot read sensor {}: {error}", sensor.id))?;
        if &current != source {
            return Err(format!(
                "sensor {} does not match its locked source",
                sensor.id
            ));
        }
        let (stdout_path, stderr_path) = self.evidence_paths(&sensor.id);
        let stdout = fs::File::create(&stdout_path)
            .map_err(|error| format!("cannot capture sensor stdout: {error}"))?;
        let stderr = fs::File::create(&stderr_path)
            .map_err(|error| format!("cannot capture sensor stderr: {error}"))?;

        let mut command = sensor_command(&self.python, &script, workspace, self.sandbox)
            .map_err(|error| format!("cannot prepare sensor {}: {error}", sensor.id))?;
        let mut child = command
            .current_dir(workspace)
            .env_clear()
            .env("HARNESS_WORKSPACE", workspace)
            .env("HARNESS_SENSOR_ID", &sensor.id)
            .env("HARNESS_CRITERION_ID", &sensor.criterion_id)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("TMPDIR", std::env::var_os("TMPDIR").unwrap_or_default())
            .env(
                "LANG",
                std::env::var_os("LANG").unwrap_or_else(|| "C.UTF-8".into()),
            )
            .env(
                "LC_ALL",
                std::env::var_os("LC_ALL").unwrap_or_else(|| "C.UTF-8".into()),
            )
            .env("PYTHONIOENCODING", "utf-8")
            .env("HOME", &self.outputs)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|error| format!("cannot start sensor {}: {error}", sensor.id))?;

        let deadline = Instant::now() + self.timeout;
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        let _ = child.kill();
                        break child.wait().map_err(|error| {
                            format!("cannot reap sensor {}: {error}", sensor.id)
                        })?;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(format!("cannot wait for sensor {}: {error}", sensor.id));
                }
            }
        };

        let stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        let _ = fs::remove_file(&stdout_path);
        let _ = fs::remove_file(&stderr_path);

        let passed = status.success() && !timed_out;
        let evidence = if timed_out {
            format!("Sensor timed out after {:?}.", self.timeout)
        } else {
            let combined = [stdout.trim(), stderr.trim()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if combined.is_empty() {
                format!("Sensor exited with {status} without output.")
            } else {
                truncate(&combined, MAX_EVIDENCE_CHARS)
            }
        };

        Ok(SensorResult {
            sensor_id: sensor.id.clone(),
            criterion_id: sensor.criterion_id.clone(),
            passed,
            evidence,
        })
    }
}

fn resolve_python() -> OsString {
    if let Some(configured) = std::env::var_os("HARNESS_PYTHON") {
        return configured;
    }
    if let Ok(output) = Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !path.is_empty() {
                return path.into();
            }
        }
    }
    "python3".into()
}

fn sensor_command(
    python: &OsString,
    script: &Path,
    workspace: &Path,
    sandbox: SandboxMode,
) -> Result<Command, String> {
    if sandbox == SandboxMode::Off {
        return Ok({
            let mut command = Command::new(python);
            command.arg(script);
            command
        });
    }
    #[cfg(target_os = "macos")]
    {
        if sandbox_exec_available() {
            let profile = sandbox_profile(workspace)?;
            let mut command = Command::new("sandbox-exec");
            command.arg("-p").arg(profile).arg(python).arg(script);
            return Ok(command);
        }
        if sandbox == SandboxMode::Required {
            return Err("sandbox-exec não está disponível".to_owned());
        }
    }
    #[cfg(not(target_os = "macos"))]
    if sandbox == SandboxMode::Required {
        return Err("sandbox de sensores não implementado nesta plataforma".to_owned());
    }
    Ok({
        let mut command = Command::new(python);
        command.arg(script);
        command
    })
}

#[cfg(target_os = "macos")]
fn sandbox_exec_available() -> bool {
    std::process::Command::new("sandbox-exec")
        .arg("-h")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[cfg(target_os = "macos")]
fn sandbox_profile(workspace: &Path) -> Result<String, String> {
    let workspace = fs::canonicalize(workspace)
        .map_err(|error| format!("cannot resolve sensor workspace: {error}"))?;
    let workspace = escape_profile_path(&workspace.to_string_lossy());
    Ok(format!(
        "(version 1)\n(allow default)\n(deny network*)\n(deny file-write*)\n(allow file-write* (subpath \"{workspace}\") (subpath \"/private/tmp\") (subpath \"/tmp\") (subpath \"/dev\"))\n"
    ))
}

#[cfg(target_os = "macos")]
fn escape_profile_path(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn safe_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn truncate(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

#[cfg(unix)]
fn make_read_only(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("cannot inspect sensor script: {error}"))?
        .permissions();
    permissions.set_mode(0o400);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("cannot lock sensor script: {error}"))
}

#[cfg(not(unix))]
fn make_read_only(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_artifacts::{
        SCHEMA_VERSION, SensorImplementation, SensorKind, SensorScript, SensorSpec, SensorSuite,
        SensorType, SensorVisibility,
    };

    fn suite() -> SensorSuite {
        SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![SensorSpec {
                id: "probe".into(),
                criterion_id: "criterion".into(),
                sensor_type: SensorType::Test,
                kind: SensorKind::Computational,
                visibility: SensorVisibility::Visible,
                purpose: "acceptance".into(),
                check: "probe".into(),
            }],
        }
    }

    fn implementation(source: &str) -> SensorImplementation {
        SensorImplementation {
            schema_version: SCHEMA_VERSION,
            scripts: vec![SensorScript {
                sensor_id: "probe".into(),
                filename: "probe.py".into(),
                source: source.into(),
            }],
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "harness-sensors-test-{}-{nanos}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn run(source: &str, timeout: Duration) -> SensorResult {
        let suite = suite();
        let implementation = implementation(source);
        let directory = temp_dir("run");
        let mut runner = ScriptSensorRunner::materialize(&implementation, &suite, &directory)
            .expect("materialize")
            .with_timeout(timeout);
        let result = runner
            .run_sensor(&suite.sensors[0], &directory)
            .expect("run sensor");
        fs::remove_dir_all(directory).expect("remove temp dir");
        result
    }

    #[test]
    fn passing_script_returns_pass_and_evidence() {
        let result = run(
            "import os\nassert os.environ['HARNESS_WORKSPACE']\nprint('workspace ok')",
            Duration::from_secs(5),
        );
        assert!(result.passed);
        assert!(result.evidence.contains("workspace ok"));
        assert_eq!(result.sensor_id, "probe");
        assert_eq!(result.criterion_id, "criterion");
    }

    #[test]
    fn failing_script_returns_fail_with_stderr() {
        let result = run(
            "import sys\nprint('expected failure', file=sys.stderr)\nsys.exit(3)",
            Duration::from_secs(5),
        );
        assert!(!result.passed);
        assert!(result.evidence.contains("expected failure"));
    }

    #[test]
    fn slow_script_times_out() {
        let result = run(
            "import time\ntime.sleep(5)\nprint('too late')",
            Duration::from_millis(100),
        );
        assert!(!result.passed);
        assert!(result.evidence.contains("timed out"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sandbox_profile_denies_network_and_writes_outside_workspace() {
        let profile = sandbox_profile(Path::new("/tmp")).expect("profile");
        assert!(profile.contains("(deny network*)"));
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains("(allow file-write*"));
    }

    #[test]
    fn tampered_script_is_rejected_before_running() {
        let suite = suite();
        let implementation = implementation("print('ok')");
        let directory = temp_dir("tampered");
        let mut runner = ScriptSensorRunner::materialize(&implementation, &suite, &directory)
            .expect("materialize");
        let path = runner.script_path("probe").expect("script").to_path_buf();
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o600);
        }
        fs::set_permissions(&path, permissions).expect("unlock script");
        fs::write(&path, "print('tampered')").expect("tamper script");

        let error = runner
            .run_sensor(&suite.sensors[0], &directory)
            .expect_err("script alterado deve ser recusado");
        assert!(error.contains("locked source"));
        fs::remove_dir_all(directory).expect("remove temp dir");
    }

    #[test]
    fn materialize_rejects_implementation_without_script_for_every_sensor() {
        let suite = suite();
        let implementation = SensorImplementation {
            schema_version: SCHEMA_VERSION,
            scripts: vec![],
        };
        let directory = temp_dir("invalid");
        let result = ScriptSensorRunner::materialize(&implementation, &suite, &directory);
        assert!(result.is_err());
        fs::remove_dir_all(directory).expect("remove temp dir");
    }
}
