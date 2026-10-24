use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use harness_artifacts::{Contract, SensorResult, SensorSuite, StepReport, Verdict, json_schemas};
use harness_engine::{ExecutionContext, Executor, Verifier};
use serde::de::DeserializeOwned;

type CodexResult<T> = Result<T, Box<dyn Error>>;

pub struct CodexCliExecutor {
    binary: OsString,
    model: Option<String>,
}

impl Default for CodexCliExecutor {
    fn default() -> Self {
        Self {
            binary: std::env::var_os("HARNESS_CODEX_BIN").unwrap_or_else(|| "codex".into()),
            model: std::env::var("HARNESS_CODEX_MODEL").ok(),
        }
    }
}

impl Executor for CodexCliExecutor {
    fn execute(
        &mut self,
        context: &ExecutionContext,
        workspace: &Path,
    ) -> Result<StepReport, String> {
        let prompt = format!(
            "Você é o executor do Harness. Implemente apenas o contrato e os critérios abaixo no workspace.\n\
             Use somente os sensores visíveis fornecidos para autoverificação. Sensores ocultos não fazem parte do seu contexto.\n\
             Não altere arquivos de sensores, schemas ou fora do workspace. Não faça operações git.\n\
             Ao terminar, retorne somente um step_report JSON válido no schema fornecido.\n\
             O campo step_id do step_report deve ser exatamente: {}.\n\
             Tentativa: {}\nContrato:\n{}\nSensores visíveis:\n{}\nFeedback anterior:\n{}",
            context.step_id,
            context.attempt,
            serde_json::to_string_pretty(&context.contract).map_err(|error| error.to_string())?,
            serde_json::to_string_pretty(&context.visible_sensors)
                .map_err(|error| error.to_string())?,
            serde_json::to_string_pretty(&context.feedback).map_err(|error| error.to_string())?,
        );
        invoke_codex(
            &self.binary,
            self.model.as_deref(),
            "workspace-write",
            workspace,
            "step_report.schema.json",
            &prompt,
        )
    }
}

pub struct CodexCliVerifier {
    binary: OsString,
    model: Option<String>,
}

impl Default for CodexCliVerifier {
    fn default() -> Self {
        Self {
            binary: std::env::var_os("HARNESS_CODEX_BIN").unwrap_or_else(|| "codex".into()),
            model: std::env::var("HARNESS_VERIFIER_MODEL")
                .ok()
                .or_else(|| std::env::var("HARNESS_CODEX_MODEL").ok()),
        }
    }
}

impl Verifier for CodexCliVerifier {
    fn verify(
        &mut self,
        contract: &Contract,
        suite: &SensorSuite,
        step_report: &StepReport,
        sensor_results: &[SensorResult],
        workspace: &Path,
    ) -> Result<Verdict, String> {
        let prompt = format!(
            "Você é o verifier independente. Julgue o contrato e a implementação integrada.\n\
             Inspecione no workspace os caminhos de files_changed; um caminho ausente pode ter sido removido pela implementação. Não edite nada.\n\
             Não use histórico de conversa nem suposições. Cada critério precisa de evidência; qualquer sensor de aceite ausente ou reprovado impede aprovação.\n\
             Retorne somente um verdict JSON válido no schema fornecido.\nContrato:\n{}\nSuíte de sensores (visíveis e ocultos):\n{}\nStep report:\n{}\nResultados dos sensores:\n{}",
            serde_json::to_string_pretty(contract).map_err(|error| error.to_string())?,
            serde_json::to_string_pretty(suite).map_err(|error| error.to_string())?,
            serde_json::to_string_pretty(step_report).map_err(|error| error.to_string())?,
            serde_json::to_string_pretty(sensor_results).map_err(|error| error.to_string())?,
        );
        invoke_codex(
            &self.binary,
            self.model.as_deref(),
            "read-only",
            workspace,
            "verdict.schema.json",
            &prompt,
        )
    }
}

fn invoke_codex<T: DeserializeOwned>(
    binary: &OsString,
    model: Option<&str>,
    sandbox: &str,
    workspace: &Path,
    schema_name: &str,
    prompt: &str,
) -> Result<T, String> {
    let schema = json_schemas()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|(name, _)| *name == schema_name)
        .ok_or_else(|| format!("schema não encontrado: {schema_name}"))?
        .1;
    let schema_file = ReadOnlySchema::create(schema_name, &schema).map_err(|e| e.to_string())?;

    let mut command = Command::new(binary);
    command
        .args(["exec", "--ephemeral", "--sandbox", sandbox, "--cd"])
        .arg(workspace)
        .args(["--skip-git-repo-check", "--output-schema"])
        .arg(&schema_file.path)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(model) = model {
        command.args(["--model", model]);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("não foi possível iniciar Codex CLI: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "stdin do Codex CLI indisponível".to_owned())?
        .write_all(prompt.as_bytes())
        .map_err(|error| format!("falha ao enviar contexto ao Codex CLI: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("falha ao aguardar Codex CLI: {error}"))?;
    if !output.status.success() {
        let diagnostics = String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|line| line.starts_with("ERROR:"))
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "Codex CLI encerrou com {}: {}",
            output.status,
            if diagnostics.is_empty() {
                "diagnóstico omitido para não repetir o prompt enviado".to_owned()
            } else {
                diagnostics
            }
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "Codex CLI retornou JSON inválido: {error}; stdout: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })
}

struct ReadOnlySchema {
    path: PathBuf,
}

impl ReadOnlySchema {
    fn create(name: &str, contents: &str) -> CodexResult<Self> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "harness-schema-{}-{nanos}-{name}",
            std::process::id()
        ));
        fs::write(&path, contents)?;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions)?;
        Ok(Self { path })
    }
}

impl Drop for ReadOnlySchema {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
