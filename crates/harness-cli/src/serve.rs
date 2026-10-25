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
    match (method, path) {
        ("GET", "/health") => respond_json(&mut stream, 200, r#"{"status":"ok"}"#),
        ("GET", "/sessions") => respond_json(&mut stream, 200, r#"{"sessions":[]}"#),
        ("POST", "/chat") => handle_chat(&mut stream, body),
        _ => respond_json(&mut stream, 404, r#"{"error":"rota não encontrada"}"#),
    }
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
