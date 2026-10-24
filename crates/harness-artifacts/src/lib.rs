//! MVP artifacts shared by the CLI and deterministic engine.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Criterion {
    pub id: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Contract {
    pub schema_version: u32,
    pub objective: String,
    pub criteria: Vec<Criterion>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorVisibility {
    Visible,
    Hidden,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SensorSpec {
    pub id: String,
    pub criterion_id: String,
    pub visibility: SensorVisibility,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SensorResult {
    pub sensor_id: String,
    pub passed: bool,
    pub evidence: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliveryReport {
    pub schema_version: u32,
    pub run_id: String,
    pub summary: String,
    pub criteria_passed: usize,
    pub criteria_total: usize,
    pub sensor_results: Vec<SensorResult>,
    pub states: Vec<String>,
}
