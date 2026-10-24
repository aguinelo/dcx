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

fn write_fake_codex(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fake-codex");
    let script = format!(
        "#!/bin/sh\nschema=\"\"\nprevious=\"\"\nfor arg in \"$@\"; do\n  if [ \"$previous\" = \"--output-schema\" ]; then schema=\"$arg\"; fi\n  previous=\"$arg\"\ndone\n{body}\n"
    );
    fs::write(&path, script).expect("escrever Codex falso");
    let mut permissions = fs::metadata(&path).expect("metadados").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("tornar executável");
    path
}

fn run_request(project: &Path, codex: &Path, input: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args([
            "run",
            "--request",
            "Uma ferramenta para organizar minha pasta de downloads.",
        ])
        .current_dir(project)
        .env("HARNESS_CODEX_BIN", codex)
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
    let codex = write_fake_codex(
        &project,
        &format!(
            "case \"$schema\" in\n  *understanding.schema.json) cat <<'JSON'\n{UNDERSTANDING}\nJSON\n    ;;\n  *contract_bundle.schema.json) cat <<'JSON'\n{CONTRACT}\nJSON\n    ;;\n  *) exit 3 ;;\nesac"
        ),
    );

    let output = run_request(&project, &codex, "tipo\ns\n");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Pergunta bloqueante: Organizar por tipo ou por data?"));
    assert!(stdout.contains("Contrato aprovado"));
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
}

#[test]
fn rejected_contract_keeps_previous_wording_and_revises() {
    let project = unique_dir("reject");
    let counter = project.join("contract-calls");
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
    let codex = write_fake_codex(
        &project,
        &format!(
            "case \"$schema\" in\n  *understanding.schema.json) cat <<'JSON'\n{UNDERSTANDING}\nJSON\n    ;;\n  *contract_bundle.schema.json)\n    count=0\n    [ -f {counter:?} ] && count=$(cat {counter:?})\n    count=$((count + 1))\n    echo \"$count\" > {counter:?}\n    case \"$count\" in\n      1) cat <<'JSON'\n{CONTRACT}\nJSON\n        ;;\n      2) cat <<'JSON'\n{changed}\nJSON\n        ;;\n      *) cat <<'JSON'\n{new_id}\nJSON\n        ;;\n    esac\n    ;;\n  *) exit 3 ;;\nesac"
        ),
    );

    let output = run_request(
        &project,
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
}
