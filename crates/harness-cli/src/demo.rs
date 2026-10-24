use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::codex::{CodexCliExecutor, CodexCliVerifier};
use harness_artifacts::{
    Contract, CriterionVerdict, DeliveryReport, IssueSeverity, SCHEMA_VERSION,
    SensorImplementation, SensorLock, SensorResult, SensorScript, SensorSuite, StepReport, Verdict,
    VerdictIssue,
};
use harness_engine::{
    ExecutionContext, Executor, RunCoordinator, RunParticipants, RunRequest, SensorRunner,
    Verifier, check_sensors_fail,
};
use harness_sensors::ScriptSensorRunner;

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

const VISIBLE_RECORD_CONVERSION: &str = r#"import json
import os
import subprocess
import sys

workspace = os.environ["HARNESS_WORKSPACE"]
source = os.path.join(workspace, "visible.csv")
with open(source, "w", encoding="utf-8") as handle:
    handle.write("name,city\nAda,Lisbon\n")

result = subprocess.run(
    [sys.executable, os.path.join(workspace, "convert.py"), source],
    capture_output=True,
    text=True,
)
if result.returncode != 0:
    print(f"conversor falhou: {result.stderr.strip()}", file=sys.stderr)
    sys.exit(1)
try:
    actual = json.loads(result.stdout)
except json.JSONDecodeError:
    print(f"saída não é JSON: {result.stdout!r}", file=sys.stderr)
    sys.exit(1)
expected = [{"name": "Ada", "city": "Lisbon"}]
if actual != expected:
    print(f"saída inesperada: {actual!r}", file=sys.stderr)
    sys.exit(1)
print("CSV simples convertido para um array JSON de objetos.")
"#;

const VISIBLE_QUOTED_FIELDS: &str = r#"import csv
import json
import os
import subprocess
import sys

workspace = os.environ["HARNESS_WORKSPACE"]
source = os.path.join(workspace, "visible-quoted.csv")
with open(source, "w", newline="", encoding="utf-8") as handle:
    writer = csv.writer(handle)
    writer.writerow(["name", "city", "quote"])
    writer.writerow(["Ada", "London, UK", 'She said "hello"'])

result = subprocess.run(
    [sys.executable, os.path.join(workspace, "convert.py"), source],
    capture_output=True,
    text=True,
)
if result.returncode != 0:
    print(f"conversor falhou: {result.stderr.strip()}", file=sys.stderr)
    sys.exit(1)
try:
    actual = json.loads(result.stdout)
except json.JSONDecodeError:
    print(f"saída não é JSON: {result.stdout!r}", file=sys.stderr)
    sys.exit(1)
expected = [{"name": "Ada", "city": "London, UK", "quote": 'She said "hello"'}]
if actual != expected:
    print(f"saída inesperada: {actual!r}", file=sys.stderr)
    sys.exit(1)
print("Vírgula e aspas escapadas preservadas em campos CSV.")
"#;

const HIDDEN_QUOTED_FIELDS: &str = r#"import csv
import json
import os
import subprocess
import sys

workspace = os.environ["HARNESS_WORKSPACE"]
source = os.path.join(workspace, "hidden.csv")
with open(source, "w", newline="", encoding="utf-8") as handle:
    writer = csv.writer(handle)
    writer.writerow(["name", "quote", "note"])
    writer.writerow(["Ada", 'She said "hello"', "line one\nline two"])

result = subprocess.run(
    [sys.executable, os.path.join(workspace, "convert.py"), source],
    capture_output=True,
    text=True,
)
if result.returncode != 0:
    print(f"conversor falhou: {result.stderr.strip()}", file=sys.stderr)
    sys.exit(1)
try:
    actual = json.loads(result.stdout)
except json.JSONDecodeError:
    print(f"saída não é JSON: {result.stdout!r}", file=sys.stderr)
    sys.exit(1)
expected = [{"name": "Ada", "quote": 'She said "hello"', "note": "line one\nline two"}]
if actual != expected:
    print(f"saída inesperada: {actual!r}", file=sys.stderr)
    sys.exit(1)
print("Campos com aspas, vírgula ou quebra de linha preservados.")
"#;

pub fn run() -> DemoResult<()> {
    let (workspace, run_id) = create_workspace("demo")?;
    let mut executor = DemoExecutor;
    let mut verifier = DemoVerifier;
    let (sensor_dir, mut sensor_runner) = script_runner()?;
    let result = run_in_workspace(
        &workspace,
        &run_id,
        &mut executor,
        &mut verifier,
        &mut sensor_runner,
    );
    if let Err(error) = &result {
        eprintln!("Workspace do demo preservado em {}", workspace.display());
        eprintln!("Sensores preservados em {}", sensor_dir.display());
        return Err(error.to_string().into());
    }
    let _ = fs::remove_dir_all(&sensor_dir);
    result
}

pub fn run_pilot() -> DemoResult<()> {
    let (workspace, run_id) = create_workspace("pilot")?;
    let mut executor = CodexCliExecutor::default();
    let mut verifier = CodexCliVerifier::default();
    let (sensor_dir, mut sensor_runner) = script_runner()?;
    let result = run_in_workspace(
        &workspace,
        &run_id,
        &mut executor,
        &mut verifier,
        &mut sensor_runner,
    );
    if let Err(error) = &result {
        eprintln!("Workspace do piloto preservado em {}", workspace.display());
        eprintln!("Sensores preservados em {}", sensor_dir.display());
        return Err(error.to_string().into());
    }
    let _ = fs::remove_dir_all(&sensor_dir);
    result
}

fn script_runner() -> DemoResult<(PathBuf, ScriptSensorRunner)> {
    let sensors = demo_sensors()?;
    let implementation = demo_sensor_implementation()?;
    let (directory, _) = create_workspace("sensors")?;
    let runner = ScriptSensorRunner::materialize(&implementation, &sensors, &directory)?;
    Ok((directory, runner))
}

fn run_in_workspace(
    workspace: &Path,
    run_id: &str,
    executor: &mut dyn Executor,
    verifier: &mut dyn Verifier,
    sensor_runner: &mut dyn SensorRunner,
) -> DemoResult<()> {
    let contract = demo_contract()?;
    let sensors = demo_sensors()?;
    let lock = SensorLock::from_suite(&sensors)?;
    lock.verify(&sensors)?;
    check_sensors_fail_on_baseline(&sensors, sensor_runner)?;
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
                executor,
                sensor_runner,
                verifier,
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
        run_id: run_id.to_owned(),
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
    if !execution.verdict.approved {
        return Err("verificação reprovou a execução".into());
    }
    Ok(())
}

fn check_sensors_fail_on_baseline(
    sensors: &SensorSuite,
    sensor_runner: &mut dyn SensorRunner,
) -> DemoResult<()> {
    let (baseline, _) = create_workspace("baseline")?;
    let result = check_sensors_fail(sensors, sensor_runner, &baseline);
    fs::remove_dir_all(&baseline)?;
    let results = result?;
    println!(
        "Sensores travados ({}); {} falharam antes da implementação.",
        sensors.checksum()?,
        results.len()
    );
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

fn demo_sensor_implementation() -> DemoResult<SensorImplementation> {
    Ok(SensorImplementation {
        schema_version: SCHEMA_VERSION,
        scripts: vec![
            SensorScript {
                sensor_id: "visible-record-conversion".into(),
                filename: "visible_record_conversion.py".into(),
                source: VISIBLE_RECORD_CONVERSION.into(),
            },
            SensorScript {
                sensor_id: "visible-quoted-fields".into(),
                filename: "visible_quoted_fields.py".into(),
                source: VISIBLE_QUOTED_FIELDS.into(),
            },
            SensorScript {
                sensor_id: "hidden-quoted-fields".into(),
                filename: "hidden_quoted_fields.py".into(),
                source: HIDDEN_QUOTED_FIELDS.into(),
            },
        ],
    })
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
        let converter_path = workspace.join("convert.py");
        fs::write(&converter_path, CONVERTER).map_err(|error| error.to_string())?;
        Ok(StepReport {
            schema_version: SCHEMA_VERSION,
            step_id: "cli-01-converter".to_owned(),
            attempts: 1,
            summary: "Worker de demonstração escreveu convert.py no diretório temporário."
                .to_owned(),
            files_changed: vec!["convert.py".to_owned()],
            decisions: vec![],
            disputes: vec![],
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
        _workspace: &Path,
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

fn create_workspace(prefix: &str) -> DemoResult<(PathBuf, String)> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_id = format!("{prefix}-{}-{nanos}", std::process::id());
    let path = std::env::temp_dir().join(format!("harness-{run_id}"));
    fs::create_dir_all(&path)?;
    Ok((path, run_id))
}

#[cfg(test)]
mod tests {
    use super::{demo_contract, demo_sensor_implementation, demo_sensors};
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

    #[test]
    fn demo_sensor_implementation_covers_every_sensor() {
        let sensors = demo_sensors().expect("sensor_suite demo parseável");
        let implementation = demo_sensor_implementation().expect("implementação demo");
        implementation
            .validate_for(&sensors)
            .expect("todo sensor tem script");
        assert_eq!(implementation.scripts.len(), 3);
    }
}
