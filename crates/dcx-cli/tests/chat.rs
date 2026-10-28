#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio antes da época")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "dcx-chat-test-{}-{nanos}-{name}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("criar diretório de teste");
    path
}

fn write_fake_claude(dir: &Path) -> PathBuf {
    let path = dir.join("fake-claude");
    let script = r#"#!/usr/bin/env python3
import json
import os
import sys

args = sys.argv[1:]
prompt = args[-1]
args_log = os.environ.get("DCX_FAKE_CHAT_ARGS")
if args_log:
    with open(args_log, "w", encoding="utf-8") as handle:
        handle.write("\n".join(args))
prompt_log = os.environ.get("DCX_FAKE_CHAT_LOG")
if prompt_log:
    with open(prompt_log, "a", encoding="utf-8") as handle:
        handle.write(prompt + "\n---\n")
api_key_log = os.environ.get("DCX_FAKE_CHAT_API_KEY_LOG")
if api_key_log:
    with open(api_key_log, "w", encoding="utf-8") as handle:
        handle.write(os.environ.get("ANTHROPIC_API_KEY", ""))
env_log = os.environ.get("DCX_FAKE_CHAT_ENV_LOG")
if env_log:
    keys = [
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
    ]
    with open(env_log, "w", encoding="utf-8") as handle:
        for key in keys:
            handle.write("%s=%s\n" % (key, os.environ.get(key, "")))
counter_path = os.environ["DCX_FAKE_CHAT_COUNTER"]
try:
    with open(counter_path, encoding="utf-8") as handle:
        count = int(handle.read())
except FileNotFoundError:
    count = 0
count += 1
with open(counter_path, "w", encoding="utf-8") as handle:
    handle.write(str(count))
prefix = os.environ.get("DCX_FAKE_CHAT_PREFIX", "resposta")
text = "%s-%d" % (prefix, count)
first = text[:4]
rest = text[4:]
if os.environ.get("DCX_FAKE_CHAT_OVERFLOW") == "1":
    for index in range(2):
        event = {
            "type": "assistant",
            "message": {
                "usage": {
                    "input_tokens": 42,
                    "output_tokens": 7,
                    "cache_creation_input_tokens": 3,
                    "cache_read_input_tokens": 11,
                },
                "content": [{
                    "type": "tool_use",
                    "id": "overflow-%d" % index,
                    "name": "Read",
                    "input": {"file_path": "README.md"},
                }],
            },
        }
        print(json.dumps(event), flush=True)
if os.environ.get("DCX_FAKE_CHAT_NARRATE") == "1":
    narration = "I'll read the README.md file."
    print(json.dumps({"type": "assistant", "message": {"content": [{"type": "text", "text": narration}, {"type": "tool_use", "id": "tool-1", "name": "Read", "input": {"file_path": "README.md"}}]}}), flush=True)
    print(json.dumps({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "tool-1", "content": "marker"}]}}), flush=True)
print(json.dumps({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": first}}}), flush=True)
print(json.dumps({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": rest}}}), flush=True)
print(json.dumps({"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}), flush=True)
print(json.dumps({"type": "result", "subtype": "success", "is_error": False, "result": text, "total_cost_usd": 0.001, "duration_ms": 7, "usage": {"input_tokens": 10, "output_tokens": 2}}), flush=True)
"#;
    fs::write(&path, script).expect("escrever Claude falso");
    let mut permissions = fs::metadata(&path).expect("metadados").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("tornar executável");
    path
}

fn run_chat(project: &Path, args: &[&str], input: &str, fake: &Path) -> std::process::Output {
    run_chat_with_env(project, args, input, fake, &[])
}

fn run_chat_with_env(
    project: &Path,
    args: &[&str],
    input: &str,
    fake: &Path,
    extra_env: &[(&str, &str)],
) -> std::process::Output {
    let counter = project.join("fake-counter");
    let prompt_log = project.join("fake-prompts.log");
    let args_log = project.join("fake-args.log");
    let api_key_log = project.join("fake-api-key.log");
    let env_log = project.join("fake-env.log");
    let mut command = Command::new(env!("CARGO_BIN_EXE_dcx"));
    command
        .args(args)
        .current_dir(project)
        .env("DCX_CLAUDE_BIN", fake)
        .env("DCX_FAKE_CHAT_COUNTER", &counter)
        .env("DCX_FAKE_CHAT_LOG", &prompt_log)
        .env("DCX_FAKE_CHAT_ARGS", &args_log)
        .env("DCX_FAKE_CHAT_API_KEY_LOG", &api_key_log)
        .env("DCX_FAKE_CHAT_ENV_LOG", &env_log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("dcx chat deve iniciar");
    child
        .stdin
        .take()
        .expect("stdin disponível")
        .write_all(input.as_bytes())
        .expect("enviar entrada");
    child.wait_with_output().expect("aguardar chat")
}

fn only_session(project: &Path) -> (String, serde_json::Value) {
    let sessions = fs::read_dir(project.join(".dcx").join("sessions"))
        .expect("diretório de sessões")
        .collect::<Result<Vec<_>, _>>()
        .expect("listar sessões");
    assert_eq!(sessions.len(), 1);
    let id = sessions[0].file_name().to_string_lossy().to_string();
    let session = serde_json::from_slice(
        &fs::read(sessions[0].path().join("session.json")).expect("session.json"),
    )
    .expect("sessão JSON");
    (id, session)
}

#[test]
fn chat_streams_and_persists_a_read_only_session() {
    let project = unique_dir("chat");
    fs::write(project.join("README.md"), "original\n").expect("README");
    let fake = write_fake_claude(&project);

    let output = run_chat(&project, &["chat"], "olá\n/exit\n", &fake);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("modo read-only"));
    assert!(stdout.contains("resposta-1"));
    assert_eq!(
        fs::read_to_string(project.join("README.md")).expect("README"),
        "original\n"
    );
    assert!(!project.join("implemented.txt").exists());

    let (_, session) = only_session(&project);
    assert_eq!(session["schema_version"], 1);
    assert_eq!(session["mode"], "explore");
    assert_eq!(session["provider"], "claude");
    let messages = session["messages"].as_array().expect("mensagens");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "olá");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"], "resposta-1");
    assert_eq!(messages[1]["cost_usd"], 0.001);
    assert_eq!(messages[1]["duration_ms"], 7);

    let args = fs::read_to_string(project.join("fake-args.log")).expect("args");
    assert!(args.lines().any(|line| line == "stream-json"));
    assert!(args.lines().any(|line| line == "plan"));
    assert!(args.lines().any(|line| line == "Read,Glob,Grep"));
    assert!(args.lines().any(|line| line == "--strict-mcp-config"));
    assert!(args.lines().any(|line| line == "--disable-slash-commands"));
    assert!(
        args.lines()
            .any(|line| line == "--allowedTools=Read,Glob,Grep")
    );
    assert!(args.lines().any(|line| line
        == "--disallowedTools=Bash,Edit,Write,NotebookEdit,WebSearch,WebFetch,ToolSearch,Agent"));
    assert!(!args.contains("acceptEdits"));
    assert!(args.contains("modo conversa read-only"));
}

#[test]
fn chat_resumes_history_and_keeps_context() {
    let project = unique_dir("chat-resume");
    let fake = write_fake_claude(&project);

    let first = run_chat(&project, &["chat"], "olá\n/exit\n", &fake);
    assert!(
        first.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let (id, _) = only_session(&project);

    let second = run_chat(
        &project,
        &["chat", "--session", &id],
        "de novo\n/exit\n",
        &fake,
    );
    assert!(
        second.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("Sessão:"));
    assert!(stdout.contains("resposta-2"));

    let (_, session) = only_session(&project);
    let messages = session["messages"].as_array().expect("mensagens");
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[2]["content"], "de novo");

    let prompts = fs::read_to_string(project.join("fake-prompts.log")).expect("prompts");
    assert!(prompts.contains("Usuário: olá"));
    assert!(prompts.contains("Assistente: resposta-1"));
    assert!(prompts.contains("Usuário: de novo"));
}

#[test]
fn chat_lists_sessions() {
    let project = unique_dir("chat-list");
    let fake = write_fake_claude(&project);
    let first = run_chat(&project, &["chat"], "olá\n/exit\n", &fake);
    assert!(first.status.success());
    let (id, _) = only_session(&project);

    let output = run_chat(&project, &["chat", "--list"], "", &fake);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Sessões em"));
    assert!(stdout.contains(&id));
    assert!(stdout.contains("2 mensagem(ns)"));
}

#[test]
fn chat_loads_dotenv_and_shell_values_win() {
    let project = unique_dir("chat-env");
    fs::write(project.join(".env"), "DCX_FAKE_CHAT_PREFIX=dotenv\n").expect(".env");
    let fake = write_fake_claude(&project);

    let output = run_chat(&project, &["chat"], "olá\n/exit\n", &fake);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("dotenv-1"));

    let output = run_chat_with_env(
        &project,
        &["chat"],
        "olá\n/exit\n",
        &fake,
        &[("DCX_FAKE_CHAT_PREFIX", "shell")],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("shell-2"));
}

#[test]
fn chat_prefers_the_dotenv_api_key_over_a_shell_value() {
    let project = unique_dir("chat-api-key");
    fs::write(
        project.join(".env"),
        "ANTHROPIC_API_KEY=from-dotenv\nDCX_FAKE_CHAT_PREFIX=dotenv\n",
    )
    .expect(".env");
    let fake = write_fake_claude(&project);

    let output = run_chat_with_env(
        &project,
        &["chat"],
        "olá\n/exit\n",
        &fake,
        &[("ANTHROPIC_API_KEY", "from-shell")],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(project.join("fake-api-key.log")).expect("chave registrada"),
        "from-dotenv"
    );
}

#[test]
fn chat_uses_the_minimax_anthropic_compatible_endpoint() {
    let project = unique_dir("chat-minimax");
    fs::write(
        project.join(".env"),
        "DCX_CHAT_PROVIDER=minimax\n\
         MINIMAX_API_KEY=minimax-key\n\
         DCX_MINIMAX_BASE_URL=https://api.minimax.io/anthropic\n\
         DCX_MINIMAX_MODEL=MiniMax-M3\n",
    )
    .expect(".env");
    let fake = write_fake_claude(&project);

    let output = run_chat_with_env(
        &project,
        &["chat"],
        "olá\n/exit\n",
        &fake,
        &[("ANTHROPIC_API_KEY", "from-shell")],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let envs = fs::read_to_string(project.join("fake-env.log")).expect("env do MiniMax");
    assert!(envs.contains("ANTHROPIC_BASE_URL=https://api.minimax.io/anthropic"));
    assert!(envs.contains("ANTHROPIC_AUTH_TOKEN=minimax-key"));
    assert!(envs.contains("ANTHROPIC_API_KEY=\n"));
    assert!(envs.contains("ANTHROPIC_MODEL=MiniMax-M3"));
    assert!(envs.contains("ANTHROPIC_SMALL_FAST_MODEL=MiniMax-M3"));

    let args = fs::read_to_string(project.join("fake-args.log")).expect("args");
    assert!(args.lines().any(|line| line == "MiniMax-M3"));

    let (_, session) = only_session(&project);
    assert_eq!(session["provider"], "minimax");
    assert_eq!(session["model"], "MiniMax-M3");
}

#[test]
fn chat_stores_only_the_final_answer_not_pre_tool_narration() {
    let project = unique_dir("narration");
    let fake = write_fake_claude(&project);
    let output = run_chat_with_env(
        &project,
        &["chat"],
        "leia o README\n/exit\n",
        &fake,
        &[("DCX_FAKE_CHAT_NARRATE", "1")],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let (_, session) = only_session(&project);
    let assistant = session["messages"]
        .as_array()
        .expect("mensagens")
        .iter()
        .rev()
        .find(|message| message["role"] == "assistant")
        .expect("resposta do assistente");
    assert_eq!(assistant["content"], "resposta-1");
}

#[test]
fn interrupted_tool_limit_persists_partial_usage_and_marks_cost_unknown() {
    let project = unique_dir("chat-tool-limit");
    let fake = write_fake_claude(&project);

    let output = run_chat_with_env(
        &project,
        &["chat"],
        "olá\n/exit\n",
        &fake,
        &[
            ("DCX_CHAT_MAX_TOOL_CALLS", "1"),
            ("DCX_FAKE_CHAT_OVERFLOW", "1"),
        ],
    );
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("custo exato indisponível"));

    let (_, session) = only_session(&project);
    let calls = session["interrupted_calls"]
        .as_array()
        .expect("tentativa interrompida salva");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["reason"], "tool_limit");
    assert_eq!(calls[0]["cost_usd"], serde_json::Value::Null);
    assert_eq!(calls[0]["cost_complete"], false);
    assert_eq!(calls[0]["input_tokens"], 42);
    assert_eq!(calls[0]["output_tokens"], 7);
    assert_eq!(calls[0]["cache_creation_input_tokens"], 3);
    assert_eq!(calls[0]["cache_read_input_tokens"], 11);
}
