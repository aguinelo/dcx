//! Deterministic state model for the Harness MVP.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub use harness_artifacts::{
    Contract, Criterion, DeliveryReport, SensorResult, SensorSpec, SensorSuite, SensorVisibility,
    StepReport, Verdict,
};

use std::path::{Path, PathBuf};

/// Input visible to an executor. Hidden sensors are deliberately absent.
#[derive(Clone, Debug)]
pub struct ExecutionContext {
    pub contract: Contract,
    pub visible_sensors: Vec<SensorSpec>,
    pub step_id: String,
    pub attempt: u32,
    pub feedback: Vec<String>,
}

impl ExecutionContext {
    pub fn new(
        contract: &Contract,
        suite: &SensorSuite,
        step_id: &str,
        attempt: u32,
        feedback: Vec<String>,
    ) -> Result<Self, harness_artifacts::ValidationError> {
        suite.validate_for(contract)?;
        Ok(Self {
            contract: contract.clone(),
            visible_sensors: suite
                .sensors
                .iter()
                .filter(|sensor| sensor.visibility == SensorVisibility::Visible)
                .cloned()
                .collect(),
            step_id: step_id.to_owned(),
            attempt,
            feedback,
        })
    }
}

/// A single worker implementation, such as a model-backed executor.
pub trait Executor {
    fn execute(
        &mut self,
        context: &ExecutionContext,
        workspace: &Path,
    ) -> Result<StepReport, String>;
}

/// Runs one acceptance sensor against the current workspace.
pub trait SensorRunner {
    fn run_sensor(&mut self, sensor: &SensorSpec, workspace: &Path)
    -> Result<SensorResult, String>;
}

/// Independent verifier interface. Its input excludes session history and hidden sensor definitions.
pub trait Verifier {
    fn verify(
        &mut self,
        contract: &Contract,
        suite: &SensorSuite,
        step_report: &StepReport,
        sensor_results: &[SensorResult],
        workspace: &Path,
    ) -> Result<Verdict, String>;
}

#[derive(Clone, Debug)]
pub struct RunExecution {
    pub step_reports: Vec<StepReport>,
    pub sensor_results: Vec<SensorResult>,
    pub verdict: Verdict,
    pub states: Vec<RunState>,
}

pub struct RunRequest<'a> {
    pub contract: &'a Contract,
    pub suite: &'a SensorSuite,
    pub step_id: &'a str,
    pub workspace: &'a Path,
}

pub struct RunParticipants<'a> {
    pub executor: &'a mut dyn Executor,
    pub sensor_runner: &'a mut dyn SensorRunner,
    pub verifier: &'a mut dyn Verifier,
}

/// Coordinates one planned step, retries visible failures, then verifies with hidden sensors.
pub struct RunCoordinator {
    max_attempts: u32,
}

impl RunCoordinator {
    pub fn new(max_attempts: u32) -> Result<Self, String> {
        if max_attempts == 0 {
            return Err("max_attempts must be greater than zero".to_owned());
        }
        Ok(Self { max_attempts })
    }

    pub fn execute(
        &self,
        request: RunRequest<'_>,
        participants: RunParticipants<'_>,
    ) -> Result<RunExecution, String> {
        let RunRequest {
            contract,
            suite,
            step_id,
            workspace,
        } = request;
        let RunParticipants {
            executor,
            sensor_runner,
            verifier,
        } = participants;
        contract.validate().map_err(|error| error.to_string())?;
        suite
            .validate_for(contract)
            .map_err(|error| error.to_string())?;
        let mut lifecycle = RunLifecycle::new();
        for next in [
            RunState::Understanding,
            RunState::Contracting,
            RunState::AcceptanceAuthoring,
            RunState::Planning,
            RunState::Executing,
        ] {
            lifecycle
                .transition(next)
                .map_err(|error| error.to_string())?;
        }

        let visible = suite
            .sensors
            .iter()
            .filter(|sensor| sensor.visibility == SensorVisibility::Visible)
            .collect::<Vec<_>>();
        let hidden = suite
            .sensors
            .iter()
            .filter(|sensor| sensor.visibility == SensorVisibility::Hidden)
            .collect::<Vec<_>>();
        let mut step_reports = Vec::new();
        let mut feedback = Vec::new();
        let mut changed_files = BTreeSet::new();

        for attempt in 1..=self.max_attempts {
            let before = snapshot_workspace(workspace)?;
            let context =
                ExecutionContext::new(contract, suite, step_id, attempt, feedback.clone())
                    .map_err(|error| error.to_string())?;
            let mut step_report = executor.execute(&context, workspace)?;
            let after = snapshot_workspace(workspace)?;
            changed_files.extend(changed_paths(&before, &after));
            step_report.files_changed = changed_files
                .iter()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .collect();
            step_report.attempts = attempt;
            if step_report.step_id != step_id {
                return Err(format!(
                    "executor returned step_id {}; expected {step_id}",
                    step_report.step_id
                ));
            }
            step_reports.push(step_report.clone());

            let mut sensor_results = visible
                .iter()
                .map(|sensor| sensor_runner.run_sensor(sensor, workspace))
                .collect::<Result<Vec<_>, _>>()?;
            let visible_passed = visible.iter().all(|sensor| {
                sensor_results.iter().any(|result| {
                    result.sensor_id == sensor.id
                        && result.criterion_id == sensor.criterion_id
                        && result.passed
                })
            });

            if !visible_passed {
                feedback = sensor_results
                    .iter()
                    .filter(|result| !result.passed)
                    .map(|result| {
                        format!(
                            "{}: {} — {}",
                            result.criterion_id, result.sensor_id, result.evidence
                        )
                    })
                    .collect();
                if attempt < self.max_attempts {
                    continue;
                }
                let verdict = visible_failure_verdict(contract, suite, &sensor_results);
                lifecycle
                    .transition(RunState::Failed)
                    .map_err(|error| error.to_string())?;
                return Ok(RunExecution {
                    step_reports,
                    sensor_results,
                    verdict,
                    states: lifecycle.history().to_vec(),
                });
            }

            lifecycle
                .transition(RunState::Verifying)
                .map_err(|error| error.to_string())?;
            sensor_results.extend(
                hidden
                    .iter()
                    .map(|sensor| sensor_runner.run_sensor(sensor, workspace))
                    .collect::<Result<Vec<_>, _>>()?,
            );

            let mut verdict =
                verifier.verify(contract, suite, &step_report, &sensor_results, workspace)?;
            let verdict_covers_all_criteria = verdict.criteria.len() == contract.criteria.len()
                && contract.criteria.iter().all(|criterion| {
                    verdict
                        .criteria
                        .iter()
                        .filter(|result| result.criterion_id == criterion.id)
                        .count()
                        == 1
                });
            let verdict_criteria_passed = verdict_covers_all_criteria
                && verdict.criteria.iter().all(|criterion| criterion.passed);
            let all_acceptance_sensors_passed = suite.sensors.iter().all(|sensor| {
                sensor_results.iter().any(|result| {
                    result.sensor_id == sensor.id
                        && result.criterion_id == sensor.criterion_id
                        && result.passed
                })
            });
            if !all_acceptance_sensors_passed || !verdict_criteria_passed {
                verdict.approved = false;
                for sensor in suite.sensors.iter().filter(|sensor| {
                    !sensor_results.iter().any(|result| {
                        result.sensor_id == sensor.id
                            && result.criterion_id == sensor.criterion_id
                            && result.passed
                    })
                }) {
                    if !verdict
                        .issues
                        .iter()
                        .any(|issue| issue.criterion_id == sensor.criterion_id)
                    {
                        verdict.issues.push(harness_artifacts::VerdictIssue {
                            criterion_id: sensor.criterion_id.clone(),
                            description: format!(
                                "Acceptance sensor {} failed or returned no passing result.",
                                sensor.id
                            ),
                            severity: harness_artifacts::IssueSeverity::Blocker,
                        });
                    }
                }
                for criterion in contract.criteria.iter().filter(|criterion| {
                    !verdict
                        .criteria
                        .iter()
                        .any(|result| result.criterion_id == criterion.id && result.passed)
                }) {
                    if !verdict
                        .issues
                        .iter()
                        .any(|issue| issue.criterion_id == criterion.id)
                    {
                        verdict.issues.push(harness_artifacts::VerdictIssue {
                            criterion_id: criterion.id.clone(),
                            description: "Verifier did not mark this contract criterion as passed."
                                .to_owned(),
                            severity: harness_artifacts::IssueSeverity::Blocker,
                        });
                    }
                }
            }
            if verdict.approved && all_acceptance_sensors_passed && verdict_criteria_passed {
                lifecycle
                    .transition(RunState::AwaitingMerge)
                    .map_err(|error| error.to_string())?;
                lifecycle
                    .transition(RunState::Done)
                    .map_err(|error| error.to_string())?;
                return Ok(RunExecution {
                    step_reports,
                    sensor_results,
                    verdict,
                    states: lifecycle.history().to_vec(),
                });
            }

            feedback = verdict
                .issues
                .iter()
                .map(|issue| format!("{}: {}", issue.criterion_id, issue.description))
                .collect();
            if feedback.is_empty() {
                feedback.push("Verifier did not approve the implementation; re-check every contract criterion.".to_owned());
            }
            if attempt == self.max_attempts {
                lifecycle
                    .transition(RunState::Failed)
                    .map_err(|error| error.to_string())?;
                return Ok(RunExecution {
                    step_reports,
                    sensor_results,
                    verdict,
                    states: lifecycle.history().to_vec(),
                });
            }
            if lifecycle.current() == RunState::Verifying {
                lifecycle
                    .transition(RunState::Executing)
                    .map_err(|error| error.to_string())?;
            }
        }

        Err("run ended without a verdict".to_owned())
    }
}

fn visible_failure_verdict(
    contract: &Contract,
    suite: &SensorSuite,
    sensor_results: &[SensorResult],
) -> Verdict {
    let criteria = contract
        .criteria
        .iter()
        .map(|criterion| {
            let sensors = suite
                .sensors
                .iter()
                .filter(|sensor| sensor.criterion_id == criterion.id)
                .collect::<Vec<_>>();
            let evidence = sensor_results
                .iter()
                .filter(|result| result.criterion_id == criterion.id)
                .map(|result| format!("{}: {}", result.sensor_id, result.evidence))
                .collect::<Vec<_>>();
            let passed = !sensors.is_empty()
                && sensors.iter().all(|sensor| {
                    sensor_results.iter().any(|result| {
                        result.sensor_id == sensor.id
                            && result.criterion_id == criterion.id
                            && result.passed
                    })
                });
            harness_artifacts::CriterionVerdict {
                criterion_id: criterion.id.clone(),
                passed,
                evidence,
            }
        })
        .collect::<Vec<_>>();
    let issues = criteria
        .iter()
        .filter(|criterion| !criterion.passed)
        .map(|criterion| harness_artifacts::VerdictIssue {
            criterion_id: criterion.criterion_id.clone(),
            description: "Visible acceptance sensors failed after the final attempt.".to_owned(),
            severity: harness_artifacts::IssueSeverity::Blocker,
        })
        .collect();
    Verdict {
        schema_version: harness_artifacts::SCHEMA_VERSION,
        approved: false,
        criteria,
        issues,
        notes: vec![],
    }
}

const MAX_SNAPSHOT_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SNAPSHOT_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

fn snapshot_workspace(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<PathBuf, Vec<u8>>,
        total_bytes: &mut u64,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(directory).map_err(|error| {
            format!("cannot inspect workspace {}: {error}", directory.display())
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("cannot read workspace entry: {error}"))?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                format!("cannot inspect workspace path {}: {error}", path.display())
            })?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                visit(root, &path, files, total_bytes)?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_SNAPSHOT_FILE_BYTES {
                    return Err(format!(
                        "workspace file exceeds snapshot limit: {}",
                        path.display()
                    ));
                }
                *total_bytes = total_bytes.saturating_add(metadata.len());
                if *total_bytes > MAX_SNAPSHOT_TOTAL_BYTES {
                    return Err("workspace exceeds total snapshot limit".to_owned());
                }
                let relative = path
                    .strip_prefix(root)
                    .map_err(|error| format!("workspace path escaped root: {error}"))?
                    .to_path_buf();
                let contents = std::fs::read(&path).map_err(|error| {
                    format!("cannot read workspace file {}: {error}", path.display())
                })?;
                files.insert(relative, contents);
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files, &mut 0)?;
    Ok(files)
}

fn changed_paths(
    before: &BTreeMap<PathBuf, Vec<u8>>,
    after: &BTreeMap<PathBuf, Vec<u8>>,
) -> BTreeSet<PathBuf> {
    before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect()
}

/// States included in the current MVP scope.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RunState {
    New,
    Understanding,
    Contracting,
    AcceptanceAuthoring,
    Planning,
    Executing,
    Verifying,
    AwaitingHuman,
    AwaitingMerge,
    Done,
    Failed,
    Cancelled,
}

impl RunState {
    pub const ALL: [Self; 12] = [
        Self::New,
        Self::Understanding,
        Self::Contracting,
        Self::AcceptanceAuthoring,
        Self::Planning,
        Self::Executing,
        Self::Verifying,
        Self::AwaitingHuman,
        Self::AwaitingMerge,
        Self::Done,
        Self::Failed,
        Self::Cancelled,
    ];

    /// Returns whether the engine permits a direct transition to `next`.
    ///
    /// Gate-specific return states are handled by the run coordinator, which
    /// records the state to resume before entering `AwaitingHuman`.
    pub const fn can_transition_to(self, next: Self) -> bool {
        use RunState::*;

        matches!(
            (self, next),
            (New, Understanding)
                | (Understanding, Contracting)
                | (Contracting, AcceptanceAuthoring)
                | (AcceptanceAuthoring, Planning)
                | (Planning, Executing)
                | (Planning, Failed)
                | (Executing, Planning)
                | (Executing, Verifying)
                | (Executing, AwaitingHuman)
                | (Executing, Failed)
                | (Verifying, Executing)
                | (Verifying, AwaitingMerge)
                | (Verifying, Failed)
                | (AwaitingMerge, Done)
                | (AwaitingMerge, Cancelled)
        ) || (!self.is_terminal() && matches!(next, Cancelled))
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Understanding => "understanding",
            Self::Contracting => "contracting",
            Self::AcceptanceAuthoring => "acceptance_authoring",
            Self::Planning => "planning",
            Self::Executing => "executing",
            Self::Verifying => "verifying",
            Self::AwaitingHuman => "awaiting_human",
            Self::AwaitingMerge => "awaiting_merge",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Records and validates state changes for one run.
#[derive(Clone, Debug)]
pub struct RunLifecycle {
    current: RunState,
    history: Vec<RunState>,
}

impl Default for RunLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl RunLifecycle {
    pub fn new() -> Self {
        Self {
            current: RunState::New,
            history: vec![RunState::New],
        }
    }

    pub const fn current(&self) -> RunState {
        self.current
    }

    pub fn history(&self) -> &[RunState] {
        &self.history
    }

    pub fn transition(&mut self, next: RunState) -> Result<(), InvalidTransition> {
        if !self.current.can_transition_to(next) {
            return Err(InvalidTransition {
                from: self.current,
                to: next,
            });
        }
        self.current = next;
        self.history.push(next);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTransition {
    pub from: RunState,
    pub to: RunState,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "transição inválida: {} -> {}", self.from, self.to)
    }
}

impl std::error::Error for InvalidTransition {}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionContext, Executor, RunCoordinator, RunParticipants, RunRequest, RunState,
        SensorRunner, Verifier,
    };
    use harness_artifacts::{
        Contract, Criterion, CriterionVerdict, SCHEMA_VERSION, SensorKind, SensorResult,
        SensorSpec, SensorSuite, SensorType, SensorVisibility, StepReport, Verdict,
    };
    use std::path::Path;

    struct MinimalExecutor;

    impl Executor for MinimalExecutor {
        fn execute(
            &mut self,
            _context: &ExecutionContext,
            _workspace: &Path,
        ) -> Result<StepReport, String> {
            Ok(StepReport {
                schema_version: SCHEMA_VERSION,
                step_id: "step-1".into(),
                attempts: 1,
                summary: "completed".into(),
                files_changed: vec!["invented.py".into()],
                decisions: vec![],
                disputes: vec![],
            })
        }
    }

    #[derive(Default)]
    struct CountingExecutor {
        attempts: Vec<u32>,
        feedback: Vec<Vec<String>>,
    }

    impl Executor for CountingExecutor {
        fn execute(
            &mut self,
            context: &ExecutionContext,
            _workspace: &Path,
        ) -> Result<StepReport, String> {
            self.attempts.push(context.attempt);
            self.feedback.push(context.feedback.clone());
            Ok(StepReport {
                schema_version: SCHEMA_VERSION,
                step_id: "step-1".into(),
                attempts: context.attempt,
                summary: "attempt".into(),
                files_changed: vec![],
                decisions: vec![],
                disputes: vec![],
            })
        }
    }

    #[derive(Default)]
    struct RetrySensorRunner {
        visible_runs: u32,
        hidden_runs: u32,
    }

    impl SensorRunner for RetrySensorRunner {
        fn run_sensor(
            &mut self,
            sensor: &SensorSpec,
            _workspace: &Path,
        ) -> Result<SensorResult, String> {
            let passed = match sensor.visibility {
                SensorVisibility::Visible => {
                    self.visible_runs += 1;
                    self.visible_runs > 1
                }
                SensorVisibility::Hidden => {
                    self.hidden_runs += 1;
                    true
                }
            };
            Ok(SensorResult {
                sensor_id: sensor.id.clone(),
                criterion_id: sensor.criterion_id.clone(),
                passed,
                evidence: "probe".into(),
            })
        }
    }

    #[derive(Default)]
    struct HiddenFailureSensorRunner {
        hidden_runs: u32,
    }

    impl SensorRunner for HiddenFailureSensorRunner {
        fn run_sensor(
            &mut self,
            sensor: &SensorSpec,
            _workspace: &Path,
        ) -> Result<SensorResult, String> {
            let passed = match sensor.visibility {
                SensorVisibility::Visible => true,
                SensorVisibility::Hidden => {
                    self.hidden_runs += 1;
                    false
                }
            };
            Ok(SensorResult {
                sensor_id: sensor.id.clone(),
                criterion_id: sensor.criterion_id.clone(),
                passed,
                evidence: "fixture result".into(),
            })
        }
    }

    struct AlwaysApproveVerifier;

    impl Verifier for AlwaysApproveVerifier {
        fn verify(
            &mut self,
            contract: &Contract,
            _suite: &SensorSuite,
            _step_report: &StepReport,
            _sensor_results: &[SensorResult],
            _workspace: &Path,
        ) -> Result<Verdict, String> {
            Ok(Verdict {
                schema_version: SCHEMA_VERSION,
                approved: true,
                criteria: contract
                    .criteria
                    .iter()
                    .map(|criterion| CriterionVerdict {
                        criterion_id: criterion.id.clone(),
                        passed: true,
                        evidence: vec!["verifier claim".into()],
                    })
                    .collect(),
                issues: vec![],
                notes: vec![],
            })
        }
    }

    #[test]
    fn mvp_happy_path_is_allowed() {
        let path = [
            RunState::New,
            RunState::Understanding,
            RunState::Contracting,
            RunState::AcceptanceAuthoring,
            RunState::Planning,
            RunState::Executing,
            RunState::Verifying,
            RunState::AwaitingMerge,
            RunState::Done,
        ];

        for pair in path.windows(2) {
            assert!(pair[0].can_transition_to(pair[1]));
        }
    }

    #[test]
    fn verification_rejection_returns_to_execution() {
        assert!(RunState::Verifying.can_transition_to(RunState::Executing));
    }

    #[test]
    fn terminal_runs_cannot_transition() {
        for terminal in [RunState::Done, RunState::Failed, RunState::Cancelled] {
            assert!(terminal.is_terminal());
            assert!(!terminal.can_transition_to(RunState::Executing));
            assert!(!terminal.can_transition_to(RunState::Cancelled));
        }
    }

    #[test]
    fn unrelated_direct_transitions_are_rejected() {
        assert!(!RunState::New.can_transition_to(RunState::Executing));
        assert!(!RunState::Planning.can_transition_to(RunState::Verifying));
        assert!(!RunState::AwaitingMerge.can_transition_to(RunState::Executing));
    }

    #[test]
    fn executor_context_contains_visible_but_no_hidden_sensors() {
        let contract = Contract {
            schema_version: SCHEMA_VERSION,
            objective: "objective".into(),
            assumptions: vec![],
            criteria: vec![Criterion {
                id: "criterion".into(),
                description: "observable".into(),
            }],
            interface: serde_json::Value::Null,
            constraints: vec![],
            out_of_scope: vec![],
        };
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![
                SensorSpec {
                    id: "visible".into(),
                    criterion_id: "criterion".into(),
                    sensor_type: SensorType::Test,
                    kind: SensorKind::Computational,
                    visibility: SensorVisibility::Visible,
                    purpose: "acceptance".into(),
                    check: "visible check".into(),
                },
                SensorSpec {
                    id: "hidden".into(),
                    criterion_id: "criterion".into(),
                    sensor_type: SensorType::Test,
                    kind: SensorKind::Computational,
                    visibility: SensorVisibility::Hidden,
                    purpose: "acceptance".into(),
                    check: "hidden check".into(),
                },
            ],
        };

        let context =
            ExecutionContext::new(&contract, &suite, "step-1", 1, vec![]).expect("context válido");
        assert_eq!(context.visible_sensors.len(), 1);
        assert_eq!(context.step_id, "step-1");
        assert_eq!(context.visible_sensors[0].id, "visible");
        assert!(
            !context
                .visible_sensors
                .iter()
                .any(|sensor| sensor.id == "hidden")
        );
    }

    #[test]
    fn failed_hidden_sensor_overrules_verifier_approval() {
        let contract = Contract {
            schema_version: SCHEMA_VERSION,
            objective: "objective".into(),
            assumptions: vec![],
            criteria: vec![Criterion {
                id: "criterion".into(),
                description: "observable".into(),
            }],
            interface: serde_json::Value::Null,
            constraints: vec![],
            out_of_scope: vec![],
        };
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![
                SensorSpec {
                    id: "visible".into(),
                    criterion_id: "criterion".into(),
                    sensor_type: SensorType::Test,
                    kind: SensorKind::Computational,
                    visibility: SensorVisibility::Visible,
                    purpose: "acceptance".into(),
                    check: "visible check".into(),
                },
                SensorSpec {
                    id: "hidden".into(),
                    criterion_id: "criterion".into(),
                    sensor_type: SensorType::Test,
                    kind: SensorKind::Computational,
                    visibility: SensorVisibility::Hidden,
                    purpose: "acceptance".into(),
                    check: "hidden check".into(),
                },
            ],
        };
        let mut executor = MinimalExecutor;
        let mut sensor_runner = HiddenFailureSensorRunner::default();
        let mut verifier = AlwaysApproveVerifier;
        let execution = RunCoordinator::new(1)
            .expect("valid coordinator")
            .execute(
                RunRequest {
                    contract: &contract,
                    suite: &suite,
                    step_id: "step-1",
                    workspace: Path::new("."),
                },
                RunParticipants {
                    executor: &mut executor,
                    sensor_runner: &mut sensor_runner,
                    verifier: &mut verifier,
                },
            )
            .expect("execution returns a failed verdict");

        assert_eq!(sensor_runner.hidden_runs, 1);
        assert!(!execution.verdict.approved);
        assert_eq!(execution.states.last(), Some(&RunState::Failed));
        assert!(execution.step_reports[0].files_changed.is_empty());
    }

    #[test]
    fn workspace_snapshot_detects_added_modified_and_deleted_files() {
        let root = std::env::temp_dir().join(format!(
            "harness-snapshot-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        std::fs::write(root.join("modified.txt"), "old").expect("create modified fixture");
        std::fs::write(root.join("deleted.txt"), "gone").expect("create deleted fixture");
        let before = super::snapshot_workspace(&root).expect("snapshot before");

        std::fs::write(root.join("modified.txt"), "new").expect("modify fixture");
        std::fs::write(root.join("added.txt"), "new file").expect("add fixture");
        std::fs::remove_file(root.join("deleted.txt")).expect("delete fixture");
        let after = super::snapshot_workspace(&root).expect("snapshot after");
        let changed = super::changed_paths(&before, &after);

        assert_eq!(
            changed,
            ["added.txt", "deleted.txt", "modified.txt"]
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect()
        );
        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[test]
    fn coordinator_retries_visible_failure_and_runs_hidden_only_after_visible_passes() {
        let contract = Contract {
            schema_version: SCHEMA_VERSION,
            objective: "objective".into(),
            assumptions: vec![],
            criteria: vec![Criterion {
                id: "criterion".into(),
                description: "observable".into(),
            }],
            interface: serde_json::Value::Null,
            constraints: vec![],
            out_of_scope: vec![],
        };
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![
                SensorSpec {
                    id: "visible".into(),
                    criterion_id: "criterion".into(),
                    sensor_type: SensorType::Test,
                    kind: SensorKind::Computational,
                    visibility: SensorVisibility::Visible,
                    purpose: "acceptance".into(),
                    check: "visible check".into(),
                },
                SensorSpec {
                    id: "hidden".into(),
                    criterion_id: "criterion".into(),
                    sensor_type: SensorType::Test,
                    kind: SensorKind::Computational,
                    visibility: SensorVisibility::Hidden,
                    purpose: "acceptance".into(),
                    check: "hidden check".into(),
                },
            ],
        };
        let mut executor = CountingExecutor::default();
        let mut sensor_runner = RetrySensorRunner::default();
        let mut verifier = AlwaysApproveVerifier;
        let execution = RunCoordinator::new(2)
            .expect("valid coordinator")
            .execute(
                RunRequest {
                    contract: &contract,
                    suite: &suite,
                    step_id: "step-1",
                    workspace: Path::new("."),
                },
                RunParticipants {
                    executor: &mut executor,
                    sensor_runner: &mut sensor_runner,
                    verifier: &mut verifier,
                },
            )
            .expect("run succeeds after retry");

        assert!(execution.verdict.approved);
        assert_eq!(executor.attempts, vec![1, 2]);
        assert!(executor.feedback[0].is_empty());
        assert!(!executor.feedback[1].is_empty());
        assert_eq!(sensor_runner.visible_runs, 2);
        assert_eq!(sensor_runner.hidden_runs, 1);
    }
}
