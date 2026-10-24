use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_artifacts::{
    Contract, DeliveryReport, SCHEMA_VERSION, SensorResult, SensorSpec, SensorSuite,
    SensorVisibility,
};
use harness_engine::{RunLifecycle, RunState};

type DemoResult<T> = Result<T, Box<dyn Error>>;

const CONVERTER: &str = r#"import argparse
import csv
import json

parser = argparse.ArgumentParser()
parser.add_argument("input")
args = parser.parse_args()

with open(args.input, newline="", encoding="utf-8-sig") as source:
    rows = list(csv.DictReader(source))
json.dump(rows, fp=__import__("sys").stdout, ensure_ascii=False)
print()
"#;

pub fn run() -> DemoResult<()> {
    let workspace = create_workspace()?;
    let result = run_in_workspace(&workspace);
    if let Err(error) = &result {
        eprintln!("Workspace do demo preservado em {}", workspace.display());
        return Err(error.to_string().into());
    }
    result
}

fn run_in_workspace(workspace: &Path) -> DemoResult<()> {
    let contract = demo_contract()?;
    contract.validate()?;
    let sensors = demo_sensors()?;
    sensors.validate_for(&contract)?;
    let mut lifecycle = RunLifecycle::new();

    lifecycle.transition(RunState::Understanding)?;
    lifecycle.transition(RunState::Contracting)?;
    lifecycle.transition(RunState::AcceptanceAuthoring)?;
    lifecycle.transition(RunState::Planning)?;
    lifecycle.transition(RunState::Executing)?;

    let worker_dir = workspace.join("worker");
    fs::create_dir_all(&worker_dir)?;
    let converter_path = worker_dir.join("convert.py");
    fs::write(&converter_path, CONVERTER)?;
    println!("Worker de demonstração escreveu convert.py no diretório temporário.");

    let visible = sensors
        .sensors
        .iter()
        .filter(|sensor| sensor.visibility == SensorVisibility::Visible)
        .collect::<Vec<_>>();
    let visible_results = run_visible_sensors(&visible, &converter_path, workspace)?;
    if visible_results.iter().any(|result| !result.passed) {
        lifecycle.transition(RunState::Failed)?;
        return Err("sensor visível reprovou".into());
    }

    lifecycle.transition(RunState::Verifying)?;
    let hidden = sensors
        .sensors
        .iter()
        .filter(|sensor| sensor.visibility == SensorVisibility::Hidden)
        .collect::<Vec<_>>();
    let mut sensor_results = visible_results;
    sensor_results.extend(run_hidden_sensors(&hidden, &converter_path, workspace)?);
    let criteria_passed = contract
        .criteria
        .iter()
        .filter(|criterion| {
            let linked = sensors
                .sensors
                .iter()
                .filter(|sensor| sensor.criterion_id == criterion.id)
                .collect::<Vec<_>>();
            !linked.is_empty()
                && linked.iter().all(|sensor| {
                    sensor_results
                        .iter()
                        .any(|result| result.sensor_id == sensor.id && result.passed)
                })
        })
        .count();

    if criteria_passed == contract.criteria.len() {
        lifecycle.transition(RunState::AwaitingMerge)?;
        lifecycle.transition(RunState::Done)?;
    } else {
        lifecycle.transition(RunState::Executing)?;
        lifecycle.transition(RunState::Failed)?;
    }

    let report = DeliveryReport {
        schema_version: SCHEMA_VERSION,
        run_id: format!("demo-{}", std::process::id()),
        summary: if criteria_passed == contract.criteria.len() {
            "Conversor CSV para JSON passou nos sensores visível e oculto.".to_owned()
        } else {
            "A execução de demonstração não passou em todos os critérios.".to_owned()
        },
        criteria_passed,
        criteria_total: contract.criteria.len(),
        sensor_results,
        states: lifecycle
            .history()
            .iter()
            .map(|state| state.as_str().to_owned())
            .collect(),
    };
    let report_path = workspace.join("delivery_report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!("Relatório salvo em {}", report_path.display());
    Ok(())
}

fn demo_contract() -> DemoResult<Contract> {
    Ok(serde_json::from_str(include_str!(
        "../../../examples/cli-01/contract.json"
    ))?)
}

fn demo_sensors() -> DemoResult<SensorSuite> {
    Ok(serde_json::from_str(include_str!(
        "../../../examples/cli-01/sensor_suite.json"
    ))?)
}

fn run_visible_sensors(
    sensors: &[&SensorSpec],
    converter: &Path,
    workspace: &Path,
) -> DemoResult<Vec<SensorResult>> {
    sensors
        .iter()
        .map(|sensor| {
            let (filename, csv, expected, evidence) = match sensor.id.as_str() {
                "visible-record-conversion" => (
                    "visible.csv",
                    "name,city\nAda,Lisbon\n",
                    serde_json::json!([{"name":"Ada", "city":"Lisbon"}]),
                    "CSV simples convertido para um array JSON de objetos.",
                ),
                "visible-quoted-fields" => (
                    "visible-quoted.csv",
                    "name,city,quote\nAda,\"London, UK\",\"She said \"\"hello\"\"\"\n",
                    serde_json::json!([{"name":"Ada", "city":"London, UK", "quote":"She said \"hello\""}]),
                    "Vírgula e aspas escapadas preservadas em campos CSV.",
                ),
                other => return Err(format!("sensor visível desconhecido: {other}").into()),
            };
            let input = workspace.join(filename);
            fs::write(&input, csv)?;
            let output = run_converter(converter, &input)?;
            let parsed: serde_json::Value = serde_json::from_slice(&output)?;
            let passed = parsed == expected;
            Ok(SensorResult {
                sensor_id: sensor.id.clone(),
                passed,
                evidence: if passed {
                    evidence.to_owned()
                } else {
                    "A saída não corresponde ao resultado esperado.".to_owned()
                },
            })
        })
        .collect()
}

fn run_hidden_sensors(
    sensors: &[&SensorSpec],
    converter: &Path,
    workspace: &Path,
) -> DemoResult<Vec<SensorResult>> {
    let input = workspace.join("hidden.csv");
    fs::write(
        &input,
        "name,quote,note\nAda,\"She said \"\"hello\"\"\",\"line one\nline two\"\n",
    )?;
    let output = run_converter(converter, &input)?;
    let parsed: serde_json::Value = serde_json::from_slice(&output)?;
    let passed = parsed
        == serde_json::json!([{
            "name":"Ada",
            "quote":"She said \"hello\"",
            "note":"line one\nline two"
        }]);
    Ok(sensors
        .iter()
        .map(|sensor| SensorResult {
            sensor_id: sensor.id.clone(),
            passed,
            evidence: if passed {
                "Campos com aspas, vírgula ou quebra de linha preservados.".to_owned()
            } else {
                "O caso CSV com campos citados não foi preservado.".to_owned()
            },
        })
        .collect())
}

fn run_converter(converter: &Path, input: &Path) -> DemoResult<Vec<u8>> {
    let output = Command::new("python3").arg(converter).arg(input).output()?;
    if !output.status.success() {
        return Err(format!(
            "conversor encerrou com {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(output.stdout)
}

fn create_workspace() -> DemoResult<PathBuf> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!("harness-demo-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::{demo_contract, demo_sensors};
    use harness_artifacts::{SCHEMA_VERSION, SensorVisibility};

    #[test]
    fn demo_artifacts_have_visible_and_hidden_coverage() {
        let contract = demo_contract().expect("contrato demo parseável");
        let sensors = demo_sensors().expect("sensor_suite demo parseável");
        assert_eq!(contract.schema_version, SCHEMA_VERSION);
        sensors.validate_for(&contract).expect("suite válida");
        assert_eq!(contract.criteria.len(), 2);
        assert_eq!(
            sensors
                .sensors
                .iter()
                .filter(|sensor| sensor.visibility == SensorVisibility::Visible)
                .count(),
            2
        );
        assert_eq!(
            sensors
                .sensors
                .iter()
                .filter(|sensor| sensor.visibility == SensorVisibility::Hidden)
                .count(),
            1
        );
    }
}
