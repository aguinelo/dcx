use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dcx_artifacts::{
    Contract, ContractBundle, Plan, SensorImplementation, SensorResult, SensorSuite, StepReport,
    Understanding, Verdict, json_schemas,
};
use dcx_engine::{ExecutionContext, Executor, Verifier};

use crate::metrics::{self, CallMetric};
use serde::de::DeserializeOwned;

type CodexResult<T> = Result<T, Box<dyn Error>>;

pub struct CodexCliAnalyst {
    binary: OsString,
    model: Option<String>,
}

impl Default for CodexCliAnalyst {
    fn default() -> Self {
        Self {
            binary: std::env::var_os("DCX_CODEX_BIN").unwrap_or_else(|| "codex".into()),
            model: std::env::var("DCX_ANALYST_MODEL")
                .ok()
                .or_else(|| std::env::var("DCX_CODEX_MODEL").ok()),
        }
    }
}

impl CodexCliAnalyst {
    pub fn understand(
        &mut self,
        request: &str,
        workspace: &Path,
        previous: Option<&Understanding>,
    ) -> Result<Understanding, String> {
        let prompt = understanding_prompt(request, workspace, previous)?;
        invoke_codex(
            &self.binary,
            self.model.as_deref(),
            "read-only",
            workspace,
            "understanding.schema.json",
            &prompt,
        )
    }

    pub fn contract(
        &mut self,
        request: &str,
        workspace: &Path,
        understanding: &Understanding,
        previous: Option<&ContractBundle>,
        rejection_feedback: Option<&str>,
        validation_feedback: Option<&str>,
    ) -> Result<ContractBundle, String> {
        let prompt = contract_prompt(
            request,
            workspace,
            understanding,
            previous,
            rejection_feedback,
            validation_feedback,
        )?;
        invoke_codex(
            &self.binary,
            self.model.as_deref(),
            "read-only",
            workspace,
            "contract_bundle.schema.json",
            &prompt,
        )
    }
}

pub struct CodexCliAcceptanceAuthor {
    binary: OsString,
    model: Option<String>,
}

impl Default for CodexCliAcceptanceAuthor {
    fn default() -> Self {
        Self {
            binary: std::env::var_os("DCX_CODEX_BIN").unwrap_or_else(|| "codex".into()),
            model: std::env::var("DCX_ACCEPTANCE_MODEL")
                .ok()
                .or_else(|| std::env::var("DCX_CODEX_MODEL").ok()),
        }
    }
}

impl CodexCliAcceptanceAuthor {
    pub fn implement(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
        previous: Option<&SensorImplementation>,
        feedback: Option<&str>,
    ) -> Result<SensorImplementation, String> {
        let prompt = acceptance_prompt(workspace, bundle, locale, previous, feedback)?;
        invoke_codex(
            &self.binary,
            self.model.as_deref(),
            "read-only",
            workspace,
            "sensor_implementation.schema.json",
            &prompt,
        )
    }
}

pub struct CodexCliPlanner {
    binary: OsString,
    model: Option<String>,
}

impl Default for CodexCliPlanner {
    fn default() -> Self {
        Self {
            binary: std::env::var_os("DCX_CODEX_BIN").unwrap_or_else(|| "codex".into()),
            model: std::env::var("DCX_PLANNER_MODEL")
                .ok()
                .or_else(|| std::env::var("DCX_CODEX_MODEL").ok()),
        }
    }
}

impl CodexCliPlanner {
    pub fn plan(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
    ) -> Result<Plan, String> {
        let prompt = planner_prompt(workspace, bundle, locale)?;
        invoke_codex(
            &self.binary,
            self.model.as_deref(),
            "read-only",
            workspace,
            "plan.schema.json",
            &prompt,
        )
    }
}

pub struct CodexCliExecutor {
    binary: OsString,
    model: Option<String>,
}

impl Default for CodexCliExecutor {
    fn default() -> Self {
        Self {
            binary: std::env::var_os("DCX_CODEX_BIN").unwrap_or_else(|| "codex".into()),
            model: std::env::var("DCX_CODEX_MODEL").ok(),
        }
    }
}

impl Executor for CodexCliExecutor {
    fn execute(
        &mut self,
        context: &ExecutionContext,
        workspace: &Path,
    ) -> Result<StepReport, String> {
        let prompt = executor_prompt(context)?;
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
            binary: std::env::var_os("DCX_CODEX_BIN").unwrap_or_else(|| "codex".into()),
            model: std::env::var("DCX_VERIFIER_MODEL")
                .ok()
                .or_else(|| std::env::var("DCX_CODEX_MODEL").ok()),
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
        let prompt = verifier_prompt(contract, suite, step_report, sensor_results)?;
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

pub(crate) fn understanding_prompt(
    request: &str,
    workspace: &Path,
    previous: Option<&Understanding>,
) -> Result<String, String> {
    let role =
        include_str!("../../../prompts/analyst.understanding.md").replace("{{locale}}", "pt-BR");
    Ok(format!(
        "{role}\n\nContexto desta execução:\n- Locale: pt-BR\n- Projeto: {}\n- O workspace deve ser lido sem alterações. Não acesse a rede.\n\nPedido:\n{request}\n\nEntendimento anterior e respostas já recebidas (se houver):\n{}\n\nRetorne apenas o artifact understanding, incluindo respostas anteriores e perguntas antigas sem reabri-las.",
        workspace.display(),
        serde_json::to_string_pretty(&previous).map_err(|error| error.to_string())?,
    ))
}

pub(crate) fn contract_prompt(
    request: &str,
    workspace: &Path,
    understanding: &Understanding,
    previous: Option<&ContractBundle>,
    rejection_feedback: Option<&str>,
    validation_feedback: Option<&str>,
) -> Result<String, String> {
    let role = include_str!("../../../prompts/analyst.contracting.md");
    Ok(format!(
        "{role}\n\nContexto desta execução:\n- Locale: {}\n- Projeto: {}\n- O workspace deve ser lido sem alterações. Não acesse a rede.\n\nPedido original:\n{request}\n\nEntendimento aprovado pelo usuário:\n{}\n\nContrato anterior (se houver):\n{}\n\nFeedback de rejeição do usuário (se houver):\n{}\n\nErros de validação que devem ser corrigidos (se houver):\n{}\n\nRetorne apenas um contract_bundle JSON válido. Preserve ids e redação de critérios inalterados quando estiver refazendo o contrato.",
        understanding.locale,
        workspace.display(),
        serde_json::to_string_pretty(understanding).map_err(|error| error.to_string())?,
        serde_json::to_string_pretty(&previous).map_err(|error| error.to_string())?,
        rejection_feedback.unwrap_or("Nenhum."),
        validation_feedback.unwrap_or("Nenhum."),
    ))
}

pub(crate) fn acceptance_prompt(
    workspace: &Path,
    bundle: &ContractBundle,
    locale: &str,
    previous: Option<&SensorImplementation>,
    feedback: Option<&str>,
) -> Result<String, String> {
    let role = include_str!("../../../prompts/acceptance_author.md").replace("{{locale}}", locale);
    Ok(format!(
        "{role}\n\nContexto desta execução:\n- Locale: {locale}\n- Projeto: {}\n- O workspace deve ser lido sem alterações. Não acesse a rede.\n\nContrato aprovado e suíte de sensores:\n{}\n\nImplementação anterior (se houver):\n{}\n\nFeedback de validação ou de baseline (se houver):\n{}\n\nRetorne apenas um sensor_implementation JSON válido, com um script Python para cada sensor da suíte.",
        workspace.display(),
        serde_json::to_string_pretty(bundle).map_err(|error| error.to_string())?,
        serde_json::to_string_pretty(&previous).map_err(|error| error.to_string())?,
        feedback.unwrap_or("Nenhum."),
    ))
}

pub(crate) fn planner_prompt(
    workspace: &Path,
    bundle: &ContractBundle,
    locale: &str,
) -> Result<String, String> {
    let role = include_str!("../../../prompts/planner.md").replace("{{locale}}", locale);
    Ok(format!(
        "{role}\n\nContexto desta execução:\n- Locale: {locale}\n- Projeto: {}\n- O workspace deve ser lido sem alterações. Não acesse a rede.\n\nContrato aprovado e suíte de sensores:\n{}\n\nRetorne apenas um plan JSON válido que cubra todos os critérios e atribua todos os sensores.",
        workspace.display(),
        serde_json::to_string_pretty(bundle).map_err(|error| error.to_string())?,
    ))
}

pub(crate) fn executor_prompt(context: &ExecutionContext) -> Result<String, String> {
    Ok(format!(
        "Você é o executor do dcode. Implemente apenas o contrato e os critérios abaixo no workspace.\n\
         Use somente os sensores visíveis fornecidos para autoverificação. Sensores ocultos não fazem parte do seu contexto.\n\
         Não altere arquivos de sensores, schemas ou fora do workspace. Não faça operações git.\n\
         Ao terminar, retorne somente um step_report JSON válido no schema fornecido.\n\
         O campo step_id do step_report deve ser exatamente: {}.\n\
         Tentativa: {}\nContrato:\n{}\nSensores visíveis:\n{}\nFeedback anterior:\n{}",
        context.step_id,
        context.attempt,
        serde_json::to_string_pretty(&context.contract).map_err(|error| error.to_string())?,
        serde_json::to_string_pretty(&context.visible_sensors).map_err(|error| error.to_string())?,
        serde_json::to_string_pretty(&context.feedback).map_err(|error| error.to_string())?,
    ))
}

pub(crate) fn verifier_prompt(
    contract: &Contract,
    suite: &SensorSuite,
    step_report: &StepReport,
    sensor_results: &[SensorResult],
) -> Result<String, String> {
    Ok(format!(
        "Você é o verifier independente. Julgue o contrato e a implementação integrada.\n\
         Inspecione no workspace os caminhos de files_changed; um caminho ausente pode ter sido removido pela implementação. Não edite nada.\n\
         Não use histórico de conversa nem suposições. Cada critério precisa de evidência; qualquer sensor de aceite ausente ou reprovado impede aprovação.\n\
         Retorne somente um verdict JSON válido no schema fornecido.\nContrato:\n{}\nSuíte de sensores (visíveis e ocultos):\n{}\nStep report:\n{}\nResultados dos sensores:\n{}",
        serde_json::to_string_pretty(contract).map_err(|error| error.to_string())?,
        serde_json::to_string_pretty(suite).map_err(|error| error.to_string())?,
        serde_json::to_string_pretty(step_report).map_err(|error| error.to_string())?,
        serde_json::to_string_pretty(sensor_results).map_err(|error| error.to_string())?,
    ))
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

    let started = Instant::now();
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

    metrics::before_call("codex")?;
    let output = metrics::run_provider_command(&mut command, Some(prompt.as_bytes()))
        .map_err(|error| format!("Codex CLI: {error}"))?;
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
    let parsed = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "Codex CLI retornou JSON inválido: {error}; stdout: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })?;
    metrics::record(CallMetric {
        role: metrics::role_for_schema(schema_name),
        provider: "codex".to_owned(),
        model: model.map(str::to_owned),
        duration_ms: started.elapsed().as_millis(),
        cost_usd: None,
        input_tokens: None,
        output_tokens: None,
    })?;
    Ok(parsed)
}

struct ReadOnlySchema {
    path: PathBuf,
}

impl ReadOnlySchema {
    fn create(name: &str, contents: &str) -> CodexResult<Self> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path =
            std::env::temp_dir().join(format!("dcx-schema-{}-{nanos}-{name}", std::process::id()));
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
