use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;

use serde::Deserialize;

use crate::chat;

#[derive(Debug, Deserialize)]
struct ChatRequest {
    workspace: String,
    session_id: Option<String>,
    message: String,
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

pub fn run(bind: &str) -> Result<(), String> {
    let listener = TcpListener::bind(bind)
        .map_err(|error| format!("não foi possível abrir {bind}: {error}"))?;
    println!("Harness serve ouvindo em http://{bind}");
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
        ("POST", "/chat") => handle_chat(&mut stream, body),
        ("POST", "/runs/from-session") => handle_promote(&mut stream, body),
        ("POST", "/runs/from-session/approve") => handle_approve_promotion(&mut stream, body),
        ("POST", "/runs/from-session/start") => handle_start_run(&mut stream, body),
        ("POST", "/runs/input") => handle_run_input(&mut stream, body),
        ("GET", "/runs/from-session/status") => handle_promotion_status(&mut stream, query),
        ("GET", "/runs/status") => handle_run_status(&mut stream, query),
        ("GET", "/runs/events") => handle_run_events(&mut stream, query),
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
            "next": format!("harness run --from-session {}", approval.session_id),
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

fn handle_run_status(stream: &mut TcpStream, query: &str) -> Result<(), String> {
    let workspace = query_value(query, "workspace")
        .ok_or("GET /runs/status exige ?workspace=<diretório>".to_owned())?;
    let run_id =
        query_value(query, "run_id").ok_or("GET /runs/status exige &run_id=<id>".to_owned())?;
    let workspace = PathBuf::from(percent_decode(&workspace)?);
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
    let run_id = percent_decode(&run_id)?;
    let events = crate::jobs::events(&workspace, &run_id)?;
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n")
        .map_err(|error| error.to_string())?;
    for event in events {
        let payload = format!(
            "event: {}\ndata: {}\n\n",
            event.event,
            serde_json::to_string(&event.data).map_err(|error| error.to_string())?
        );
        stream
            .write_all(payload.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    stream.flush().map_err(|error| error.to_string())
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
    let mut stream_text = |text: &str| {
        let _ = send_event("token", serde_json::json!({"text": text}));
    };
    let result = chat::send_message(&workspace, &mut session, &request.message, &mut stream_text);
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
        404 => "Not Found",
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
}
