use std::path::Path;

use harness_artifacts::{ContractBundle, Plan, SensorImplementation, Understanding};
use harness_engine::{Executor, Verifier};

use crate::claude::{
    ClaudeCliAcceptanceAuthor, ClaudeCliAnalyst, ClaudeCliExecutor, ClaudeCliPlanner,
    ClaudeCliVerifier,
};
use crate::codex::{
    CodexCliAcceptanceAuthor, CodexCliAnalyst, CodexCliExecutor, CodexCliPlanner, CodexCliVerifier,
};

pub trait Analyst {
    fn understand(
        &mut self,
        request: &str,
        workspace: &Path,
        previous: Option<&Understanding>,
    ) -> Result<Understanding, String>;

    fn contract(
        &mut self,
        request: &str,
        workspace: &Path,
        understanding: &Understanding,
        previous: Option<&ContractBundle>,
        rejection_feedback: Option<&str>,
        validation_feedback: Option<&str>,
    ) -> Result<ContractBundle, String>;
}

pub trait AcceptanceAuthor {
    fn implement(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
        previous: Option<&SensorImplementation>,
        feedback: Option<&str>,
    ) -> Result<SensorImplementation, String>;
}

pub trait Planner {
    fn plan(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
    ) -> Result<Plan, String>;
}

impl Analyst for CodexCliAnalyst {
    fn understand(
        &mut self,
        request: &str,
        workspace: &Path,
        previous: Option<&Understanding>,
    ) -> Result<Understanding, String> {
        CodexCliAnalyst::understand(self, request, workspace, previous)
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
        CodexCliAnalyst::contract(
            self,
            request,
            workspace,
            understanding,
            previous,
            rejection_feedback,
            validation_feedback,
        )
    }
}

impl AcceptanceAuthor for CodexCliAcceptanceAuthor {
    fn implement(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
        previous: Option<&SensorImplementation>,
        feedback: Option<&str>,
    ) -> Result<SensorImplementation, String> {
        CodexCliAcceptanceAuthor::implement(self, workspace, bundle, locale, previous, feedback)
    }
}

impl Planner for CodexCliPlanner {
    fn plan(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
    ) -> Result<Plan, String> {
        CodexCliPlanner::plan(self, workspace, bundle, locale)
    }
}

enum Provider {
    Codex,
    Claude,
}

fn selected() -> Result<Provider, String> {
    match std::env::var("HARNESS_PROVIDER")
        .unwrap_or_else(|_| "codex".to_owned())
        .to_ascii_lowercase()
        .as_str()
    {
        "codex" => Ok(Provider::Codex),
        "claude" => Ok(Provider::Claude),
        other => Err(format!(
            "HARNESS_PROVIDER desconhecido: {other}; use codex ou claude"
        )),
    }
}

pub fn analyst() -> Result<Box<dyn Analyst>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliAnalyst::default()),
        Provider::Claude => Box::new(ClaudeCliAnalyst::new("HARNESS_ANALYST_MODEL")),
    })
}

pub fn acceptance_author() -> Result<Box<dyn AcceptanceAuthor>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliAcceptanceAuthor::default()),
        Provider::Claude => Box::new(ClaudeCliAcceptanceAuthor::new("HARNESS_ACCEPTANCE_MODEL")),
    })
}

pub fn planner() -> Result<Box<dyn Planner>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliPlanner::default()),
        Provider::Claude => Box::new(ClaudeCliPlanner::new("HARNESS_PLANNER_MODEL")),
    })
}

pub fn executor() -> Result<Box<dyn Executor>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliExecutor::default()),
        Provider::Claude => Box::new(ClaudeCliExecutor::new()),
    })
}

pub fn verifier() -> Result<Box<dyn Verifier>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliVerifier::default()),
        Provider::Claude => Box::new(ClaudeCliVerifier::new("HARNESS_VERIFIER_MODEL")),
    })
}
