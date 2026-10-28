use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat;

#[derive(Debug, Deserialize)]
struct ChatRequest {
    workspace: String,
    session_id: Option<String>,
    message: String,
    #[serde(default)]
    context_workspaces: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PromoteRequest {
    workspace: String,
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct ApprovePromotionRequest {
    workspace: String,
    session_id: String,
    approved: bool,
}

#[derive(Debug, Deserialize)]
struct StartRunRequest {
    workspace: String,
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct RunInputRequest {
    run_id: String,
    input: String,
}

#[derive(Debug, Deserialize)]
struct CancelRunRequest {
    workspace: String,
    run_id: String,
}

#[derive(Debug, Deserialize)]
struct LoopRequest {
    workspace: String,
    id: String,
}

#[derive(Debug, Deserialize)]
struct SetPinnedRequest {
    workspace: String,
    session_id: String,
    pinned: bool,
}

#[derive(Debug, Deserialize)]
struct SetConfigRequest {
    workspace: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    web_enabled: Option<bool>,
    #[serde(default)]
    max_tool_calls: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SaveRoutineRequest {
    workspace: String,
    id: Option<String>,
    title: String,
    prompt: String,
    #[serde(default)]
    interval_hours: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct DeleteRoutineRequest {
    workspace: String,
    id: String,
}

pub fn run(bind: &str) -> Result<(), String> {
    crate::scheduler::start();
    let listener = TcpListener::bind(bind)
        .map_err(|error| format!("não foi possível abrir {bind}: {error}"))?;
    if let Ok(workspace) = std::env::current_dir() {
        crate::worker::recover(&workspace);
    }
    // Report the bound address, not the requested one, so `--bind host:0`
    // tells the caller which port the OS picked.
    let address = listener
        .local_addr()
        .map_or_else(|_| bind.to_owned(), |address| address.to_string());
    println!("dcxd ouvindo em http://{address}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| {
                    if let Err(error) = handle(stream) {
                        eprintln!("Erro no servidor: {error}");
                    }
                });
            }
            Err(error) => eprintln!("Conexão recusada: {error}"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream) -> Result<(), String> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end;
    loop {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("requisição encerrada antes dos headers".to_owned());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
        if bytes.len() > 64 * 1024 {
            return Err("headers HTTP excedem 64 KiB".to_owned());
        }
    }
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = header_text
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("corpo HTTP incompleto".to_owned());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > header_end + content_length + 8 * 1024 * 1024 {
            return Err("corpo HTTP excede 8 MiB".to_owned());
        }
    }
    let request =
        String::from_utf8(bytes).map_err(|error| format!("requisição inválida: {error}"))?;
    let (head, body) = request.split_once("\r\n\r\n").unwrap_or((&request, ""));
    let mut lines = head.lines();
    let request_line = lines.next().ok_or("requisição sem linha inicial")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    match (method, route) {
        ("GET", "/health") => respond_json(&mut stream, 200, r#"{"status":"ok"}"#),
        ("GET", "/sessions") => handle_sessions(&mut stream, query),
        ("GET", "/sessions/search") => handle_session_search(&mut stream, query),
        ("GET", "/routines") => handle_routines(&mut stream, query),
        ("POST", "/routines") => handle_save_routine(&mut stream, body),
        ("POST", "/routines/delete") => handle_delete_routine(&mut stream, body),
        ("POST", "/sessions/pin") => handle_set_pinned(&mut stream, body),
        ("GET", "/loops") => handle_loops(&mut stream, query),
        ("POST", "/loops/pause") => handle_loop_pause(&mut stream, body),
        ("POST", "/loops/resume") => handle_loop_resume(&mut stream, body),
        ("GET", "/session") => handle_session(&mut stream, query),
        ("POST", "/chat") => handle_chat(&mut stream, body),
        ("POST", "/runs/from-session") => handle_promote(&mut stream, body),
        ("POST", "/runs/from-session/approve") => handle_approve_promotion(&mut stream, body),
        ("POST", "/runs/from-session/start") => handle_start_run(&mut stream, body),
        ("POST", "/runs/input") => handle_run_input(&mut stream, body),
        ("POST", "/runs/cancel") => handle_run_cancel(&mut stream, body),
        ("GET", "/runs/from-session/status") => handle_promotion_status(&mut stream, query),
        ("GET", "/runs/status") => handle_run_status(&mut stream, query),
        ("GET", "/runs/events") => handle_run_events(&mut stream, query),
        ("GET", "/projects") => handle_projects(&mut stream, query),
        ("GET", "/runs/artifacts") => handle_artifacts(&mut stream, query),
        ("GET", "/runs/catalog") => handle_run_catalog(&mut stream, query),
        ("GET", "/runs/artifact") => handle_artifact_content(&mut stream, query),
        ("GET", "/config") => handle_get_config(&mut stream, query),
        ("POST", "/config") => handle_set_config(&mut stream, body),
        ("GET", "/account") => handle_account(&mut stream, query),
        ("GET", "/git/branch") => handle_git_branch(&mut stream, query),
        ("GET", "/github/repos") => handle_github_repos(&mut stream, query),
        ("GET", "/github/prs") => handle_github_prs(&mut stream, query),
        _ => respond_json(&mut stream, 404, r#"{"error":"rota não encontrada"}"#),
    }
}

fn handle_sessions(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /sessions exige ?workspace=<diretório>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let sessions = chat::list_sessions(&workspace)?;
    let body = serde_json::to_string(&serde_json::json!({"sessions": sessions}))
        .map_err(|error| format!("não foi possível serializar sessões: {error}"))?;
    respond_json(stream, 200, &body)
}

fn handle_session_search(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /sessions/search exige ?workspace=<diretório>".to_owned())?;
    let term = query_value(query, "q").ok_or("GET /sessions/search exige &q=<termo>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let term = percent_decode(&term)?.trim().to_lowercase();
    if term.chars().count() < 2 {
        return respond_json(stream, 200, "{\"results\":[]}");
    }

    let sessions = chat::list_sessions(&workspace)?;
    let mut results = Vec::new();
    for summary in sessions {
        let Ok(session) = chat::session(&workspace, &summary.session_id) else {
            continue;
        };
        if let Some(message) = session
            .messages
            .iter()
            .rev()
            .find(|message| message.content.to_lowercase().contains(&term))
        {
            results.push(serde_json::json!({
                "session_id": summary.session_id,
                "title": summary.title,
                "snippet": search_excerpt(&message.content, &term),
                "role": message.role,
                "updated_at_unix_ms": message.created_at_unix_ms,
            }));
            if results.len() == 100 {
                break;
            }
        }
    }
    respond_json(
        stream,
        200,
        &serde_json::json!({"results": results}).to_string(),
    )
}

fn search_excerpt(content: &str, term: &str) -> String {
    let chars = content.chars().collect::<Vec<_>>();
    let normalized = chars
        .iter()
        .map(|ch| ch.to_lowercase().collect::<String>())
        .collect::<String>();
    let start_byte = normalized.find(term).unwrap_or(0);
    let normalized_prefix = &normalized[..start_byte];
    let start_char = normalized_prefix.chars().count().saturating_sub(80);
    let end_char = (start_char + 240).min(chars.len());
    let mut excerpt = chars[start_char..end_char].iter().collect::<String>();
    if start_char > 0 {
        excerpt.insert(0, '…');
    }
    if end_char < chars.len() {
        excerpt.push('…');
    }
    excerpt
}

fn handle_routines(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /routines exige ?workspace=<diretório>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let routines = crate::routines::list(&workspace)?;
    if routines
        .iter()
        .any(|routine| routine.interval_hours.is_some())
    {
        crate::routines::register_workspace(&workspace)?;
    }
    respond_json(
        stream,
        200,
        &serde_json::json!({"routines": routines}).to_string(),
    )
}

fn handle_save_routine(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: SaveRoutineRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    let routine = crate::routines::save(
        Path::new(&request.workspace),
        request.id.as_deref(),
        &request.title,
        &request.prompt,
        request.interval_hours,
    )?;
    if routine.interval_hours.is_some() {
        crate::routines::register_workspace(Path::new(&request.workspace))?;
    }
    respond_json(
        stream,
        200,
        &serde_json::to_string(&routine).map_err(|e| e.to_string())?,
    )
}

fn handle_delete_routine(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: DeleteRoutineRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    crate::routines::delete(Path::new(&request.workspace), &request.id)?;
    respond_json(
        stream,
        200,
        &serde_json::json!({"id": request.id, "deleted": true}).to_string(),
    )
}

fn handle_loops(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let Some(workspace) = query_value(query, "workspace") else {
        return respond_error(stream, 400, "GET /loops exige ?workspace=<diretório>");
    };
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    match crate::loop_queue::list(&workspace) {
        Ok(loops) => respond_json(
            stream,
            200,
            &serde_json::json!({"loops": loops}).to_string(),
        ),
        Err(error) => respond_error(stream, 500, &error),
    }
}

fn handle_loop_pause(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: LoopRequest = match serde_json::from_str(body.trim()) {
        Ok(request) => request,
        Err(error) => return respond_error(stream, 400, &format!("corpo JSON inválido: {error}")),
    };
    match crate::loop_queue::request_pause(Path::new(&request.workspace), &request.id) {
        Ok(()) => respond_json(
            stream,
            202,
            &serde_json::json!({"status": "pause_requested", "id": request.id}).to_string(),
        ),
        Err(error) => respond_error(stream, 404, &error),
    }
}

/// Resumes a queue as a detached `dcx loop resume`, logging to
/// `.dcx/loops/<id>.log` so the app can be closed while it runs.
fn handle_loop_resume(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: LoopRequest = match serde_json::from_str(body.trim()) {
        Ok(request) => request,
        Err(error) => return respond_error(stream, 400, &format!("corpo JSON inválido: {error}")),
    };
    let workspace = PathBuf::from(&request.workspace);
    match crate::loop_queue::is_running(&workspace, &request.id) {
        Ok(true) => return respond_error(stream, 409, "a fila já está em execução"),
        Ok(false) => {}
        Err(error) => return respond_error(stream, 404, &error),
    }
    match spawn_loop_resume(&workspace, &request.id) {
        Ok(()) => respond_json(
            stream,
            202,
            &serde_json::json!({"status": "resuming", "id": request.id}).to_string(),
        ),
        Err(error) => respond_error(stream, 500, &error),
    }
}

fn spawn_loop_resume(workspace: &Path, id: &str) -> Result<(), String> {
    let log_path = workspace
        .join(".dcx")
        .join("loops")
        .join(format!("{id}.log"));
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("não foi possível abrir {}: {error}", log_path.display()))?;
    let stderr = log.try_clone().map_err(|error| error.to_string())?;
    let mut command = std::process::Command::new(crate::worker::cli_executable()?);
    command
        .args(["loop", "resume", "--id", id])
        .current_dir(workspace)
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(stderr);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("não foi possível retomar a fila: {error}"))?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn respond_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<(), String> {
    respond_json(
        stream,
        status,
        &serde_json::json!({"error": message}).to_string(),
    )
}

fn handle_set_pinned(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: SetPinnedRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    let workspace = PathBuf::from(request.workspace);
    chat::set_pinned(&workspace, &request.session_id, request.pinned)?;
    respond_json(
        stream,
        200,
        &serde_json::json!({"session_id": request.session_id, "pinned": request.pinned})
            .to_string(),
    )
}

fn handle_session(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /session exige ?workspace=<diretório>".to_owned())?;
    let session_id =
        query_value(query, "session_id").ok_or("GET /session exige &session_id=<id>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let session_id = percent_decode(&session_id)?;
    let session = chat::session(&workspace, &session_id)?;
    respond_json(
        stream,
        200,
        &serde_json::to_string(&session).map_err(|error| error.to_string())?,
    )
}

fn handle_promote(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: PromoteRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    let workspace = PathBuf::from(request.workspace);
    let prompt = chat::request_from_session(&workspace, &request.session_id)?;
    let body = serde_json::to_string(&serde_json::json!({
        "status": "ready_for_approval",
        "session_id": request.session_id,
        "request": prompt,
        "next": "inicie o fluxo de run após a aprovação explícita do usuário"
    }))
    .map_err(|error| format!("não foi possível serializar promoção: {error}"))?;
    respond_json(stream, 200, &body)
}

fn handle_approve_promotion(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: ApprovePromotionRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    if !request.approved {
        return respond_json(
            stream,
            200,
            &serde_json::json!({
                "status": "rejected",
                "session_id": request.session_id,
            })
            .to_string(),
        );
    }
    let workspace = PathBuf::from(request.workspace);
    let approval = chat::approve_session(&workspace, &request.session_id)?;
    respond_json(
        stream,
        200,
        &serde_json::json!({
            "status": "approved",
            "session_id": approval.session_id,
            "approved_at_unix_ms": approval.approved_at_unix_ms,
            "next": format!("dcx run --from-session {}", approval.session_id),
        })
        .to_string(),
    )
}

fn handle_promotion_status(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /runs/from-session/status exige ?workspace=<diretório>".to_owned())?;
    let session_id = query_value(query, "session_id")
        .ok_or("GET /runs/from-session/status exige &session_id=<id>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let session_id = percent_decode(&session_id)?;
    let status = chat::promotion_status(&workspace, &session_id)?;
    respond_json(
        stream,
        200,
        &serde_json::to_string(&status).map_err(|error| error.to_string())?,
    )
}

fn handle_start_run(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: StartRunRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    let workspace = PathBuf::from(request.workspace);
    let prompt = chat::approved_request(&workspace, &request.session_id)?;
    let job = crate::jobs::enqueue(&workspace, &request.session_id, &prompt)?;
    crate::worker::start(&workspace, &job.run_id, &job.session_id)?;
    respond_json(
        stream,
        202,
        &serde_json::to_string(&serde_json::json!({
            "run_id": job.run_id,
            "session_id": job.session_id,
            "status": job.status,
            "next": "consulte GET /runs/status?workspace=<dir>&run_id=<id>"
        }))
        .map_err(|error| error.to_string())?,
    )
}

fn handle_run_input(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: RunInputRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    if request.input.trim().is_empty() {
        return Err("input não pode ser vazio".to_owned());
    }
    crate::worker::send_input(&request.run_id, &request.input)?;
    respond_json(
        stream,
        202,
        &serde_json::json!({"status": "sent", "run_id": request.run_id}).to_string(),
    )
}

fn handle_run_cancel(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: CancelRunRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    let workspace = PathBuf::from(request.workspace);
    crate::worker::cancel(&workspace, &request.run_id)?;
    respond_json(
        stream,
        202,
        &serde_json::json!({"status": "cancelled", "run_id": request.run_id}).to_string(),
    )
}

fn handle_run_status(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /runs/status exige ?workspace=<diretório>".to_owned())?;
    let run_id =
        query_value(query, "run_id").ok_or("GET /runs/status exige &run_id=<id>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    crate::worker::recover(&workspace);
    let run_id = percent_decode(&run_id)?;
    let job = crate::jobs::load(&workspace, &run_id)?;
    respond_json(
        stream,
        200,
        &serde_json::to_string(&job).map_err(|error| error.to_string())?,
    )
}

fn handle_run_events(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /runs/events exige ?workspace=<diretório>".to_owned())?;
    let run_id =
        query_value(query, "run_id").ok_or("GET /runs/events exige &run_id=<id>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    crate::worker::recover(&workspace);
    let run_id = percent_decode(&run_id)?;
    let mut sent = query_value(query, "since")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n")
        .map_err(|error| error.to_string())?;
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(30) {
        let events = crate::jobs::events(&workspace, &run_id)?;
        for event in events.into_iter().skip(sent) {
            let payload = format!(
                "event: {}\ndata: {}\n\n",
                event.event,
                serde_json::to_string(&event.data).map_err(|error| error.to_string())?
            );
            stream
                .write_all(payload.as_bytes())
                .map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            sent += 1;
        }
        if let Ok(job) = crate::jobs::load(&workspace, &run_id) {
            if matches!(job.status.as_str(), "done" | "failed" | "cancelled") {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    stream.flush().map_err(|error| error.to_string())
}

fn handle_projects(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let root =
        query_value(query, "root").ok_or("GET /projects exige ?root=<diretório>".to_owned())?;
    let root = PathBuf::from(percent_decode(&root)?);
    let mut projects = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let is_hidden = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.'));
        if !is_hidden
            && entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
        {
            projects.push(entry.path());
        }
    }
    projects.sort();
    respond_json(
        stream,
        200,
        &serde_json::json!({"projects": projects}).to_string(),
    )
}

fn handle_artifacts(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /runs/artifacts exige ?workspace=<diretório>".to_owned())?;
    let run_id =
        query_value(query, "run_id").ok_or("GET /runs/artifacts exige &run_id=<id>".to_owned())?;
    let root = PathBuf::from(percent_decode(&workspace)?)
        .join(".dcx")
        .join("runs")
        .join(percent_decode(&run_id)?);
    let mut files = Vec::new();
    if root.is_dir() {
        for entry in std::fs::read_dir(&root).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
            {
                files.push(entry.file_name());
            }
        }
    }
    files.sort();
    respond_json(
        stream,
        200,
        &serde_json::json!({"run_id": run_id, "files": files}).to_string(),
    )
}

fn handle_run_catalog(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    use std::collections::BTreeMap;

    let workspace = query_value(query, "workspace")
        .ok_or("GET /runs/catalog exige ?workspace=<diretório>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    crate::worker::recover(&workspace);
    let runs_root = workspace.join(".dcx").join("runs");
    let mut runs = BTreeMap::<String, Value>::new();
    if let Ok(entries) = std::fs::read_dir(&runs_root) {
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let run_id = entry.file_name().to_string_lossy().into_owned();
            if !valid_run_id(&run_id) {
                continue;
            }
            let root = entry.path();
            let understanding = read_json_value(&root.join("understanding.json"));
            let report = read_json_value(&root.join("delivery_report.json"));
            let run = read_json_value(&root.join("run.json"));
            let states = report
                .as_ref()
                .and_then(|value| value.get("states"))
                .or_else(|| run.as_ref().and_then(|value| value.get("states")))
                .cloned()
                .unwrap_or_else(|| serde_json::json!([]));
            let title = understanding
                .as_ref()
                .and_then(|value| value.get("summary").and_then(Value::as_str))
                .or_else(|| {
                    understanding
                        .as_ref()
                        .and_then(|value| value.get("goal").and_then(Value::as_str))
                })
                .unwrap_or("Execução do dcode");
            let report_summary = report
                .as_ref()
                .and_then(|value| value.get("summary").and_then(Value::as_str))
                .unwrap_or("");
            let status = states
                .as_array()
                .and_then(|items| items.last())
                .and_then(Value::as_str)
                .unwrap_or("incomplete");
            let files = artifact_names(&root);
            let item = serde_json::json!({
                "run_id": run_id,
                "title": title,
                "summary": report_summary,
                "status": status,
                "criteria_passed": report.as_ref().and_then(|value| value.get("criteria_passed")).and_then(Value::as_u64),
                "criteria_total": report.as_ref().and_then(|value| value.get("criteria_total")).and_then(Value::as_u64),
                "states": states,
                "files": files,
            });
            runs.insert(run_id, item);
        }
    }
    let jobs_root = workspace.join(".dcx").join("jobs");
    if let Ok(entries) = std::fs::read_dir(jobs_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(job) = read_json_value(&path) else {
                continue;
            };
            let Some(run_id) = job.get("run_id").and_then(Value::as_str) else {
                continue;
            };
            if !valid_run_id(run_id) {
                continue;
            }
            let root = runs_root.join(run_id);
            let files = artifact_names(&root);
            let item = serde_json::json!({
                "run_id": run_id,
                "title": job.get("request").and_then(Value::as_str).unwrap_or("Run do dcode"),
                "summary": "",
                "status": job.get("status").and_then(Value::as_str).unwrap_or("unknown"),
                "criteria_passed": null,
                "criteria_total": null,
                "states": [],
                "files": files,
            });
            runs.insert(run_id.to_owned(), item);
        }
    }
    let mut values = runs.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .get("run_id")
            .and_then(Value::as_str)
            .cmp(&left.get("run_id").and_then(Value::as_str))
    });
    respond_json(
        stream,
        200,
        &serde_json::json!({"runs": values}).to_string(),
    )
}

fn handle_artifact_content(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /runs/artifact exige ?workspace=<diretório>".to_owned())?;
    let run_id =
        query_value(query, "run_id").ok_or("GET /runs/artifact exige &run_id=<id>".to_owned())?;
    let name = query_value(query, "name")
        .ok_or("GET /runs/artifact exige &name=<arquivo.json>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let run_id = percent_decode(&run_id)?;
    let name = percent_decode(&name)?;
    if !valid_run_id(&run_id) {
        return Err("run_id inválido".to_owned());
    }
    if Path::new(&name).file_name().and_then(|part| part.to_str()) != Some(name.as_str())
        || !name.ends_with(".json")
    {
        return Err("nome de artefato inválido".to_owned());
    }
    let path = workspace.join(".dcx").join("runs").join(run_id).join(&name);
    if !std::fs::symlink_metadata(&path)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
    {
        return Err("artefato não encontrado".to_owned());
    }
    let bytes =
        std::fs::read(&path).map_err(|error| format!("não foi possível ler artefato: {error}"))?;
    if bytes.len() > 256 * 1024 {
        return Err("artefato excede o limite de 256 KiB".to_owned());
    }
    let content = String::from_utf8(bytes).map_err(|_| "artefato não é UTF-8".to_owned())?;
    respond_json(
        stream,
        200,
        &serde_json::json!({"name": name, "content": content}).to_string(),
    )
}

fn read_json_value(path: &Path) -> Option<Value> {
    if !std::fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn valid_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && !run_id.contains("..")
        && run_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "-_.".contains(ch))
}

fn artifact_names(root: &Path) -> Vec<String> {
    let mut files = std::fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let name = entry.file_name().to_str()?.to_owned();
            (file_type.is_file() && name.ends_with(".json")).then_some(name)
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn handle_get_config(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /config exige ?workspace=<diretório>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let config = chat::config(&workspace);
    respond_json(
        stream,
        200,
        &serde_json::to_string(&config).map_err(|error| error.to_string())?,
    )
}

fn handle_set_config(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: SetConfigRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    let workspace = PathBuf::from(request.workspace);
    chat::set_config(
        &workspace,
        request.provider,
        request.web_enabled,
        request.max_tool_calls,
    )?;
    let config = chat::config(&workspace);
    respond_json(
        stream,
        200,
        &serde_json::to_string(&config).map_err(|error| error.to_string())?,
    )
}

fn handle_account(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /account exige ?workspace=<diretório>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let name = git_config(&workspace, "user.name")
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "Usuário".to_owned());
    let email = git_config(&workspace, "user.email");
    respond_json(
        stream,
        200,
        &serde_json::json!({"name": name, "email": email}).to_string(),
    )
}

fn handle_git_branch(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /git/branch exige ?workspace=<diretório>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
    let branch = git_branch(&workspace);
    respond_json(
        stream,
        200,
        &serde_json::json!({"branch": branch}).to_string(),
    )
}

fn handle_github_repos(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let limit = query_value(query, "limit")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(6);
    let repos = github_repos(limit);
    respond_json(
        stream,
        200,
        &serde_json::json!({"repos": repos}).to_string(),
    )
}

/// Most recently pushed-to repos the local `gh` login can see (own repos plus
/// every org it belongs to). Uses whatever `gh auth` already has on this
/// machine; returns an empty list rather than an error when `gh` is missing
/// or not logged in, since this is a decorative panel, not a blocking action.
fn github_repos(limit: u32) -> Vec<String> {
    let output = std::process::Command::new("gh")
        .args([
            "api",
            &format!(
                "user/repos?sort=pushed&per_page={limit}&affiliation=owner,collaborator,organization_member"
            ),
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
        return Vec::new();
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|repo| repo.get("full_name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Serialize)]
struct PullRequestSummary {
    title: String,
    repo: String,
    number: u64,
    url: String,
    additions: u64,
    deletions: u64,
    files: u64,
    ready: bool,
}

fn handle_github_prs(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let limit = query_value(query, "limit")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(5);
    let prs = github_prs(limit);
    respond_json(stream, 200, &serde_json::json!({"prs": prs}).to_string())
}

/// This user's own open pull requests across every repo the local `gh` login
/// can see, enriched one by one with diff stats (the search API doesn't
/// return those). Same fail-quiet contract as `github_repos`.
fn github_prs(limit: u32) -> Vec<PullRequestSummary> {
    let output = std::process::Command::new("gh")
        .args([
            "search",
            "prs",
            "--author",
            "@me",
            "--state",
            "open",
            "--json",
            "title,number,repository,isDraft,url",
            "--limit",
            &limit.to_string(),
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
        return Vec::new();
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(github_pr_summary)
        .collect()
}

fn github_pr_summary(entry: &Value) -> Option<PullRequestSummary> {
    let title = entry.get("title")?.as_str()?.to_owned();
    let number = entry.get("number")?.as_u64()?;
    let repo = entry
        .get("repository")?
        .get("nameWithOwner")?
        .as_str()?
        .to_owned();
    let url = entry.get("url")?.as_str()?.to_owned();
    let is_draft = entry
        .get("isDraft")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let stats = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            &repo,
            "--json",
            "additions,deletions,changedFiles",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());

    Some(PullRequestSummary {
        title,
        repo,
        number,
        url,
        additions: stats
            .as_ref()
            .and_then(|s| s.get("additions"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        deletions: stats
            .as_ref()
            .and_then(|s| s.get("deletions"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        files: stats
            .as_ref()
            .and_then(|s| s.get("changedFiles"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        ready: !is_draft,
    })
}

fn git_branch(workspace: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty() && value != "HEAD").then(|| value.to_owned())
}

fn git_config(workspace: &Path, key: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("config")
        .arg(key)
        .current_dir(workspace)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_owned())
    })
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("query string contém escape incompleto".to_owned());
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                .map_err(|_| "query string inválida".to_owned())?;
            let byte = u8::from_str_radix(hex, 16)
                .map_err(|_| "query string contém escape inválido".to_owned())?;
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| "query string não é UTF-8".to_owned())
}

fn handle_chat(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let request: ChatRequest = serde_json::from_str(body.trim())
        .map_err(|error| format!("corpo JSON inválido: {error}"))?;
    let workspace = PathBuf::from(request.workspace);
    let mut session = chat::open_session(&workspace, request.session_id.as_deref())?;
    let session_id = session.session_id.clone();
    let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    stream
        .write_all(headers.as_bytes())
        .map_err(|error| error.to_string())?;
    let started = std::time::Instant::now();
    let mut send_event = |event: &str, data: serde_json::Value| -> Result<(), String> {
        let payload = format!("event: {event}\ndata: {}\n\n", data);
        stream
            .write_all(payload.as_bytes())
            .map_err(|error| error.to_string())?;
        stream.flush().map_err(|error| error.to_string())
    };
    send_event("session", serde_json::json!({"session_id": session_id}))?;
    let mut stream_event = |event: chat::ChatEvent| match event {
        chat::ChatEvent::Text(text) => {
            let _ = send_event("token", serde_json::json!({"text": text}));
        }
        chat::ChatEvent::Progress(message) => {
            let _ = send_event("progress", serde_json::json!({"message": message}));
        }
        chat::ChatEvent::ToolStart { id, name, input } => {
            let _ = send_event(
                "tool_start",
                serde_json::json!({"id": id, "name": name, "input": input}),
            );
        }
        chat::ChatEvent::ToolEnd { id, ok, output } => {
            let _ = send_event(
                "tool_end",
                serde_json::json!({"id": id, "ok": ok, "output": output}),
            );
        }
    };
    let result = chat::send_message_with_context(
        &workspace,
        &mut session,
        &request.message,
        &request.context_workspaces,
        &mut stream_event,
    );
    match result {
        Ok(reply) => send_event(
            "done",
            serde_json::json!({
                "session_id": session.session_id,
                "duration_ms": reply.duration_ms,
                "cost_usd": reply.cost_usd,
                "elapsed_ms": started.elapsed().as_millis() as u64,
            }),
        ),
        Err(error) => send_event("error", serde_json::json!({"message": error})),
    }
}

fn respond_json(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_workspace_query_is_decoded() {
        assert_eq!(
            query_value("workspace=%2Ftmp%2Fmy%20project", "workspace"),
            Some("%2Ftmp%2Fmy%20project".to_owned())
        );
        assert_eq!(
            percent_decode("%2Ftmp%2Fmy%20project").expect("decodificar"),
            "/tmp/my project"
        );
    }

    #[test]
    fn malformed_query_escape_is_rejected() {
        assert!(percent_decode("%2").is_err());
        assert!(percent_decode("%GG").is_err());
    }

    #[test]
    fn run_ids_are_safe_path_components() {
        assert!(valid_run_id("run-20261026-abc_123.ok"));
        assert!(!valid_run_id(""));
        assert!(!valid_run_id(".."));
        assert!(!valid_run_id("../outside"));
        assert!(!valid_run_id("run/id"));
        assert!(!valid_run_id("run id"));
    }

    #[test]
    fn search_excerpt_centers_on_case_insensitive_match() {
        let content = format!(
            "{} Consulte a Política de reembolso e o prazo.",
            "x".repeat(100)
        );
        let excerpt = search_excerpt(&content, "política");
        assert!(excerpt.contains("Política de reembolso"));
        assert!(excerpt.starts_with('…'));
        assert!(excerpt.len() < content.len());
    }
}
