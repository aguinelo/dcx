use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use harness_artifacts::{
    Contract, ContractBundle, Plan, SensorImplementation, SensorResult, SensorSuite, StepReport,
    Understanding, Verdict, json_schemas,
};
use harness_engine::{ExecutionContext, Executor, Verifier};
use serde::de::DeserializeOwned;

use crate::codex::{
    acceptance_prompt, contract_prompt, executor_prompt, planner_prompt, understanding_prompt,
    verifier_prompt,
};
use crate::metrics::{self, CallMetric};
use crate::providers::{AcceptanceAuthor, Analyst, Planner};

enum Mode {
    ReadOnly,
    WorkspaceWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    Anthropic,
    Minimax,
}

impl Backend {
    pub(crate) fn provider_name(self) -> &'static str {
        match self {
            Self::Anthropic => "claude",
            Self::Minimax => "minimax",
        }
    }
}

pub(crate) fn resolve_model(
    workspace: &Path,
    backend: Backend,
    model_env: Option<&str>,
    chat: bool,
) -> Option<String> {
    if chat {
        if let Some(model) = crate::env_file::value(workspace, "HARNESS_CHAT_MODEL") {
            return Some(model);
        }
    }
    if let Some(name) = model_env {
        if let Some(model) = crate::env_file::value(workspace, name) {
            return Some(model);
        }
    }
    match backend {
        Backend::Anthropic => crate::env_file::value(workspace, "HARNESS_CLAUDE_MODEL"),
        Backend::Minimax => crate::env_file::value(workspace, "HARNESS_MINIMAX_MODEL")
            .or_else(|| Some("MiniMax-M3".to_owned())),
    }
}

pub(crate) fn configure_backend(
    command: &mut Command,
    workspace: &Path,
    backend: Backend,
    model: Option<&str>,
) -> Result<(), String> {
    match backend {
        Backend::Anthropic => {
            if let Some(key) = crate::env_file::value(workspace, "ANTHROPIC_API_KEY") {
                command.env("ANTHROPIC_API_KEY", key);
            }
        }
        Backend::Minimax => {
            let key = crate::env_file::value(workspace, "MINIMAX_API_KEY")
                .or_else(|| crate::env_file::value(workspace, "HARNESS_MINIMAX_API_KEY"))
                .ok_or_else(|| {
                    "MINIMAX_API_KEY não definida; configure no .env ou no ambiente".to_owned()
                })?;
            let base = crate::env_file::value(workspace, "HARNESS_MINIMAX_BASE_URL")
                .unwrap_or_else(|| "https://api.minimax.io/anthropic".to_owned());
            let model = model.unwrap_or("MiniMax-M3");
            command
                .env_remove("ANTHROPIC_API_KEY")
                .env("ANTHROPIC_BASE_URL", base)
                .env("ANTHROPIC_AUTH_TOKEN", key)
                .env("ANTHROPIC_MODEL", model)
                .env("ANTHROPIC_SMALL_FAST_MODEL", model);
        }
    }
    Ok(())
}

struct ClaudeCli {
    binary: OsString,
    backend: Backend,
    model_env: Option<String>,
}

impl ClaudeCli {
    fn new(model_env: Option<&str>) -> Self {
        Self {
            binary: std::env::var_os("HARNESS_CLAUDE_BIN").unwrap_or_else(|| "claude".into()),
            backend: Backend::Anthropic,
            model_env: model_env.map(str::to_owned),
        }
    }

    fn minimax(model_env: Option<&str>) -> Self {
        Self {
            binary: std::env::var_os("HARNESS_CLAUDE_BIN").unwrap_or_else(|| "claude".into()),
            backend: Backend::Minimax,
            model_env: model_env.map(str::to_owned),
        }
    }

    fn invoke<T: DeserializeOwned>(
        &self,
        workspace: &Path,
        mode: Mode,
        schema_name: &str,
        prompt: &str,
    ) -> Result<T, String> {
        let schema = json_schemas()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|(name, _)| *name == schema_name)
            .ok_or_else(|| format!("schema não encontrado: {schema_name}"))?
            .1;
        let (permission, tools) = match mode {
            Mode::ReadOnly => ("plan", "Read,Glob,Grep"),
            Mode::WorkspaceWrite => ("acceptEdits", "Read,Glob,Grep,Edit,Write,Bash"),
        };
        let mut command = Command::new(&self.binary);
        command
            .current_dir(workspace)
            .args([
                "-p",
                "--output-format",
                "json",
                "--no-session-persistence",
                "--json-schema",
            ])
            .arg(schema)
            .args(["--permission-mode", permission])
            .arg(format!("--allowedTools={tools}"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let model = resolve_model(workspace, self.backend, self.model_env.as_deref(), false);
        if let Some(model) = &model {
            command.args(["--model", model]);
        }
        configure_backend(&mut command, workspace, self.backend, model.as_deref())?;
        command.arg(prompt);

        metrics::before_call(self.backend.provider_name())?;
        let started = Instant::now();
        let output = metrics::run_provider_command(&mut command, None)
            .map_err(|error| format!("Claude CLI: {error}"))?;
        if !output.status.success() {
            let diagnostics = String::from_utf8_lossy(&output.stderr)
                .lines()
                .filter(|line| !line.trim().is_empty())
                .take(3)
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!(
                "Claude CLI encerrou com {}: {}",
                output.status,
                if diagnostics.is_empty() {
                    "diagnóstico indisponível".to_owned()
                } else {
                    diagnostics
                }
            ));
        }
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("Claude CLI retornou JSON inválido: {error}"))?;
        if envelope["is_error"].as_bool().unwrap_or(false) {
            return Err(format!(
                "Claude CLI recusou a tarefa: {}",
                envelope["result"].as_str().unwrap_or("sem detalhes")
            ));
        }
        let structured = envelope
            .get("structured_output")
            .filter(|value| !value.is_null())
            .ok_or_else(|| {
                format!(
                    "Claude CLI não retornou structured_output; resultado: {}",
                    envelope["result"].as_str().unwrap_or("vazio")
                )
            })?;
        let parsed = serde_json::from_value(structured.clone())
            .map_err(|error| format!("Claude CLI retornou structured_output inválido: {error}"))?;
        metrics::record(CallMetric {
            role: metrics::role_for_schema(schema_name),
            provider: self.backend.provider_name().to_owned(),
            model,
            duration_ms: started.elapsed().as_millis(),
            cost_usd: envelope["total_cost_usd"].as_f64(),
            input_tokens: envelope["usage"]["input_tokens"].as_u64(),
            output_tokens: envelope["usage"]["output_tokens"].as_u64(),
        })?;
        Ok(parsed)
    }
}

pub struct ClaudeCliAnalyst {
    cli: ClaudeCli,
}

impl ClaudeCliAnalyst {
    pub fn new(model_env: &str) -> Self {
        Self {
            cli: ClaudeCli::new(Some(model_env)),
        }
    }

    pub fn minimax(model_env: &str) -> Self {
        Self {
            cli: ClaudeCli::minimax(Some(model_env)),
        }
    }
}

impl Analyst for ClaudeCliAnalyst {
    fn understand(
        &mut self,
        request: &str,
        workspace: &Path,
        previous: Option<&Understanding>,
    ) -> Result<Understanding, String> {
        let prompt = understanding_prompt(request, workspace, previous)?;
        self.cli.invoke(
            workspace,
            Mode::ReadOnly,
            "understanding.schema.json",
            &prompt,
        )
    }

    fn contract(
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
        self.cli.invoke(
            workspace,
            Mode::ReadOnly,
            "contract_bundle.schema.json",
            &prompt,
        )
    }
}

pub struct ClaudeCliAcceptanceAuthor {
    cli: ClaudeCli,
}

impl ClaudeCliAcceptanceAuthor {
    pub fn new(model_env: &str) -> Self {
        Self {
            cli: ClaudeCli::new(Some(model_env)),
        }
    }

    pub fn minimax(model_env: &str) -> Self {
        Self {
            cli: ClaudeCli::minimax(Some(model_env)),
        }
    }
}

impl AcceptanceAuthor for ClaudeCliAcceptanceAuthor {
    fn implement(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
        previous: Option<&SensorImplementation>,
        feedback: Option<&str>,
    ) -> Result<SensorImplementation, String> {
        let prompt = acceptance_prompt(workspace, bundle, locale, previous, feedback)?;
        self.cli.invoke(
            workspace,
            Mode::ReadOnly,
            "sensor_implementation.schema.json",
            &prompt,
        )
    }
}

pub struct ClaudeCliPlanner {
    cli: ClaudeCli,
}

impl ClaudeCliPlanner {
    pub fn new(model_env: &str) -> Self {
        Self {
            cli: ClaudeCli::new(Some(model_env)),
        }
    }

    pub fn minimax(model_env: &str) -> Self {
        Self {
            cli: ClaudeCli::minimax(Some(model_env)),
        }
    }
}

impl Planner for ClaudeCliPlanner {
    fn plan(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
    ) -> Result<Plan, String> {
        let prompt = planner_prompt(workspace, bundle, locale)?;
        self.cli
            .invoke(workspace, Mode::ReadOnly, "plan.schema.json", &prompt)
    }
}

pub struct ClaudeCliExecutor {
    cli: ClaudeCli,
}

impl ClaudeCliExecutor {
    pub fn new() -> Self {
        Self {
            cli: ClaudeCli::new(None),
        }
    }

    pub fn minimax() -> Self {
        Self {
            cli: ClaudeCli::minimax(None),
        }
    }
}

impl Executor for ClaudeCliExecutor {
    fn execute(
        &mut self,
        context: &ExecutionContext,
        workspace: &Path,
    ) -> Result<StepReport, String> {
        let prompt = executor_prompt(context)?;
        self.cli.invoke(
            workspace,
            Mode::WorkspaceWrite,
            "step_report.schema.json",
            &prompt,
        )
    }
}

pub struct ClaudeCliVerifier {
    cli: ClaudeCli,
}

impl ClaudeCliVerifier {
    pub fn new(model_env: &str) -> Self {
        Self {
            cli: ClaudeCli::new(Some(model_env)),
        }
    }

    pub fn minimax(model_env: &str) -> Self {
        Self {
            cli: ClaudeCli::minimax(Some(model_env)),
        }
    }
}

impl Verifier for ClaudeCliVerifier {
    fn verify(
        &mut self,
        contract: &Contract,
        suite: &SensorSuite,
        step_report: &StepReport,
        sensor_results: &[SensorResult],
        workspace: &Path,
    ) -> Result<Verdict, String> {
        let prompt = verifier_prompt(contract, suite, step_report, sensor_results)?;
        self.cli
            .invoke(workspace, Mode::ReadOnly, "verdict.schema.json", &prompt)
    }
}
