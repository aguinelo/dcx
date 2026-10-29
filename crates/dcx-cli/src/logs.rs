use std::fs;
use std::path::{Path, PathBuf};

/// Format a single session summary line.
/// Returns None when the session JSON is missing required fields.
pub fn format_session_line(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let updated_ms = value["updated_at_unix_ms"].as_u64()?;
    let provider = value["provider"].as_str().unwrap_or("—");
    let model = value["model"].as_str().unwrap_or("—");
    let messages = value["messages"].as_array().map(|m| m.len()).unwrap_or(0);
    let cost = value["calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .filter_map(|c| c["cost_usd"].as_f64())
                .sum::<f64>()
        })
        .or_else(|| value["cost_usd"].as_f64())
        .unwrap_or(0.0);
    let duration_ms: u64 = value["calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .filter_map(|c| c["duration_ms"].as_u64())
                .sum::<u64>()
        })
        .unwrap_or(0);

    let date = epoch_ms_to_date(updated_ms);
    let cost_str = if cost > 0.0 {
        format!("${cost:.4}")
    } else {
        "—".to_owned()
    };
    let dur_str = if duration_ms > 0 {
        format!("{}s", duration_ms / 1000)
    } else {
        "—".to_owned()
    };
    Some(format!(
        "{date}  {provider}  {model}  {messages} msgs  {cost_str}  {dur_str}"
    ))
}

fn epoch_ms_to_date(ms: u64) -> String {
    let secs = ms / 1000;
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    let h = hours % 24;
    let mi = mins % 60;
    let (y, mo, d) = crate::daemon_log::days_to_ymd_pub(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

pub fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut tail: usize = 50;
    let mut sessions = false;
    let mut workspace: Option<PathBuf> = None;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--tail" => {
                let n = args
                    .next()
                    .and_then(|v| v.parse::<usize>().ok())
                    .ok_or("--tail exige um número inteiro")?;
                tail = n;
            }
            "--sessions" => sessions = true,
            "--workspace" => {
                workspace = args.next().map(PathBuf::from);
            }
            _ => {
                return Err(format!(
                    "argumento desconhecido: {flag}\nUso: dcx logs [--tail N] [--sessions] [--workspace <dir>]"
                ));
            }
        }
    }

    if sessions {
        let ws = workspace
            .or_else(|| std::env::current_dir().ok())
            .ok_or("não foi possível determinar o workspace")?;
        print_sessions(&ws)?;
    } else {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("variável HOME não definida")?;
        print_daemon_log(&home.join(".dcx").join("daemon.log"), tail);
    }
    Ok(())
}

fn print_daemon_log(log_path: &Path, tail: usize) {
    match fs::read_to_string(log_path) {
        Ok(contents) => {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(tail);
            for line in &lines[start..] {
                println!("{line}");
            }
        }
        Err(_) => {
            println!("Nenhum log encontrado em {}", log_path.display());
        }
    }
}

fn print_sessions(workspace: &Path) -> Result<(), String> {
    let sessions_dir = workspace.join(".dcx").join("sessions");
    if !sessions_dir.is_dir() {
        println!("Nenhuma sessão encontrada em {}", sessions_dir.display());
        return Ok(());
    }
    let mut entries: Vec<(u64, String)> = fs::read_dir(&sessions_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let session_file = entry.path().join("session.json");
            let raw = fs::read_to_string(&session_file).ok()?;
            let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
            let ms = value["updated_at_unix_ms"].as_u64().unwrap_or(0);
            let line = format_session_line(&raw)?;
            Some((ms, line))
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (_, line) in entries.iter().take(20) {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_session_line_with_full_data() {
        let raw = serde_json::json!({
            "updated_at_unix_ms": 1761782004000u64,
            "provider": "minimax",
            "model": "MiniMax-M3.1-Flash",
            "messages": [{"role":"user"},{"role":"assistant"},{"role":"user"},{"role":"assistant"}],
            "calls": [
                {"cost_usd": 0.0021, "duration_ms": 12500u64},
                {"cost_usd": 0.0021, "duration_ms": 12500u64}
            ]
        })
        .to_string();
        let line = format_session_line(&raw).expect("should format");
        assert!(line.contains("minimax"), "provider");
        assert!(line.contains("MiniMax-M3.1-Flash"), "model");
        assert!(line.contains("4 msgs"), "message count");
        assert!(line.contains("$0.0042"), "cost");
        assert!(line.contains("25s"), "duration");
    }

    #[test]
    fn format_session_line_missing_cost_shows_dash() {
        let raw = serde_json::json!({
            "updated_at_unix_ms": 1761782004000u64,
            "provider": "claude",
            "messages": []
        })
        .to_string();
        let line = format_session_line(&raw).expect("should format");
        assert!(line.contains("claude"));
        assert!(line.contains("—"), "cost should be dash");
    }

    #[test]
    fn format_session_line_returns_none_for_missing_required_fields() {
        assert!(format_session_line("{}").is_none());
        assert!(format_session_line("not json").is_none());
    }
}
