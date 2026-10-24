//! Versioned MVP artifacts shared by the CLI and deterministic engine.

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Understanding {
    pub schema_version: u32,
    pub summary: String,
    pub goal: String,
    #[serde(default)]
    pub assumptions: Vec<Assumption>,
    #[serde(default)]
    pub blocking_questions: Vec<BlockingQuestion>,
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
    #[serde(default)]
    pub answers: Vec<Answer>,
    #[serde(default)]
    pub context_refs: Vec<String>,
    pub locale: String,
}

impl Understanding {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::new(
                "unsupported understanding schema_version",
            ));
        }
        if self.summary.trim().is_empty() || self.goal.trim().is_empty() {
            return Err(ValidationError::new(
                "understanding summary and goal must not be empty",
            ));
        }
        if self.locale.trim().is_empty() {
            return Err(ValidationError::new(
                "understanding locale must not be empty",
            ));
        }
        unique_non_empty(
            self.assumptions.iter().map(|item| item.id.as_str()),
            "assumption id",
        )?;
        unique_non_empty(
            self.blocking_questions.iter().map(|item| item.id.as_str()),
            "blocking question id",
        )?;
        unique_non_empty(
            self.answers.iter().map(|item| item.question_id.as_str()),
            "answered question id",
        )?;
        for question in &self.blocking_questions {
            if question.question.trim().is_empty() || question.why_blocking.trim().is_empty() {
                return Err(ValidationError::new(format!(
                    "blocking question {} needs question and why_blocking text",
                    question.id
                )));
            }
            if question
                .options
                .iter()
                .any(|option| option.trim().is_empty())
            {
                return Err(ValidationError::new(format!(
                    "blocking question {} has an empty option",
                    question.id
                )));
            }
        }
        for answer in &self.answers {
            if answer.answer.trim().is_empty() || answer.answered_by.trim().is_empty() {
                return Err(ValidationError::new(format!(
                    "answer for {} needs answer and answered_by text",
                    answer.question_id
                )));
            }
            if !self
                .blocking_questions
                .iter()
                .any(|question| question.id == answer.question_id)
            {
                return Err(ValidationError::new(format!(
                    "answer references unknown blocking question {}",
                    answer.question_id
                )));
            }
        }
        for question in &self.open_questions {
            if question.question.trim().is_empty() || question.default_assumption.trim().is_empty()
            {
                return Err(ValidationError::new(
                    "open question needs question and default_assumption text",
                ));
            }
        }
        Ok(())
    }

    pub fn unanswered_blocking_questions(&self) -> Vec<&BlockingQuestion> {
        self.blocking_questions
            .iter()
            .filter(|question| {
                !self
                    .answers
                    .iter()
                    .any(|answer| answer.question_id == question.id)
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Assumption {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct BlockingQuestion {
    pub id: String,
    pub question: String,
    pub why_blocking: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct OpenQuestion {
    pub question: String,
    pub default_assumption: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Answer {
    pub question_id: String,
    pub answer: String,
    pub answered_by: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Criterion {
    pub id: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Contract {
    pub schema_version: u32,
    pub objective: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    pub criteria: Vec<Criterion>,
    #[serde(default)]
    pub interface: serde_json::Value,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
}

impl Contract {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::new("unsupported contract schema_version"));
        }
        if self.objective.trim().is_empty() {
            return Err(ValidationError::new("contract objective must not be empty"));
        }
        if self.criteria.is_empty() {
            return Err(ValidationError::new(
                "contract must contain at least one criterion",
            ));
        }
        unique_non_empty(
            self.criteria.iter().map(|criterion| criterion.id.as_str()),
            "criterion id",
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct ContractBundle {
    pub schema_version: u32,
    pub contract: Contract,
    pub sensor_suite: SensorSuite,
}

impl ContractBundle {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::new(
                "unsupported contract_bundle schema_version",
            ));
        }
        self.contract.validate()?;
        self.sensor_suite.validate_for(&self.contract)
    }

    pub fn validate_revision_from(&self, previous: &Self) -> Result<(), ValidationError> {
        for old in &previous.contract.criteria {
            if let Some(reused) = self
                .contract
                .criteria
                .iter()
                .find(|criterion| criterion.id == old.id)
            {
                if reused != old {
                    return Err(ValidationError::new(format!(
                        "criterion id {} was reused for changed wording; use a new id",
                        old.id
                    )));
                }
            }
        }
        for old in &previous.sensor_suite.sensors {
            if let Some(reused) = self
                .sensor_suite
                .sensors
                .iter()
                .find(|sensor| sensor.id == old.id)
            {
                if reused != old {
                    return Err(ValidationError::new(format!(
                        "sensor id {} was reused for a changed sensor; use a new id",
                        old.id
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SensorVisibility {
    Visible,
    Hidden,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SensorType {
    Test,
    Command,
    Query,
    Judge,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    Computational,
    Inferential,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct SensorSpec {
    pub id: String,
    pub criterion_id: String,
    #[serde(rename = "type")]
    pub sensor_type: SensorType,
    pub kind: SensorKind,
    pub visibility: SensorVisibility,
    pub purpose: String,
    pub check: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct SensorSuite {
    pub schema_version: u32,
    pub sensors: Vec<SensorSpec>,
}

impl SensorSuite {
    pub fn validate_for(&self, contract: &Contract) -> Result<(), ValidationError> {
        contract.validate()?;
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::new(
                "unsupported sensor_suite schema_version",
            ));
        }
        if self.sensors.is_empty() {
            return Err(ValidationError::new(
                "sensor_suite must contain at least one sensor",
            ));
        }
        unique_non_empty(
            self.sensors.iter().map(|sensor| sensor.id.as_str()),
            "sensor id",
        )?;

        for sensor in &self.sensors {
            if sensor.purpose != "acceptance" {
                return Err(ValidationError::new(format!(
                    "sensor {} purpose must be acceptance",
                    sensor.id
                )));
            }
            if sensor.check.trim().is_empty() {
                return Err(ValidationError::new(format!(
                    "sensor {} check must not be empty",
                    sensor.id
                )));
            }
            if !contract
                .criteria
                .iter()
                .any(|criterion| criterion.id == sensor.criterion_id)
            {
                return Err(ValidationError::new(format!(
                    "sensor {} references unknown criterion {}",
                    sensor.id, sensor.criterion_id
                )));
            }
        }

        for criterion in &contract.criteria {
            let linked = self
                .sensors
                .iter()
                .filter(|sensor| sensor.criterion_id == criterion.id)
                .collect::<Vec<_>>();
            if linked.is_empty() {
                return Err(ValidationError::new(format!(
                    "criterion {} has no acceptance sensor",
                    criterion.id
                )));
            }
            if !linked
                .iter()
                .any(|sensor| sensor.visibility == SensorVisibility::Visible)
            {
                return Err(ValidationError::new(format!(
                    "criterion {} has no visible sensor",
                    criterion.id
                )));
            }
        }
        if !self
            .sensors
            .iter()
            .any(|sensor| sensor.visibility == SensorVisibility::Hidden)
        {
            return Err(ValidationError::new(
                "sensor_suite must contain at least one hidden sensor",
            ));
        }
        Ok(())
    }
}

impl SensorSpec {
    pub fn checksum(&self) -> Result<String, ValidationError> {
        checksum_of(self)
    }
}

impl SensorSuite {
    pub fn checksum(&self) -> Result<String, ValidationError> {
        checksum_of(self)
    }
}

/// Executable implementation of the acceptance sensors.
///
/// The acceptance author returns source code; the engine materializes it outside
/// the task workspace and runs each script through `ScriptSensorRunner`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct SensorImplementation {
    pub schema_version: u32,
    pub scripts: Vec<SensorScript>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct SensorScript {
    pub sensor_id: String,
    pub filename: String,
    pub source: String,
}

impl SensorScript {
    pub fn checksum(&self) -> Result<String, ValidationError> {
        checksum_of(self)
    }
}

impl SensorImplementation {
    pub fn validate_for(&self, suite: &SensorSuite) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::new(
                "unsupported sensor_implementation schema_version",
            ));
        }
        if self.scripts.len() != suite.sensors.len() {
            return Err(ValidationError::new(format!(
                "sensor_implementation has {} scripts for {} sensors",
                self.scripts.len(),
                suite.sensors.len()
            )));
        }
        unique_non_empty(
            self.scripts.iter().map(|script| script.sensor_id.as_str()),
            "sensor script sensor_id",
        )?;
        unique_non_empty(
            self.scripts.iter().map(|script| script.filename.as_str()),
            "sensor script filename",
        )?;
        for script in &self.scripts {
            if !suite
                .sensors
                .iter()
                .any(|sensor| sensor.id == script.sensor_id)
            {
                return Err(ValidationError::new(format!(
                    "sensor script references unknown sensor {}",
                    script.sensor_id
                )));
            }
            if script.filename.contains('/')
                || script.filename.contains('\\')
                || script.filename.contains("..")
                || !script.filename.ends_with(".py")
            {
                return Err(ValidationError::new(format!(
                    "sensor script filename must be a local .py file: {}",
                    script.filename
                )));
            }
            if script.source.trim().is_empty() {
                return Err(ValidationError::new(format!(
                    "sensor script {} has empty source",
                    script.sensor_id
                )));
            }
        }
        for sensor in &suite.sensors {
            if !self
                .scripts
                .iter()
                .any(|script| script.sensor_id == sensor.id)
            {
                return Err(ValidationError::new(format!(
                    "sensor {} has no executable script",
                    sensor.id
                )));
            }
        }
        Ok(())
    }

    pub fn source_for(&self, sensor_id: &str) -> Option<&str> {
        self.scripts
            .iter()
            .find(|script| script.sensor_id == sensor_id)
            .map(|script| script.source.as_str())
    }

    pub fn checksum(&self) -> Result<String, ValidationError> {
        checksum_of(self)
    }
}

/// Frozen acceptance sensors. The engine verifies it before every sensor round.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct SensorLock {
    pub schema_version: u32,
    pub suite_checksum: String,
    pub sensors: Vec<SensorLockEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct SensorLockEntry {
    pub sensor_id: String,
    pub checksum: String,
}

impl SensorLock {
    pub fn from_suite(suite: &SensorSuite) -> Result<Self, ValidationError> {
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            suite_checksum: suite.checksum()?,
            sensors: suite
                .sensors
                .iter()
                .map(|sensor| {
                    Ok(SensorLockEntry {
                        sensor_id: sensor.id.clone(),
                        checksum: sensor.checksum()?,
                    })
                })
                .collect::<Result<Vec<_>, ValidationError>>()?,
        })
    }

    pub fn verify(&self, suite: &SensorSuite) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::new(
                "unsupported sensor_lock schema_version",
            ));
        }
        if self.suite_checksum != suite.checksum()? {
            return Err(ValidationError::new(
                "sensor suite does not match the locked checksum",
            ));
        }
        if self.sensors.len() != suite.sensors.len() {
            return Err(ValidationError::new(
                "sensor lock does not match the suite sensor count",
            ));
        }
        for sensor in &suite.sensors {
            let locked = self
                .sensors
                .iter()
                .find(|entry| entry.sensor_id == sensor.id)
                .ok_or_else(|| {
                    ValidationError::new(format!("sensor {} is not part of the lock", sensor.id))
                })?;
            if locked.checksum != sensor.checksum()? {
                return Err(ValidationError::new(format!(
                    "sensor {} does not match its locked checksum",
                    sensor.id
                )));
            }
        }
        Ok(())
    }
}

fn checksum_of<T: Serialize>(value: &T) -> Result<String, ValidationError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| ValidationError::new(error.to_string()))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub description: String,
    pub criteria: Vec<String>,
    pub sensors: Vec<String>,
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub files_hint: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Plan {
    pub schema_version: u32,
    pub contract_version: u32,
    pub steps: Vec<PlanStep>,
    pub integration_step_id: String,
    pub rationale: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct StepReport {
    pub schema_version: u32,
    pub step_id: String,
    pub attempts: u32,
    pub summary: String,
    pub files_changed: Vec<String>,
    pub decisions: Vec<String>,
    pub disputes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct SensorResult {
    pub sensor_id: String,
    pub criterion_id: String,
    pub passed: bool,
    pub evidence: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct VerdictIssue {
    pub criterion_id: String,
    pub description: String,
    pub severity: IssueSeverity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Blocker,
    NonBlocking,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct CriterionVerdict {
    pub criterion_id: String,
    pub passed: bool,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Verdict {
    pub schema_version: u32,
    pub approved: bool,
    pub criteria: Vec<CriterionVerdict>,
    pub issues: Vec<VerdictIssue>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct DeliveryReport {
    pub schema_version: u32,
    pub run_id: String,
    pub summary: String,
    pub criteria_passed: usize,
    pub criteria_total: usize,
    pub sensor_results: Vec<SensorResult>,
    pub states: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Gate {
    pub schema_version: u32,
    pub id: String,
    pub gate_type: String,
    pub question: String,
    pub context_summary: String,
    pub options: Vec<String>,
    pub free_text_allowed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct RunEvent {
    pub schema_version: u32,
    pub run_id: String,
    pub sequence: u64,
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ValidationError {}

fn unique_non_empty<'a>(
    values: impl Iterator<Item = &'a str>,
    field: &str,
) -> Result<(), ValidationError> {
    let mut seen = std::collections::HashSet::new();
    for value in values {
        if value.trim().is_empty() {
            return Err(ValidationError::new(format!("{field} must not be empty")));
        }
        if !seen.insert(value) {
            return Err(ValidationError::new(format!("duplicate {field}: {value}")));
        }
    }
    Ok(())
}

/// Generate the checked-in JSON Schemas from the Rust artifact definitions.
pub fn json_schemas() -> Result<Vec<(&'static str, String)>, serde_json::Error> {
    Ok(vec![
        (
            "understanding.schema.json",
            serialize_schema::<Understanding>()?,
        ),
        (
            "contract_bundle.schema.json",
            serialize_schema::<ContractBundle>()?,
        ),
        ("contract.schema.json", serialize_schema::<Contract>()?),
        (
            "sensor_suite.schema.json",
            serialize_schema::<SensorSuite>()?,
        ),
        ("sensor_lock.schema.json", serialize_schema::<SensorLock>()?),
        (
            "sensor_implementation.schema.json",
            serialize_schema::<SensorImplementation>()?,
        ),
        ("plan.schema.json", serialize_schema::<Plan>()?),
        ("step_report.schema.json", serialize_schema::<StepReport>()?),
        ("verdict.schema.json", serialize_schema::<Verdict>()?),
        (
            "delivery_report.schema.json",
            serialize_schema::<DeliveryReport>()?,
        ),
        ("gate.schema.json", serialize_schema::<Gate>()?),
        ("run_event.schema.json", serialize_schema::<RunEvent>()?),
    ])
}

fn serialize_schema<T: JsonSchema>() -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(schema_for!(T))?;
    normalize_strict_schema(&mut value);
    serde_json::to_string_pretty(&value)
}

fn normalize_strict_schema(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.remove("$schema");
            if object.get("type") == Some(&serde_json::Value::String("object".to_owned())) {
                object.insert(
                    "additionalProperties".to_owned(),
                    serde_json::Value::Bool(false),
                );
                let required = object
                    .get("properties")
                    .and_then(|value| value.as_object())
                    .map(|properties| {
                        serde_json::Value::Array(
                            properties
                                .keys()
                                .cloned()
                                .map(serde_json::Value::String)
                                .collect(),
                        )
                    });
                if let Some(required) = required {
                    object.insert("required".to_owned(), required);
                }
            }
            for child in object.values_mut() {
                normalize_strict_schema(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                normalize_strict_schema(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> Contract {
        Contract {
            schema_version: SCHEMA_VERSION,
            objective: "Converter CSV em JSON".into(),
            assumptions: vec![],
            criteria: vec![Criterion {
                id: "records".into(),
                description: "Cada linha vira um objeto".into(),
            }],
            interface: serde_json::Value::Null,
            constraints: vec![],
            out_of_scope: vec![],
        }
    }

    fn sensor(visibility: SensorVisibility) -> SensorSpec {
        SensorSpec {
            id: "records-sensor".into(),
            criterion_id: "records".into(),
            sensor_type: SensorType::Test,
            kind: SensorKind::Computational,
            visibility,
            purpose: "acceptance".into(),
            check: "Executar CLI com arquivo CSV e comparar JSON".into(),
        }
    }

    #[test]
    fn sensor_suite_requires_a_visible_sensor_for_each_criterion() {
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![sensor(SensorVisibility::Hidden)],
        };
        assert!(suite.validate_for(&contract()).is_err());
    }

    #[test]
    fn sensor_suite_accepts_covered_visible_criterion() {
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![
                sensor(SensorVisibility::Visible),
                SensorSpec {
                    id: "hidden-records-sensor".into(),
                    visibility: SensorVisibility::Hidden,
                    ..sensor(SensorVisibility::Visible)
                },
            ],
        };
        assert!(suite.validate_for(&contract()).is_ok());
    }

    #[test]
    fn sensor_suite_requires_at_least_one_hidden_sensor() {
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![sensor(SensorVisibility::Visible)],
        };
        assert!(suite.validate_for(&contract()).is_err());
    }

    #[test]
    fn sensor_lock_detects_changed_sensor_content() {
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![sensor(SensorVisibility::Visible)],
        };
        let lock = SensorLock::from_suite(&suite).expect("checksum da suíte");
        assert!(lock.verify(&suite).is_ok());

        let mut changed = suite.clone();
        changed.sensors[0].check = "Outra verificação".into();
        assert!(lock.verify(&changed).is_err());
    }

    #[test]
    fn sensor_lock_detects_added_sensor() {
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![sensor(SensorVisibility::Visible)],
        };
        let lock = SensorLock::from_suite(&suite).expect("checksum da suíte");

        let mut extended = suite;
        extended.sensors.push(SensorSpec {
            id: "extra".into(),
            ..sensor(SensorVisibility::Hidden)
        });
        assert!(lock.verify(&extended).is_err());
    }

    #[test]
    fn sensor_implementation_requires_one_safe_script_per_sensor() {
        let suite = SensorSuite {
            schema_version: SCHEMA_VERSION,
            sensors: vec![sensor(SensorVisibility::Visible)],
        };
        let implementation = SensorImplementation {
            schema_version: SCHEMA_VERSION,
            scripts: vec![SensorScript {
                sensor_id: "records-sensor".into(),
                filename: "records_sensor.py".into(),
                source: "print('ok')".into(),
            }],
        };
        assert!(implementation.validate_for(&suite).is_ok());
        assert_eq!(
            implementation.source_for("records-sensor"),
            Some("print('ok')")
        );

        let missing = SensorImplementation {
            scripts: vec![],
            ..implementation.clone()
        };
        assert!(missing.validate_for(&suite).is_err());

        let unknown = SensorImplementation {
            scripts: vec![SensorScript {
                sensor_id: "other".into(),
                ..implementation.scripts[0].clone()
            }],
            ..implementation.clone()
        };
        assert!(unknown.validate_for(&suite).is_err());

        let traversal = SensorImplementation {
            scripts: vec![SensorScript {
                filename: "../escape.py".into(),
                ..implementation.scripts[0].clone()
            }],
            ..implementation
        };
        assert!(traversal.validate_for(&suite).is_err());
    }

    #[test]
    fn contract_bundle_validates_contract_and_sensor_coverage_together() {
        let bundle = ContractBundle {
            schema_version: SCHEMA_VERSION,
            contract: contract(),
            sensor_suite: SensorSuite {
                schema_version: SCHEMA_VERSION,
                sensors: vec![
                    sensor(SensorVisibility::Visible),
                    SensorSpec {
                        id: "hidden-records-sensor".into(),
                        visibility: SensorVisibility::Hidden,
                        ..sensor(SensorVisibility::Visible)
                    },
                ],
            },
        };
        assert!(bundle.validate().is_ok());

        let reuses_criterion_id = ContractBundle {
            contract: Contract {
                criteria: vec![Criterion {
                    id: "records".into(),
                    description: "A linha vira outro resultado".into(),
                }],
                ..bundle.contract.clone()
            },
            ..bundle.clone()
        };
        assert!(reuses_criterion_id.validate().is_ok());
        assert!(reuses_criterion_id.validate_revision_from(&bundle).is_err());

        let invalid = ContractBundle {
            sensor_suite: SensorSuite {
                schema_version: SCHEMA_VERSION,
                sensors: vec![sensor(SensorVisibility::Visible)],
            },
            ..bundle
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn schemas_are_generated_for_all_mvp_artifacts() {
        let schemas = json_schemas().expect("schemas serializáveis");
        assert_eq!(schemas.len(), 12);
        assert!(
            schemas
                .iter()
                .any(|(name, _)| *name == "understanding.schema.json")
        );
        assert!(
            schemas
                .iter()
                .any(|(name, _)| *name == "contract_bundle.schema.json")
        );
        assert!(
            schemas
                .iter()
                .any(|(name, _)| *name == "contract.schema.json")
        );
        for (_, schema) in schemas {
            let value: serde_json::Value =
                serde_json::from_str(&schema).expect("schema JSON válido");
            assert!(value.get("$schema").is_none());
            assert_closed_objects(&value);
        }
    }

    #[test]
    fn understanding_requires_answers_before_contracting() {
        let understanding = Understanding {
            schema_version: SCHEMA_VERSION,
            summary: "Organizar arquivos da pasta de downloads".into(),
            goal: "Arquivos classificados em pastas".into(),
            assumptions: vec![],
            blocking_questions: vec![BlockingQuestion {
                id: "operation".into(),
                question: "Mover ou copiar?".into(),
                why_blocking: "Muda os efeitos da tarefa".into(),
                options: vec!["mover".into(), "copiar".into()],
            }],
            open_questions: vec![],
            answers: vec![],
            context_refs: vec![],
            locale: "pt-BR".into(),
        };
        assert!(understanding.validate().is_ok());
        assert_eq!(understanding.unanswered_blocking_questions().len(), 1);

        let answered = Understanding {
            answers: vec![Answer {
                question_id: "operation".into(),
                answer: "mover".into(),
                answered_by: "user".into(),
            }],
            ..understanding
        };
        assert!(answered.validate().is_ok());
        assert!(answered.unanswered_blocking_questions().is_empty());
    }

    fn assert_closed_objects(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if object.get("type") == Some(&serde_json::Value::String("object".into())) {
                    assert_eq!(
                        object.get("additionalProperties"),
                        Some(&serde_json::Value::Bool(false))
                    );
                    if let Some(properties) = object.get("properties").and_then(|x| x.as_object()) {
                        let required = object
                            .get("required")
                            .and_then(|x| x.as_array())
                            .expect("strict objects declare required fields")
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .collect::<std::collections::BTreeSet<_>>();
                        let property_names = properties
                            .keys()
                            .map(String::as_str)
                            .collect::<std::collections::BTreeSet<_>>();
                        assert_eq!(required, property_names);
                    }
                }
                for child in object.values() {
                    assert_closed_objects(child);
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    assert_closed_objects(child);
                }
            }
            _ => {}
        }
    }
}
