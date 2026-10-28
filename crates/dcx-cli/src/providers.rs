use std::path::Path;

use dcx_artifacts::{ContractBundle, Plan, SensorImplementation, Understanding};
use dcx_engine::{Executor, Verifier};

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

#[derive(Debug, PartialEq, Eq)]
enum Provider {
    Codex,
    Claude,
    Minimax,
    Auto,
}

fn selected() -> Result<Provider, String> {
    let value = crate::env_file::value(Path::new("."), "DCX_PROVIDER")
        .unwrap_or_else(|| "codex".to_owned());
    parse_provider(&value)
}

fn parse_provider(value: &str) -> Result<Provider, String> {
    match value.to_ascii_lowercase().as_str() {
        "codex" => Ok(Provider::Codex),
        "claude" => Ok(Provider::Claude),
        "minimax" => Ok(Provider::Minimax),
        "auto" => Ok(Provider::Auto),
        other => Err(format!(
            "DCX_PROVIDER desconhecido: {other}; use codex, claude, minimax ou auto"
        )),
    }
}

pub fn analyst() -> Result<Box<dyn Analyst>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliAnalyst::default()),
        Provider::Claude => Box::new(ClaudeCliAnalyst::new("DCX_ANALYST_MODEL")),
        Provider::Minimax => Box::new(ClaudeCliAnalyst::minimax("DCX_ANALYST_MODEL")),
        Provider::Auto => Box::new(FallbackAnalyst {
            primary: Box::new(ClaudeCliAnalyst::new("DCX_ANALYST_MODEL")),
            fallback: Box::new(CodexCliAnalyst::default()),
        }),
    })
}

pub fn acceptance_author() -> Result<Box<dyn AcceptanceAuthor>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliAcceptanceAuthor::default()),
        Provider::Claude => Box::new(ClaudeCliAcceptanceAuthor::new("DCX_ACCEPTANCE_MODEL")),
        Provider::Minimax => Box::new(ClaudeCliAcceptanceAuthor::minimax("DCX_ACCEPTANCE_MODEL")),
        Provider::Auto => Box::new(FallbackAcceptanceAuthor {
            primary: Box::new(ClaudeCliAcceptanceAuthor::new("DCX_ACCEPTANCE_MODEL")),
            fallback: Box::new(CodexCliAcceptanceAuthor::default()),
        }),
    })
}

pub fn planner() -> Result<Box<dyn Planner>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliPlanner::default()),
        Provider::Claude => Box::new(ClaudeCliPlanner::new("DCX_PLANNER_MODEL")),
        Provider::Minimax => Box::new(ClaudeCliPlanner::minimax("DCX_PLANNER_MODEL")),
        Provider::Auto => Box::new(FallbackPlanner {
            primary: Box::new(ClaudeCliPlanner::new("DCX_PLANNER_MODEL")),
            fallback: Box::new(CodexCliPlanner::default()),
        }),
    })
}

pub fn executor() -> Result<Box<dyn Executor>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliExecutor::default()),
        Provider::Claude => Box::new(ClaudeCliExecutor::new()),
        Provider::Minimax => Box::new(ClaudeCliExecutor::minimax()),
        Provider::Auto => Box::new(FallbackExecutor {
            primary: Box::new(ClaudeCliExecutor::new()),
            fallback: Box::new(CodexCliExecutor::default()),
        }),
    })
}

pub fn verifier() -> Result<Box<dyn Verifier>, String> {
    Ok(match selected()? {
        Provider::Codex => Box::new(CodexCliVerifier::default()),
        Provider::Claude => Box::new(ClaudeCliVerifier::new("DCX_VERIFIER_MODEL")),
        Provider::Minimax => Box::new(ClaudeCliVerifier::minimax("DCX_VERIFIER_MODEL")),
        Provider::Auto => Box::new(FallbackVerifier {
            primary: Box::new(ClaudeCliVerifier::new("DCX_VERIFIER_MODEL")),
            fallback: Box::new(CodexCliVerifier::default()),
        }),
    })
}

fn try_fallback<T>(
    result: Result<T, String>,
    fallback: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(primary_error) => fallback().map_err(|fallback_error| {
            format!("provider primário falhou: {primary_error}; fallback falhou: {fallback_error}")
        }),
    }
}

struct FallbackAnalyst {
    primary: Box<dyn Analyst>,
    fallback: Box<dyn Analyst>,
}

impl Analyst for FallbackAnalyst {
    fn understand(
        &mut self,
        request: &str,
        workspace: &Path,
        previous: Option<&Understanding>,
    ) -> Result<Understanding, String> {
        try_fallback(
            self.primary.understand(request, workspace, previous),
            || self.fallback.understand(request, workspace, previous),
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
        try_fallback(
            self.primary.contract(
                request,
                workspace,
                understanding,
                previous,
                rejection_feedback,
                validation_feedback,
            ),
            || {
                self.fallback.contract(
                    request,
                    workspace,
                    understanding,
                    previous,
                    rejection_feedback,
                    validation_feedback,
                )
            },
        )
    }
}

struct FallbackAcceptanceAuthor {
    primary: Box<dyn AcceptanceAuthor>,
    fallback: Box<dyn AcceptanceAuthor>,
}

impl AcceptanceAuthor for FallbackAcceptanceAuthor {
    fn implement(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
        previous: Option<&SensorImplementation>,
        feedback: Option<&str>,
    ) -> Result<SensorImplementation, String> {
        try_fallback(
            self.primary
                .implement(workspace, bundle, locale, previous, feedback),
            || {
                self.fallback
                    .implement(workspace, bundle, locale, previous, feedback)
            },
        )
    }
}

struct FallbackPlanner {
    primary: Box<dyn Planner>,
    fallback: Box<dyn Planner>,
}

impl Planner for FallbackPlanner {
    fn plan(
        &mut self,
        workspace: &Path,
        bundle: &ContractBundle,
        locale: &str,
    ) -> Result<Plan, String> {
        try_fallback(self.primary.plan(workspace, bundle, locale), || {
            self.fallback.plan(workspace, bundle, locale)
        })
    }
}

struct FallbackExecutor {
    primary: Box<dyn Executor>,
    fallback: Box<dyn Executor>,
}

impl Executor for FallbackExecutor {
    fn execute(
        &mut self,
        context: &dcx_engine::ExecutionContext,
        workspace: &Path,
    ) -> Result<dcx_artifacts::StepReport, String> {
        try_fallback(self.primary.execute(context, workspace), || {
            self.fallback.execute(context, workspace)
        })
    }
}

struct FallbackVerifier {
    primary: Box<dyn Verifier>,
    fallback: Box<dyn Verifier>,
}

impl Verifier for FallbackVerifier {
    fn verify(
        &mut self,
        contract: &dcx_artifacts::Contract,
        suite: &dcx_artifacts::SensorSuite,
        step_report: &dcx_artifacts::StepReport,
        sensor_results: &[dcx_artifacts::SensorResult],
        workspace: &Path,
    ) -> Result<dcx_artifacts::Verdict, String> {
        try_fallback(
            self.primary
                .verify(contract, suite, step_report, sensor_results, workspace),
            || {
                self.fallback
                    .verify(contract, suite, step_report, sensor_results, workspace)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_runs_after_a_primary_error() {
        let result: Result<u8, String> = try_fallback(Err("quota".into()), || Ok(7));
        assert_eq!(result, Ok(7));
    }

    #[test]
    fn fallback_keeps_the_primary_result_when_it_succeeds() {
        let result: Result<u8, String> = try_fallback(Ok(3), || Ok(7));
        assert_eq!(result, Ok(3));
    }

    #[test]
    fn fallback_combines_both_errors() {
        let result: Result<u8, String> =
            try_fallback(Err("quota".into()), || Err("offline".into()));
        let error = result.expect_err("ambos falharam");
        assert!(error.contains("quota"));
        assert!(error.contains("offline"));
    }

    #[test]
    fn parses_minimax_provider() {
        assert_eq!(
            parse_provider("minimax").expect("provider"),
            Provider::Minimax
        );
        assert_eq!(
            parse_provider("MiniMax").expect("provider"),
            Provider::Minimax
        );
    }

    #[test]
    fn rejects_unknown_provider() {
        assert!(parse_provider("other").is_err());
    }
}
