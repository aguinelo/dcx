#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const UNDERSTANDING: &str = r#"{
  "schema_version": 1,
  "summary": "Organizar os arquivos da pasta de downloads.",
  "goal": "Arquivos classificados em pastas por tipo.",
  "assumptions": [],
  "blocking_questions": [
    {
      "id": "organize-by",
      "question": "Organizar por tipo ou por data?",
      "why_blocking": "Muda o resultado observável da organização.",
      "options": ["tipo", "data"]
    }
  ],
  "open_questions": [],
  "answers": [],
  "context_refs": [],
  "locale": "pt-BR"
}"#;

const CONTRACT: &str = r#"{
  "schema_version": 1,
  "contract": {
    "schema_version": 1,
    "objective": "Organizar a pasta de downloads por tipo.",
    "assumptions": [],
    "criteria": [
      {
        "id": "organize-by-type",
        "description": "Cada arquivo vai para uma pasta com o nome do seu tipo."
      }
    ],
    "interface": null,
    "constraints": [],
    "out_of_scope": []
  },
  "sensor_suite": {
    "schema_version": 1,
    "sensors": [
      {
        "id": "visible-organize-by-type",
        "criterion_id": "organize-by-type",
        "type": "test",
        "kind": "computational",
        "visibility": "visible",
        "purpose": "acceptance",
        "check": "Executar a organização e conferir os caminhos resultantes."
      },
      {
        "id": "hidden-organize-by-type",
        "criterion_id": "organize-by-type",
        "type": "test",
        "kind": "computational",
        "visibility": "hidden",
        "purpose": "acceptance",
        "check": "Repetir com nomes Unicode, arquivos parciais e conflitos de nome."
      }
    ]
  }
}"#;

fn unique_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio antes da época")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "harness-cli-test-{}-{nanos}-{name}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("criar diretório de teste");
    path
}

fn write_fake_codex(dir: &Path, contracts: &[&str]) -> PathBuf {
    let path = dir.join("fake-codex");
    let counter = dir.join("contract-calls");
    let contracts = contracts
        .iter()
        .map(|contract| format!("{contract:?}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let script = r#"#!/usr/bin/env python3
import json
import sys

UNDERSTANDING = __UNDERSTANDING__
CONTRACTS = [
__CONTRACTS__
]
COUNTER = __COUNTER__

schema = ""
args = sys.argv[1:]
for index, arg in enumerate(args):
    if arg == "--output-schema" and index + 1 < len(args):
        schema = args[index + 1]
prompt = sys.stdin.read()

if schema.endswith("understanding.schema.json"):
    print(UNDERSTANDING)
elif schema.endswith("contract_bundle.schema.json"):
    count = 0
    try:
        with open(COUNTER, encoding="utf-8") as handle:
            count = int(handle.read())
    except FileNotFoundError:
        pass
    count += 1
    with open(COUNTER, "w", encoding="utf-8") as handle:
        handle.write(str(count))
    print(CONTRACTS[min(count, len(CONTRACTS)) - 1])
elif schema.endswith("sensor_implementation.schema.json"):
    marker = "Contrato aprovado e suíte de sensores:\n"
    start = prompt.index(marker) + len(marker)
    end = prompt.index("\n\nImplementação anterior", start)
    bundle = json.loads(prompt[start:end])
    scripts = []
    for sensor in bundle["sensor_suite"]["sensors"]:
        scripts.append({
            "sensor_id": sensor["id"],
            "filename": sensor["id"].replace("-", "_") + ".py",
            "source": "import sys\nprint('not implemented')\nsys.exit(1)\n",
        })
    print(json.dumps({"schema_version": 1, "scripts": scripts}))
else:
    sys.exit(3)
"#
    .replace("__UNDERSTANDING__", &format!("{UNDERSTANDING:?}"))
    .replace("__CONTRACTS__", &contracts)
    .replace("__COUNTER__", &format!("{counter:?}"));
    fs::write(&path, script).expect("escrever Codex falso");
    let mut permissions = fs::metadata(&path).expect("metadados").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("tornar executável");
    path
}

fn run_request(project: &Path, state: &Path, codex: &Path, input: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args([
            "run",
            "--request",
            "Uma ferramenta para organizar minha pasta de downloads.",
        ])
        .current_dir(project)
        .env("HARNESS_CODEX_BIN", codex)
        .env("HARNESS_STATE_DIR", state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("harness CLI deve iniciar");
    child
        .stdin
        .take()
        .expect("stdin disponível")
        .write_all(input.as_bytes())
        .expect("enviar respostas do gate");
    child.wait_with_output().expect("aguardar harness")
}

fn single_run_dir(project: &Path) -> PathBuf {
    let runs = fs::read_dir(project.join(".harness").join("runs"))
        .expect("diretório de execuções")
        .collect::<Result<Vec<_>, _>>()
        .expect("listar execuções");
    assert_eq!(runs.len(), 1);
    runs[0].path()
}

#[test]
fn blocking_question_is_answered_before_contract_approval() {
    let project = unique_dir("approve");
    let state_dir = unique_dir("approve-state");
    let codex = write_fake_codex(&project, &[CONTRACT]);

    let output = run_request(&project, &state_dir, &codex, "tipo\ns\n");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Pergunta bloqueante: Organizar por tipo ou por data?"));
    assert!(stdout.contains("Contrato aprovado"));
    assert!(stdout.contains("Sensores de aceite implementados"));
    assert!(stdout.contains("acceptance_authoring"));

    let run = single_run_dir(&project);
    let understanding: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("understanding.json")).expect("understanding"))
            .expect("JSON válido");
    assert_eq!(understanding["answers"][0]["question_id"], "organize-by");
    assert_eq!(understanding["answers"][0]["answer"], "tipo");
    assert_eq!(understanding["answers"][0]["answered_by"], "user");

    let revisions: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("contract_revisions.json")).expect("revisões"))
            .expect("JSON válido");
    assert_eq!(revisions.as_array().expect("lista").len(), 1);

    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("run.json")).expect("run")).expect("JSON válido");
    let states = state["states"].as_array().expect("estados");
    assert!(states.iter().any(|state| state == "awaiting_human"));

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("sensor_manifest.json")).expect("manifesto"))
            .expect("manifesto JSON");
    assert_eq!(manifest["scripts"].as_array().expect("scripts").len(), 2);
    assert!(!manifest.to_string().contains("not implemented"));

    let run_id = run
        .file_name()
        .expect("run id")
        .to_string_lossy()
        .to_string();
    let sensors = state_dir.join("runs").join(&run_id).join("sensors");
    assert!(sensors.join("visible_organize_by_type.py").is_file());
    assert!(sensors.join("hidden_organize_by_type.py").is_file());
}

#[test]
fn rejected_contract_keeps_previous_wording_and_revises() {
    let project = unique_dir("reject");
    let state_dir = unique_dir("reject-state");
    let changed = CONTRACT.replace(
        "\"description\": \"Cada arquivo vai para uma pasta com o nome do seu tipo.\"",
        "\"description\": \"Cada arquivo vai para uma pasta por tipo, com nomes normalizados.\"",
    );
    let new_id = changed
        .replace("organize-by-type", "organize-by-type-v2")
        .replace(
            "\"objective\": \"Organizar a pasta de downloads por tipo.\"",
            "\"objective\": \"Organizar a pasta de downloads por tipo, com nomes normalizados.\"",
        );
    let codex = write_fake_codex(&project, &[CONTRACT, &changed, &new_id]);

    let output = run_request(
        &project,
        &state_dir,
        &codex,
        "tipo\nr\nPrefiro nomes normalizados.\ns\n",
    );

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Contrato fora das regras"),
        "a reescrita com id estável deve ser recusada: {stdout}"
    );
    assert!(stdout.contains("Contrato aprovado"));

    let run = single_run_dir(&project);
    let revisions: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("contract_revisions.json")).expect("revisões"))
            .expect("JSON válido");
    let revisions = revisions.as_array().expect("lista");
    assert_eq!(revisions.len(), 2);
    assert_eq!(
        revisions[0]["bundle"]["contract"]["criteria"][0]["id"],
        "organize-by-type"
    );
    assert_eq!(
        revisions[1]["bundle"]["contract"]["criteria"][0]["id"],
        "organize-by-type-v2"
    );
    assert_eq!(
        revisions[1]["revision_reason"],
        "Prefiro nomes normalizados."
    );
    assert!(run.join("contract_revisions/contract-v2.json").is_file());
    assert!(!run.join("contract_revisions/contract-v3.json").is_file());

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("sensor_manifest.json")).expect("manifesto"))
            .expect("manifesto JSON");
    let scripts = manifest["scripts"].as_array().expect("scripts");
    assert_eq!(scripts.len(), 2);
    assert!(
        scripts
            .iter()
            .any(|script| script["sensor_id"] == "visible-organize-by-type-v2")
    );
}
