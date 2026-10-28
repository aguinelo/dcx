use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn unique_workspace(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "dcx-loops-test-{}-{nanos}-{name}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("workspace");
    fs::canonicalize(path).expect("caminho canônico")
}

fn write_loop(workspace: &Path, id: &str, statuses: &[&str], pid: Option<u32>) {
    let tasks: Vec<serde_json::Value> = statuses
        .iter()
        .enumerate()
        .map(|(index, status)| {
            serde_json::json!({
                "id": format!("task-{}", index + 1),
                "request": format!("Pedido {}", index + 1),
                "status": status,
                "run_id": null,
                "error": null,
            })
        })
        .collect();
    let current = statuses
        .iter()
        .position(|status| *status != "done")
        .unwrap_or(statuses.len());
    let state = serde_json::json!({
        "schema_version": 2,
        "id": id,
        "workspace": workspace,
        "created_at_unix_ms": 1,
        "updated_at_unix_ms": 2,
        "current_task": current,
        "tasks": tasks,
        "pid": pid,
    });
    let dir = workspace.join(".dcx").join("loops");
    fs::create_dir_all(&dir).expect("diretório de filas");
    fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_vec_pretty(&state).expect("json"),
    )
    .expect("estado da fila");
}

struct Daemon {
    child: Child,
    port: u16,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Lets the OS pick the port and reads it back from dcxd's startup line, so
/// parallel tests can't race for a port picked ahead of time.
fn start_daemon(cwd: &Path) -> Daemon {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dcxd"))
        .args(["--bind", "127.0.0.1:0"])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("dcxd deve iniciar");
    let stdout = child.stdout.take().expect("stdout do dcxd");
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stdout), &mut line)
        .expect("linha de início do dcxd");
    let port = line
        .trim()
        .rsplit(':')
        .next()
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("porta não encontrada em {line:?}"));
    Daemon { child, port }
}

fn request(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("conectar");
    let body = body.unwrap_or("");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("enviar");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("ler");
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("status HTTP");
    let payload = response
        .split_once("\r\n\r\n")
        .map(|(_, payload)| payload.to_owned())
        .unwrap_or_default();
    (status, payload)
}

fn encode(value: &Path) -> String {
    value
        .display()
        .to_string()
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[test]
fn daemon_lists_the_workspace_loops_with_progress() {
    let workspace = unique_workspace("list");
    write_loop(&workspace, "loop-a", &["done", "queued"], None);
    let daemon = start_daemon(&workspace);

    let (status, body) = request(
        daemon.port,
        "GET",
        &format!("/loops?workspace={}", encode(&workspace)),
        None,
    );

    assert_eq!(status, 200, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    let loops = json["loops"].as_array().expect("lista de filas");
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0]["id"], "loop-a");
    assert_eq!(loops[0]["status"], "paused");
    assert_eq!(loops[0]["done"], 1);
    assert_eq!(loops[0]["total"], 2);
}

#[test]
fn daemon_pause_request_is_recorded_for_the_running_queue() {
    let workspace = unique_workspace("pause");
    write_loop(&workspace, "loop-p", &["queued"], None);
    let daemon = start_daemon(&workspace);

    let body = serde_json::json!({"workspace": workspace, "id": "loop-p"}).to_string();
    let (status, response) = request(daemon.port, "POST", "/loops/pause", Some(&body));

    assert_eq!(status, 202, "{response}");
    assert!(workspace.join(".dcx/loops/loop-p.pause").is_file());
}

#[test]
fn daemon_refuses_to_resume_a_queue_that_is_already_running() {
    let workspace = unique_workspace("busy");
    write_loop(
        &workspace,
        "loop-busy",
        &["running"],
        Some(std::process::id()),
    );
    let daemon = start_daemon(&workspace);

    let body = serde_json::json!({"workspace": workspace, "id": "loop-busy"}).to_string();
    let (status, response) = request(daemon.port, "POST", "/loops/resume", Some(&body));

    assert_eq!(status, 409, "{response}");
}

#[test]
fn daemon_resumes_a_queue_in_the_background() {
    let workspace = unique_workspace("resume");
    write_loop(&workspace, "loop-r", &["done"], None);
    let daemon = start_daemon(&workspace);

    let body = serde_json::json!({"workspace": workspace, "id": "loop-r"}).to_string();
    let (status, response) = request(daemon.port, "POST", "/loops/resume", Some(&body));
    assert_eq!(status, 202, "{response}");

    let log = workspace.join(".dcx/loops/loop-r.log");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = fs::read_to_string(&log).unwrap_or_default();
        if text.contains("concluída") {
            break;
        }
        assert!(Instant::now() < deadline, "a fila não foi retomada: {text}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn cli_pause_records_the_request() {
    let workspace = unique_workspace("cli-pause");
    write_loop(&workspace, "loop-cli", &["queued"], None);

    let output = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args(["loop", "pause", "--id", "loop-cli"])
        .current_dir(&workspace)
        .output()
        .expect("dcx loop pause deve iniciar");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(workspace.join(".dcx/loops/loop-cli.pause").is_file());
}
