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
        "dcx-cli-test-{}-{nanos}-{name}",
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
import os
import sys

UNDERSTANDING = __UNDERSTANDING__
CONTRACTS = [
__CONTRACTS__
]
COUNTER = __COUNTER__

schema = ""
workspace = ""
args = sys.argv[1:]
for index, arg in enumerate(args):
    if arg == "--output-schema" and index + 1 < len(args):
        schema = args[index + 1]
    if arg == "--cd" and index + 1 < len(args):
        workspace = args[index + 1]
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
            "source": "import os\nimport sys\nworkspace = os.environ['DCX_WORKSPACE']\nif not os.path.exists(os.path.join(workspace, 'implemented.txt')):\n    print('not implemented')\n    sys.exit(1)\nprint('implemented')\n",
        })
    print(json.dumps({"schema_version": 1, "scripts": scripts}))
elif schema.endswith("plan.schema.json"):
    marker = "Contrato aprovado e suíte de sensores:\n"
    start = prompt.index(marker) + len(marker)
    end = prompt.index("\n\nRetorne apenas um plan", start)
    bundle = json.loads(prompt[start:end])
    criteria = [criterion["id"] for criterion in bundle["contract"]["criteria"]]
    sensors = [sensor["id"] for sensor in bundle["sensor_suite"]["sensors"]]
    print(json.dumps({
        "schema_version": 1,
        "contract_version": bundle["contract"]["schema_version"],
        "steps": [{
            "id": "integration",
            "title": "Implementar e validar",
            "description": "Entregar o contrato em um passo.",
            "criteria": criteria,
            "sensors": sensors,
            "depends_on": [],
            "files_hint": ["implemented.txt"],
        }],
        "integration_step_id": "integration",
        "rationale": "Tarefa pequena em um passo.",
    }))
elif schema.endswith("step_report.schema.json"):
    marker = "O campo step_id do step_report deve ser exatamente: "
    step_id = prompt.split(marker)[1].split(".")[0]
    with open(os.path.join(workspace, "implemented.txt"), "w", encoding="utf-8") as handle:
        handle.write("done\n")
    print(json.dumps({
        "schema_version": 1,
        "step_id": step_id,
        "attempts": 1,
        "summary": "Implementação criada.",
        "files_changed": [],
        "decisions": [],
        "disputes": [],
    }))
elif schema.endswith("verdict.schema.json"):
    start = prompt.index("Contrato:\n") + len("Contrato:\n")
    end = prompt.index("\nSuíte de sensores", start)
    contract = json.loads(prompt[start:end])
    print(json.dumps({
        "schema_version": 1,
        "approved": True,
        "criteria": [
            {"criterion_id": criterion["id"], "passed": True, "evidence": ["implemented"]}
            for criterion in contract["criteria"]
        ],
        "issues": [],
        "notes": [],
    }))
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args([
            "run",
            "--request",
            "Uma ferramenta para organizar minha pasta de downloads.",
        ])
        .current_dir(project)
        .env("DCX_CODEX_BIN", codex)
        .env("DCX_STATE_DIR", state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("dcx CLI deve iniciar");
    child
        .stdin
        .take()
        .expect("stdin disponível")
        .write_all(input.as_bytes())
        .expect("enviar respostas do gate");
    child.wait_with_output().expect("aguardar dcx")
}

fn single_run_dir(project: &Path) -> PathBuf {
    let runs = fs::read_dir(project.join(".dcx").join("runs"))
        .expect("diretório de execuções")
        .collect::<Result<Vec<_>, _>>()
        .expect("listar execuções");
    assert_eq!(runs.len(), 1);
    runs[0].path()
}

fn run_resume(project: &Path, state: &Path, codex: &Path, run_id: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args(["resume", "--run", run_id])
        .current_dir(project)
        .env("DCX_CODEX_BIN", codex)
        .env("DCX_STATE_DIR", state)
        .output()
        .expect("dcx resume deve iniciar")
}

fn run_dispute(
    project: &Path,
    state: &Path,
    codex: &Path,
    run_id: &str,
    input: &str,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args([
            "dispute",
            "--run",
            run_id,
            "--sensor",
            "visible-organize-by-type",
            "--reason",
            "O sensor visível não cobre o caso de arquivos duplicados.",
        ])
        .current_dir(project)
        .env("DCX_CODEX_BIN", codex)
        .env("DCX_STATE_DIR", state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("dcx dispute deve iniciar");
    child
        .stdin
        .take()
        .expect("stdin disponível")
        .write_all(input.as_bytes())
        .expect("enviar decisão");
    child.wait_with_output().expect("aguardar dispute")
}

#[test]
fn accepted_dispute_reauthorizes_only_the_contested_sensor() {
    let project = unique_dir("dispute");
    let state_dir = unique_dir("dispute-state");
    let codex = write_fake_codex(&project, &[CONTRACT]);

    let output = run_request(&project, &state_dir, &codex, "tipo\ns\n");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = single_run_dir(&project);
    let run_id = run
        .file_name()
        .expect("run id")
        .to_string_lossy()
        .to_string();
    let before: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("sensor_implementation.json")).expect("impl"))
            .expect("impl JSON");

    let output = run_dispute(&project, &state_dir, &codex, &run_id, "s\n");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Sensor visible-organize-by-type reescrito"));

    let decision: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("dispute_decision.json")).expect("decisão"))
            .expect("decisão JSON");
    assert_eq!(decision["accepted"], true);
    let after: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("sensor_implementation.json")).expect("impl"))
            .expect("impl JSON");
    assert_eq!(before["scripts"].as_array().expect("scripts").len(), 2);
    assert_eq!(after["scripts"].as_array().expect("scripts").len(), 2);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("sensor_manifest.json")).expect("manifesto"))
            .expect("manifesto JSON");
    assert_eq!(manifest["scripts"].as_array().expect("scripts").len(), 2);
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
    assert!(stdout.contains("Execução concluída. Estado: awaiting_merge"));
    assert!(stdout.contains("Revise as mudanças"));

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
    assert_eq!(
        states.last().and_then(|state| state.as_str()),
        Some("awaiting_merge")
    );

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

    assert!(run.join("plan.json").is_file());
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("verdict.json")).expect("veredito"))
            .expect("veredito JSON");
    assert_eq!(verdict["approved"], true);
    assert_eq!(verdict["criteria"].as_array().expect("critérios").len(), 1);
    assert!(run.join("delivery_report.json").is_file());
    assert!(project.join("implemented.txt").is_file());
    let metrics: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("metrics.json")).expect("métricas"))
            .expect("métricas JSON");
    assert!(metrics["call_count"].as_u64().expect("call_count") >= 5);
    assert_eq!(metrics["cost_complete"], false);
}

#[test]
fn resume_rebuilds_missing_sensors_and_plan_from_the_approved_contract() {
    let project = unique_dir("resume-partial");
    let state_dir = unique_dir("resume-partial-state");
    let codex = write_fake_codex(&project, &[CONTRACT]);

    let output = run_request(&project, &state_dir, &codex, "tipo\ns\n");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = single_run_dir(&project);
    let run_id = run
        .file_name()
        .expect("run id")
        .to_string_lossy()
        .to_string();
    fs::remove_file(run.join("plan.json")).expect("remover plano");
    fs::remove_file(run.join("sensor_implementation.json")).expect("remover implementação");
    fs::remove_dir_all(state_dir.join("runs").join(&run_id).join("sensors"))
        .expect("remover sensores materializados");
    fs::remove_file(project.join("implemented.txt")).expect("restaurar baseline");

    let output = run_resume(&project, &state_dir, &codex, &run_id);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Sensores ainda não existem"), "{stdout}");
    assert!(stdout.contains("Plano ainda não existe"), "{stdout}");
    assert!(stdout.contains("Execução concluída"), "{stdout}");
    assert!(run.join("plan.json").is_file());
    assert!(run.join("sensor_implementation.json").is_file());
    assert!(
        state_dir
            .join("runs")
            .join(&run_id)
            .join("sensors")
            .join("visible_organize_by_type.py")
            .is_file()
    );
    let events = fs::read_to_string(run.join("events.jsonl")).expect("eventos");
    assert!(events.contains("sensors.recovered"));
    assert!(events.contains("plan.recovered"));
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

    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("verdict.json")).expect("veredito"))
            .expect("veredito JSON");
    assert_eq!(verdict["approved"], true);
    assert!(project.join("implemented.txt").is_file());
}

fn run_loop_start(
    project: &Path,
    state: &Path,
    codex: &Path,
    manifest: &Path,
    max_model_calls: &str,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args(["loop", "start", "--file"])
        .arg(manifest)
        .current_dir(project)
        .env("DCX_CODEX_BIN", codex)
        .env("DCX_STATE_DIR", state)
        .env("DCX_LOOP_MAX_MODEL_CALLS", max_model_calls)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("dcx loop start deve iniciar");
    child
        .stdin
        .take()
        .expect("stdin disponível")
        .write_all(b"tipo\n")
        .expect("responder pergunta bloqueante");
    child.wait_with_output().expect("aguardar dcx loop")
}

fn run_loop_status(project: &Path, state: &Path, id: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args(["loop", "status", "--id", id])
        .current_dir(project)
        .env("DCX_STATE_DIR", state)
        .output()
        .expect("dcx loop status deve iniciar")
}

fn single_loop_state(project: &Path) -> serde_json::Value {
    let loops = fs::read_dir(project.join(".dcx").join("loops"))
        .expect("diretório de filas")
        .collect::<Result<Vec<_>, _>>()
        .expect("listar filas");
    assert_eq!(loops.len(), 1);
    serde_json::from_slice(&fs::read(loops[0].path()).expect("estado da fila"))
        .expect("estado JSON")
}

#[test]
fn loop_global_budget_pauses_after_model_call_limit() {
    let project = unique_dir("loop-budget");
    let state_dir = unique_dir("loop-budget-state");
    let codex = write_fake_codex(&project, &[CONTRACT]);
    let manifest = project.join("tasks.json");
    fs::write(
        &manifest,
        r#"{"tasks":[{"id":"one","request":"Uma tarefa para testar o orçamento global."}]}"#,
    )
    .expect("manifesto da fila");

    let output = run_loop_start(&project, &state_dir, &codex, &manifest, "1");
    assert!(
        !output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Fila pausada"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let state = single_loop_state(&project);
    assert_eq!(state["schema_version"], 2);
    assert_eq!(state["budget"]["max_model_calls"], 1);
    assert_eq!(state["usage"]["model_calls"], 1);
    assert_eq!(state["tasks"][0]["status"], "paused");
    let error = state["tasks"][0]["error"].as_str().expect("erro da tarefa");
    assert!(
        error.contains("orçamento da fila"),
        "erro inesperado: {error}"
    );

    let id = state["id"].as_str().expect("id da fila");
    let status = run_loop_status(&project, &state_dir, id);
    assert!(
        status.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("0/1 tarefa concluída"), "{stdout}");
    assert!(
        stdout.contains("Orçamento global: até 1 chamada"),
        "{stdout}"
    );
    assert!(stdout.contains("Uso: 1 chamada"), "{stdout}");
    assert!(stdout.contains("[paused] one"), "{stdout}");
    assert!(stdout.contains("orçamento da fila"), "{stdout}");
}

fn run_loop_resume(
    project: &Path,
    state: &Path,
    codex: &Path,
    id: &str,
    max_model_calls: &str,
    input: &str,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args(["loop", "resume", "--id", id])
        .current_dir(project)
        .env("DCX_CODEX_BIN", codex)
        .env("DCX_STATE_DIR", state)
        .env("DCX_LOOP_MAX_MODEL_CALLS", max_model_calls)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("dcx loop resume deve iniciar");
    child
        .stdin
        .take()
        .expect("stdin disponível")
        .write_all(input.as_bytes())
        .expect("responder gates");
    child.wait_with_output().expect("aguardar dcx loop")
}

#[test]
fn loop_resume_can_replace_the_global_budget() {
    let project = unique_dir("loop-resume-budget");
    let state_dir = unique_dir("loop-resume-budget-state");
    let codex = write_fake_codex(&project, &[CONTRACT]);
    let manifest = project.join("tasks.json");
    fs::write(
        &manifest,
        r#"{"tasks":[{"id":"one","request":"Uma tarefa para retomar com mais orçamento."}]}"#,
    )
    .expect("manifesto da fila");

    let first = run_loop_start(&project, &state_dir, &codex, &manifest, "1");
    assert!(!first.status.success());
    let state = single_loop_state(&project);
    let id = state["id"].as_str().expect("id da fila");
    assert_eq!(state["usage"]["model_calls"], 1);

    let resumed = run_loop_resume(&project, &state_dir, &codex, id, "20", "tipo\ns\n");
    assert!(
        resumed.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );

    let state = single_loop_state(&project);
    assert_eq!(state["budget"]["max_model_calls"], 20);
    assert_eq!(state["tasks"][0]["status"], "done");
    assert!(
        state["usage"]["model_calls"].as_u64().expect("chamadas") > 1,
        "o uso global deve somar as chamadas da retomada"
    );

    let status = run_loop_status(&project, &state_dir, id);
    assert!(status.status.success());
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("1/1 tarefa concluída"), "{stdout}");
    assert!(
        stdout.contains("Orçamento global: até 20 chamadas"),
        "{stdout}"
    );
}
