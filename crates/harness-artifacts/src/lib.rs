//! Versioned MVP artifacts shared by the CLI and deterministic engine.

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

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
            "contract.schema.json",
            serde_json::to_string_pretty(&schema_for!(Contract))?,
        ),
        (
            "sensor_suite.schema.json",
            serde_json::to_string_pretty(&schema_for!(SensorSuite))?,
        ),
        (
            "plan.schema.json",
            serde_json::to_string_pretty(&schema_for!(Plan))?,
        ),
        (
            "step_report.schema.json",
            serde_json::to_string_pretty(&schema_for!(StepReport))?,
        ),
        (
            "verdict.schema.json",
            serde_json::to_string_pretty(&schema_for!(Verdict))?,
        ),
        (
            "delivery_report.schema.json",
            serde_json::to_string_pretty(&schema_for!(DeliveryReport))?,
        ),
        (
            "gate.schema.json",
            serde_json::to_string_pretty(&schema_for!(Gate))?,
        ),
        (
            "run_event.schema.json",
            serde_json::to_string_pretty(&schema_for!(RunEvent))?,
        ),
    ])
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
    fn schemas_are_generated_for_all_mvp_artifacts() {
        let schemas = json_schemas().expect("schemas serializáveis");
        assert_eq!(schemas.len(), 8);
        assert!(
            schemas
                .iter()
                .any(|(name, _)| *name == "contract.schema.json")
        );
    }
}
