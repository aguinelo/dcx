use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CallMetric {
    pub role: String,
    pub provider: String,
    pub model: Option<String>,
    pub duration_ms: u128,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

struct Metrics {
    calls: Vec<CallMetric>,
    started: Instant,
    started_at_unix_ms: u64,
    output_path: Option<std::path::PathBuf>,
}

static METRICS: OnceLock<Mutex<Metrics>> = OnceLock::new();

fn metrics() -> &'static Mutex<Metrics> {
    METRICS.get_or_init(|| {
        Mutex::new(Metrics {
            calls: Vec::new(),
            started: Instant::now(),
            started_at_unix_ms: now_ms(),
            output_path: None,
        })
    })
}

pub fn reset() {
    *metrics().lock().expect("metrics mutex poisoned") = Metrics {
        calls: Vec::new(),
        started: Instant::now(),
        started_at_unix_ms: now_ms(),
        output_path: None,
    };
}

pub fn set_output_path(path: &Path) -> Result<(), String> {
    let mut guard = metrics()
        .lock()
        .map_err(|_| "métricas indisponíveis".to_owned())?;
    guard.output_path = Some(path.to_path_buf());
    write_locked(&guard)
}

pub fn resume_from(path: &Path) -> Result<(), String> {
    let Ok(bytes) = std::fs::read(path) else {
        reset();
        return Ok(());
    };
    let report: SavedMetrics = serde_json::from_slice(&bytes)
        .map_err(|error| format!("metrics.json inválido: {error}"))?;
    let started_at_unix_ms = if report.started_at_unix_ms == 0 {
        now_ms()
    } else {
        report.started_at_unix_ms
    };
    let elapsed_ms = now_ms().saturating_sub(started_at_unix_ms);
    let elapsed = Duration::from_millis(elapsed_ms);
    *metrics()
        .lock()
        .map_err(|_| "métricas indisponíveis".to_owned())? = Metrics {
        calls: report.calls,
        started: Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now),
        started_at_unix_ms,
        output_path: Some(path.to_path_buf()),
    };
    Ok(())
}

pub fn before_call(provider: &str) -> Result<(), String> {
    let guard = metrics()
        .lock()
        .map_err(|_| "métricas indisponíveis".to_owned())?;
    if let Some(limit) = configured_usize("HARNESS_RUN_MAX_MODEL_CALLS")? {
        if guard.calls.len() >= limit {
            return Err(format!(
                "limite de {limit} chamadas de modelo atingido; run interrompido"
            ));
        }
    }
    if let Some(limit) = configured_f64("HARNESS_RUN_MAX_COST_USD")? {
        if provider == "codex" {
            return Err(
                "HARNESS_RUN_MAX_COST_USD exige provider com custo reportado; Codex não informa custo"
                    .to_owned(),
            );
        }
        if guard.calls.iter().any(|call| call.cost_usd.is_none()) {
            return Err(
                "teto de custo não pode ser aplicado: uma chamada anterior não reportou custo"
                    .to_owned(),
            );
        }
        let spent = guard
            .calls
            .iter()
            .filter_map(|call| call.cost_usd)
            .sum::<f64>();
        if spent >= limit {
            return Err(format!(
                "teto de custo de US$ {limit:.4} atingido (conhecido: US$ {spent:.4})"
            ));
        }
    }
    if let Some(limit) = configured_duration()? {
        if guard.started.elapsed() >= limit {
            return Err(format!(
                "limite de duração de {} minutos atingido; run interrompido",
                limit.as_secs() / 60
            ));
        }
    }
    Ok(())
}

pub fn run_provider_command(command: &mut Command, input: Option<&[u8]>) -> Result<Output, String> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("não foi possível iniciar provider: {error}"))?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| "stdin do provider indisponível".to_owned())?
            .write_all(input)
            .map_err(|error| format!("falha ao enviar contexto ao provider: {error}"))?;
    }
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout do provider indisponível".to_owned())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr do provider indisponível".to_owned())?;
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let timeout = remaining_duration()?;
    let wait_started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("falha aguardando provider: {error}"))?
        {
            break status;
        }
        if timeout.is_some_and(|limit| wait_started.elapsed() >= limit) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err("limite de duração do run atingido durante chamada ao provider".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "leitura do stdout do provider falhou".to_owned())?
        .map_err(|error| format!("falha lendo stdout do provider: {error}"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "leitura do stderr do provider falhou".to_owned())?
        .map_err(|error| format!("falha lendo stderr do provider: {error}"))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn remaining_duration() -> Result<Option<Duration>, String> {
    let Some(limit) = configured_duration()? else {
        return Ok(None);
    };
    let guard = metrics()
        .lock()
        .map_err(|_| "métricas indisponíveis".to_owned())?;
    Ok(Some(limit.saturating_sub(guard.started.elapsed())))
}

pub fn record(call: CallMetric) -> Result<(), String> {
    let mut guard = metrics()
        .lock()
        .map_err(|_| "métricas indisponíveis".to_owned())?;
    guard.calls.push(call);
    write_locked(&guard)?;
    if let Some(limit) = configured_f64("HARNESS_RUN_MAX_COST_USD")? {
        let latest = guard.calls.last().expect("chamada registrada");
        let spent = guard
            .calls
            .iter()
            .filter_map(|call| call.cost_usd)
            .sum::<f64>();
        if latest.cost_usd.is_none() {
            return Err(
                "teto de custo não pode ser aplicado: provider não reportou custo da chamada"
                    .to_owned(),
            );
        }
        if spent >= limit {
            return Err(format!(
                "teto de custo de US$ {limit:.4} atingido (conhecido: US$ {spent:.4}); run interrompido"
            ));
        }
    }
    if let Some(limit) = configured_duration()? {
        if guard.started.elapsed() >= limit {
            return Err(format!(
                "limite de duração de {} minutos atingido; run interrompido",
                limit.as_secs() / 60
            ));
        }
    }
    Ok(())
}

fn configured_usize(name: &str) -> Result<Option<usize>, String> {
    configured_value(name, |raw| {
        let value = raw
            .parse::<usize>()
            .map_err(|_| format!("{name} deve ser um inteiro positivo"))?;
        if value == 0 {
            return Err(format!("{name} deve ser maior que zero"));
        }
        Ok(value)
    })
}

fn configured_f64(name: &str) -> Result<Option<f64>, String> {
    configured_value(name, |raw| {
        let value = raw
            .parse::<f64>()
            .map_err(|_| format!("{name} deve ser um número positivo"))?;
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("{name} deve ser um número positivo"));
        }
        Ok(value)
    })
}

fn configured_duration() -> Result<Option<Duration>, String> {
    configured_value("HARNESS_RUN_MAX_DURATION_MINUTES", |raw| {
        let minutes = raw.parse::<u64>().map_err(|_| {
            "HARNESS_RUN_MAX_DURATION_MINUTES deve ser um inteiro positivo".to_owned()
        })?;
        if minutes == 0 {
            return Err("HARNESS_RUN_MAX_DURATION_MINUTES deve ser maior que zero".to_owned());
        }
        Ok(Duration::from_secs(minutes.saturating_mul(60)))
    })
}

fn configured_value<T>(
    name: &str,
    parse: impl FnOnce(&str) -> Result<T, String>,
) -> Result<Option<T>, String> {
    let raw = std::env::var(name).ok();
    raw.map(|value| parse(value.trim())).transpose()
}

#[derive(Serialize)]
struct MetricsReport<'a> {
    schema_version: u32,
    calls: &'a [CallMetric],
    started_at_unix_ms: u64,
    call_count: usize,
    total_duration_ms: u128,
    known_cost_usd: f64,
    cost_complete: bool,
}

#[derive(Deserialize)]
struct SavedMetrics {
    #[serde(default)]
    calls: Vec<CallMetric>,
    #[serde(default)]
    started_at_unix_ms: u64,
}

pub fn write(path: &Path) -> Result<(), String> {
    let guard = metrics().lock().expect("metrics mutex poisoned");
    write_to_path(&guard, path)
}

fn write_locked(guard: &Metrics) -> Result<(), String> {
    let Some(path) = &guard.output_path else {
        return Ok(());
    };
    write_to_path(guard, path)
}

fn write_to_path(guard: &Metrics, path: &Path) -> Result<(), String> {
    let calls = &guard.calls;
    let report = MetricsReport {
        schema_version: 1,
        calls,
        started_at_unix_ms: guard.started_at_unix_ms,
        call_count: calls.len(),
        total_duration_ms: calls.iter().map(|call| call.duration_ms).sum(),
        known_cost_usd: calls.iter().filter_map(|call| call.cost_usd).sum(),
        cost_complete: !calls.is_empty() && calls.iter().all(|call| call.cost_usd.is_some()),
    };
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| format!("cannot write metrics: {error}"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub fn role_for_schema(schema_name: &str) -> String {
    schema_name.trim_end_matches(".schema.json").to_owned()
}
