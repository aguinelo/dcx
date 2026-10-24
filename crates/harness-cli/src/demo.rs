use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_artifacts::{
    Contract, CriterionVerdict, DeliveryReport, IssueSeverity, SCHEMA_VERSION, SensorResult,
    SensorSpec, SensorSuite, StepReport, Verdict, VerdictIssue,
};
use harness_engine::{ExecutionContext, Executor, RunCoordinator, SensorRunner, Verifier};
use harness_engine::{RunParticipants, RunRequest};

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
    let sensors = demo_sensors()?;
    let mut executor = DemoExecutor;
    let mut sensor_runner = DemoSensorRunner {
        converter_path: workspace.join("worker/convert.py"),
    };
    let mut verifier = DemoVerifier;
    let execution = RunCoordinator::new(3)
        .map_err(std::io::Error::other)?
        .execute(
            RunRequest {
                contract: &contract,
                suite: &sensors,
                step_id: "cli-01-converter",
                workspace,
            },
            RunParticipants {
                executor: &mut executor,
                sensor_runner: &mut sensor_runner,
                verifier: &mut verifier,
            },
        )
        .map_err(std::io::Error::other)?;
    for step_report in &execution.step_reports {
        println!("{}", step_report.summary);
    }
    let criteria_passed = execution
        .verdict
        .criteria
        .iter()
        .filter(|result| result.passed)
        .count();

    let report = DeliveryReport {
        schema_version: SCHEMA_VERSION,
        run_id: format!("demo-{}", std::process::id()),
        summary: if execution.verdict.approved {
            "Conversor CSV para JSON passou nos sensores visível e oculto.".to_owned()
        } else {
            "A execução de demonstração não passou em todos os critérios.".to_owned()
        },
        criteria_passed,
        criteria_total: contract.criteria.len(),
        sensor_results: execution.sensor_results,
        states: execution
            .states
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

struct DemoExecutor;

impl Executor for DemoExecutor {
    fn execute(
        &mut self,
        context: &ExecutionContext,
        workspace: &Path,
    ) -> Result<StepReport, String> {
        if context.visible_sensors.is_empty() {
            return Err("executor recebeu contexto sem sensores visíveis".to_owned());
        }
        let worker_dir = workspace.join("worker");
        fs::create_dir_all(&worker_dir).map_err(|error| error.to_string())?;
        let converter_path = worker_dir.join("convert.py");
        fs::write(&converter_path, CONVERTER).map_err(|error| error.to_string())?;
        Ok(StepReport {
            schema_version: SCHEMA_VERSION,
            step_id: "cli-01-converter".to_owned(),
            attempts: 1,
            summary: "Worker de demonstração escreveu convert.py no diretório temporário."
                .to_owned(),
            files_changed: vec!["worker/convert.py".to_owned()],
            decisions: vec![],
            disputes: vec![],
        })
    }
}

struct DemoSensorRunner {
    converter_path: PathBuf,
}

impl SensorRunner for DemoSensorRunner {
    fn run_sensor(
        &mut self,
        sensor: &SensorSpec,
        workspace: &Path,
    ) -> Result<SensorResult, String> {
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
            "hidden-quoted-fields" => (
                "hidden.csv",
                "name,quote,note\nAda,\"She said \"\"hello\"\"\",\"line one\nline two\"\n",
                serde_json::json!([{"name":"Ada", "quote":"She said \"hello\"", "note":"line one\nline two"}]),
                "Campos com aspas, vírgula ou quebra de linha preservados.",
            ),
            other => return Err(format!("sensor desconhecido: {other}")),
        };
        let input = workspace.join(filename);
        fs::write(&input, csv).map_err(|error| error.to_string())?;
        let output =
            run_converter(&self.converter_path, &input).map_err(|error| error.to_string())?;
        let parsed: serde_json::Value =
            serde_json::from_slice(&output).map_err(|error| error.to_string())?;
        let passed = parsed == expected;
        Ok(SensorResult {
            sensor_id: sensor.id.clone(),
            criterion_id: sensor.criterion_id.clone(),
            passed,
            evidence: if passed {
                evidence.to_owned()
            } else {
                "A saída não corresponde ao resultado esperado.".to_owned()
            },
        })
    }
}

struct DemoVerifier;

impl Verifier for DemoVerifier {
    fn verify(
        &mut self,
        contract: &Contract,
        suite: &SensorSuite,
        _step_report: &StepReport,
        sensor_results: &[SensorResult],
    ) -> Result<Verdict, String> {
        let criteria = contract
            .criteria
            .iter()
            .map(|criterion| {
                let expected = suite
                    .sensors
                    .iter()
                    .filter(|sensor| sensor.criterion_id == criterion.id)
                    .collect::<Vec<_>>();
                let evidence = sensor_results
                    .iter()
                    .filter(|result| result.criterion_id == criterion.id)
                    .map(|result| format!("{}: {}", result.sensor_id, result.evidence))
                    .collect::<Vec<_>>();
                let passed = !expected.is_empty()
                    && expected.iter().all(|sensor| {
                        sensor_results.iter().any(|result| {
                            result.sensor_id == sensor.id
                                && result.criterion_id == criterion.id
                                && result.passed
                        })
                    });
                CriterionVerdict {
                    criterion_id: criterion.id.clone(),
                    passed,
                    evidence,
                }
            })
            .collect::<Vec<_>>();
        let approved = criteria.iter().all(|criterion| criterion.passed);
        let issues = if approved {
            vec![]
        } else {
            criteria
                .iter()
                .filter(|criterion| !criterion.passed)
                .map(|criterion| VerdictIssue {
                    criterion_id: criterion.criterion_id.clone(),
                    description:
                        "Um ou mais sensores de aceite falharam ou não produziram resultado."
                            .to_owned(),
                    severity: IssueSeverity::Blocker,
                })
                .collect()
        };
        Ok(Verdict {
            schema_version: SCHEMA_VERSION,
            approved,
            criteria,
            issues,
            notes: vec!["Verificação determinística do protótipo; ainda não usa um agente LLM independente.".to_owned()],
        })
    }
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
