use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

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
use crate::providers::{AcceptanceAuthor, Analyst, Planner};

enum Mode {
    ReadOnly,
    WorkspaceWrite,
}

struct ClaudeCli {
    binary: OsString,
    model: Option<String>,
}

impl ClaudeCli {
    fn new(model_env: Option<&str>) -> Self {
        Self {
            binary: std::env::var_os("HARNESS_CLAUDE_BIN").unwrap_or_else(|| "claude".into()),
            model: model_env
                .and_then(|name| std::env::var(name).ok())
                .or_else(|| std::env::var("HARNESS_CLAUDE_MODEL").ok()),
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
        if let Some(model) = &self.model {
            command.args(["--model", model]);
        }
        command.arg(prompt);

        let output = command
            .output()
            .map_err(|error| format!("não foi possível iniciar Claude CLI: {error}"))?;
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
        serde_json::from_value(structured.clone())
            .map_err(|error| format!("Claude CLI retornou structured_output inválido: {error}"))
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
