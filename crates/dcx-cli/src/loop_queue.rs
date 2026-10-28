use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::metrics;

const MAX_TASKS: usize = 25;
const MAX_REQUEST_CHARS: usize = 12_000;
const LOOP_SCHEMA_VERSION: u32 = 2;

#[derive(Deserialize)]
struct Manifest {
    tasks: Vec<ManifestTask>,
}

#[derive(Deserialize)]
struct ManifestTask {
    id: String,
    request: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LoopTask {
    id: String,
    request: String,
    status: String,
    run_id: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct LoopBudget {
    #[serde(default)]
    max_model_calls: Option<usize>,
    #[serde(default)]
    max_cost_usd: Option<f64>,
    #[serde(default)]
    max_duration_minutes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LoopUsage {
    #[serde(default)]
    model_calls: u64,
    #[serde(default)]
    known_cost_usd: f64,
    #[serde(default = "default_true")]
    cost_complete: bool,
    #[serde(default)]
    duration_ms: u64,
}

impl Default for LoopUsage {
    fn default() -> Self {
        Self {
            model_calls: 0,
            known_cost_usd: 0.0,
            cost_complete: true,
            duration_ms: 0,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LoopState {
    schema_version: u32,
    id: String,
    workspace: PathBuf,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
    current_task: usize,
    tasks: Vec<LoopTask>,
    #[serde(default)]
    budget: LoopBudget,
    #[serde(default)]
    usage: LoopUsage,
}

impl LoopBudget {
    fn from_env() -> Result<Self, String> {
        Ok(Self {
            max_model_calls: env_usize("DCX_LOOP_MAX_MODEL_CALLS")?,
            max_cost_usd: env_f64("DCX_LOOP_MAX_COST_USD")?,
            max_duration_minutes: env_u64("DCX_LOOP_MAX_DURATION_MINUTES")?,
        })
    }

    fn from_env_override() -> Result<Option<Self>, String> {
        if env_present("DCX_LOOP_MAX_MODEL_CALLS")
            || env_present("DCX_LOOP_MAX_COST_USD")
            || env_present("DCX_LOOP_MAX_DURATION_MINUTES")
        {
            Ok(Some(Self::from_env()?))
        } else {
            Ok(None)
        }
    }

    fn to_metrics(&self) -> metrics::LoopLimits {
        metrics::LoopLimits {
            max_model_calls: self.max_model_calls,
            max_cost_usd: self.max_cost_usd,
            max_duration: self
                .max_duration_minutes
                .map(|minutes| Duration::from_secs(minutes.saturating_mul(60))),
        }
    }

    fn exhausted(&self, usage: &LoopUsage) -> Option<String> {
        if let Some(limit) = self.max_model_calls {
            if usage.model_calls >= u64::try_from(limit).unwrap_or(u64::MAX) {
                let label = if limit == 1 { "chamada" } else { "chamadas" };
                return Some(format!("limite de {limit} {label} de modelo atingido"));
            }
        }
        if let Some(limit) = self.max_cost_usd {
            if !usage.cost_complete {
                return Some("uma chamada anterior não reportou custo".to_owned());
            }
            if usage.known_cost_usd >= limit {
                return Some(format!(
                    "teto de custo de US$ {limit:.4} atingido (conhecido: US$ {:.4})",
                    usage.known_cost_usd
                ));
            }
        }
        if let Some(limit) = self.max_duration_minutes {
            let limit_ms = limit.saturating_mul(60).saturating_mul(1000);
            if usage.duration_ms >= limit_ms {
                return Some(format!("limite de duração de {limit} minutos atingido"));
            }
        }
        None
    }

    fn has_limits(&self) -> bool {
        self.max_model_calls.is_some()
            || self.max_cost_usd.is_some()
            || self.max_duration_minutes.is_some()
    }

    fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(limit) = self.max_model_calls {
            let label = if limit == 1 { "chamada" } else { "chamadas" };
            parts.push(format!("até {limit} {label}"));
        }
        if let Some(limit) = self.max_cost_usd {
            parts.push(format!("US$ {limit:.4}"));
        }
        if let Some(limit) = self.max_duration_minutes {
            parts.push(format!("{limit} min"));
        }
        if parts.is_empty() {
            "sem limite global".to_owned()
        } else {
            parts.join(" · ")
        }
    }
}

impl LoopUsage {
    fn from_metrics(usage: metrics::Usage) -> Self {
        Self {
            model_calls: u64::try_from(usage.model_calls).unwrap_or(u64::MAX),
            known_cost_usd: usage.known_cost_usd,
            cost_complete: usage.cost_complete,
            duration_ms: usage.duration_ms,
        }
    }

    fn to_metrics(&self) -> metrics::Usage {
        metrics::Usage {
            model_calls: usize::try_from(self.model_calls).unwrap_or(usize::MAX),
            known_cost_usd: self.known_cost_usd,
            cost_complete: self.cost_complete,
            duration_ms: self.duration_ms,
        }
    }

    fn summary(&self) -> String {
        let cost = if self.cost_complete {
            format!("US$ {:.4}", self.known_cost_usd)
        } else {
            format!("US$ {:.4} (incompleto)", self.known_cost_usd)
        };
        let label = if self.model_calls == 1 {
            "chamada"
        } else {
            "chamadas"
        };
        format!(
            "{} {} · {} · {}",
            self.model_calls,
            label,
            cost,
            duration_label(self.duration_ms)
        )
    }
}

fn duration_label(duration_ms: u64) -> String {
    if duration_ms < 60_000 {
        format!("{}s", duration_ms / 1000)
    } else {
        format!("{} min", duration_ms / 60_000)
    }
}

pub fn start(manifest_path: &Path) -> Result<(), String> {
    let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
    let bytes = fs::read(manifest_path)
        .map_err(|error| format!("não foi possível ler {}: {error}", manifest_path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("manifesto da fila inválido: {error}"))?;
    validate_manifest(&manifest)?;
    let budget = LoopBudget::from_env()?;
    let now = now_ms();
    let id = format!("loop-{now}-{}", std::process::id());
    let tasks = manifest
        .tasks
        .into_iter()
        .map(|task| LoopTask {
            id: task.id,
            request: task.request.trim().to_owned(),
            status: "queued".to_owned(),
            run_id: None,
            error: None,
        })
        .collect();
    let state = LoopState {
        schema_version: LOOP_SCHEMA_VERSION,
        id: id.clone(),
        workspace,
        created_at_unix_ms: now,
        updated_at_unix_ms: now,
        current_task: 0,
        tasks,
        budget,
        usage: LoopUsage::default(),
    };
    save(&state)?;
    println!("Fila finita criada: {id} ({} tarefas)", state.tasks.len());
    println!("Orçamento global: {}", state.budget.summary());
    execute(state)
}

pub fn resume(id: &str) -> Result<(), String> {
    validate_id(id)?;
    let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
    let path = state_path(&workspace, id);
    let bytes = fs::read(&path)
        .map_err(|error| format!("fila {id} não encontrada em {}: {error}", path.display()))?;
    let mut state: LoopState = serde_json::from_slice(&bytes)
        .map_err(|error| format!("estado da fila inválido: {error}"))?;
    if state.workspace != workspace {
        return Err("a fila só pode ser retomada no workspace em que foi criada".to_owned());
    }
    if let Some(budget) = LoopBudget::from_env_override()? {
        state.budget = budget;
        state.updated_at_unix_ms = now_ms();
        save(&state)?;
        println!("Orçamento global atualizado: {}", state.budget.summary());
    }
    execute(state)
}

pub fn status(id: &str) -> Result<(), String> {
    validate_id(id)?;
    let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
    let path = state_path(&workspace, id);
    let bytes = fs::read(&path)
        .map_err(|error| format!("fila {id} não encontrada em {}: {error}", path.display()))?;
    let state: LoopState = serde_json::from_slice(&bytes)
        .map_err(|error| format!("estado da fila inválido: {error}"))?;
    println!("{}", format_status(&state));
    Ok(())
}

fn format_status(state: &LoopState) -> String {
    let done = state
        .tasks
        .iter()
        .filter(|task| task.status == "done")
        .count();
    let task_label = if state.tasks.len() == 1 {
        "tarefa concluída"
    } else {
        "tarefas concluídas"
    };
    let mut lines = vec![
        format!(
            "Fila {} — {}/{} {}",
            state.id,
            done,
            state.tasks.len(),
            task_label
        ),
        format!("Workspace: {}", state.workspace.display()),
        format!("Orçamento global: {}", state.budget.summary()),
        format!("Uso: {}", state.usage.summary()),
    ];
    if let Some(task) = state.tasks.get(state.current_task) {
        lines.push(format!("Tarefa atual: {} ({})", task.id, task.status));
    }
    lines.push("Tarefas:".to_owned());
    for (index, task) in state.tasks.iter().enumerate() {
        let mut line = format!("  {}. [{}] {}", index + 1, task.status, task.id);
        if let Some(run_id) = &task.run_id {
            line.push_str(&format!(" · run {run_id}"));
        }
        if let Some(error) = &task.error {
            line.push_str(&format!(" · {}", error.lines().next().unwrap_or("")));
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn execute(mut state: LoopState) -> Result<(), String> {
    metrics::set_loop_budget(state.budget.to_metrics(), state.usage.to_metrics())?;
    let result = run_tasks(&mut state);
    sync_usage(&mut state);
    state.updated_at_unix_ms = now_ms();
    let save_result = save(&state);
    metrics::clear_loop_budget();
    save_result?;
    result
}

fn run_tasks(state: &mut LoopState) -> Result<(), String> {
    while state.current_task < state.tasks.len() {
        if let Some(reason) = state.budget.exhausted(&state.usage) {
            state.updated_at_unix_ms = now_ms();
            save(state)?;
            return Err(format!(
                "orçamento da fila esgotado: {reason}. Ajuste os limites e retome com `dcx loop resume --id {}`",
                state.id
            ));
        }
        let index = state.current_task;
        let task_id = state.tasks[index].id.clone();
        println!(
            "\nFila {} — tarefa {}/{}: {}",
            state.id,
            index + 1,
            state.tasks.len(),
            task_id
        );
        if state.budget.has_limits() {
            println!(
                "Orçamento da fila: {} (usado: {})",
                state.budget.summary(),
                state.usage.summary()
            );
        }
        state.tasks[index].status = "running".to_owned();
        state.tasks[index].error = None;
        state.updated_at_unix_ms = now_ms();
        save(state)?;

        let task_request = state.tasks[index].request.clone();
        let existing_run = state.tasks[index].run_id.clone().filter(|run_id| {
            state
                .workspace
                .join(".dcx")
                .join("runs")
                .join(run_id)
                .join("contract_bundle.json")
                .is_file()
        });
        let result = if let Some(run_id) = existing_run {
            println!("Retomando o run {run_id} da tarefa {task_id}");
            crate::interactive::resume(&run_id).map_err(|error| error.to_string())
        } else {
            crate::interactive::run_tracked(&task_request, |run_id| {
                state.tasks[index].run_id = Some(run_id.to_owned());
                state.updated_at_unix_ms = now_ms();
                save(state)?;
                println!("Run {} associado à tarefa {task_id}", run_id);
                Ok(())
            })
            .map_err(|error| error.to_string())
        };

        sync_usage(state);
        match result {
            Ok(()) => {
                state.tasks[index].status = "done".to_owned();
                state.current_task += 1;
                state.updated_at_unix_ms = now_ms();
                save(state)?;
            }
            Err(error) => {
                state.tasks[index].status = "paused".to_owned();
                state.tasks[index].error = Some(error.chars().take(1000).collect());
                state.updated_at_unix_ms = now_ms();
                save(state)?;
                return Err(format!(
                    "fila pausada na tarefa {}. Resolva o bloqueio e retome com `dcx loop resume --id {}`",
                    state.tasks[index].id, state.id
                ));
            }
        }
    }
    let task_label = if state.tasks.len() == 1 {
        "tarefa"
    } else {
        "tarefas"
    };
    println!(
        "Fila {} concluída: {} {}. Uso: {}",
        state.id,
        state.tasks.len(),
        task_label,
        state.usage.summary()
    );
    Ok(())
}

fn sync_usage(state: &mut LoopState) {
    if let Some(usage) = metrics::loop_usage() {
        state.usage = LoopUsage::from_metrics(usage);
    }
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.tasks.is_empty() || manifest.tasks.len() > MAX_TASKS {
        return Err(format!("a fila precisa ter entre 1 e {MAX_TASKS} tarefas"));
    }
    let mut ids = std::collections::HashSet::new();
    for task in &manifest.tasks {
        validate_id(&task.id)?;
        if !ids.insert(&task.id) {
            return Err(format!("id de tarefa duplicado: {}", task.id));
        }
        if task.request.trim().is_empty() || task.request.chars().count() > MAX_REQUEST_CHARS {
            return Err(format!(
                "pedido da tarefa {} deve ter entre 1 e {MAX_REQUEST_CHARS} caracteres",
                task.id
            ));
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
    {
        return Err(format!("id inválido: {id}"));
    }
    Ok(())
}

fn save(state: &LoopState) -> Result<(), String> {
    let path = state_path(&state.workspace, &state.id);
    let parent = path.parent().ok_or("estado da fila sem diretório pai")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(".{}.tmp", state.id));
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

fn state_path(workspace: &Path, id: &str) -> PathBuf {
    workspace
        .join(".dcx")
        .join("loops")
        .join(format!("{id}.json"))
}

fn env_usize(name: &str) -> Result<Option<usize>, String> {
    env_value(name, |raw| {
        let value = raw
            .parse::<usize>()
            .map_err(|_| format!("{name} deve ser um inteiro positivo"))?;
        if value == 0 {
            return Err(format!("{name} deve ser maior que zero"));
        }
        Ok(value)
    })
}

fn env_u64(name: &str) -> Result<Option<u64>, String> {
    env_value(name, |raw| {
        let value = raw
            .parse::<u64>()
            .map_err(|_| format!("{name} deve ser um inteiro positivo"))?;
        if value == 0 {
            return Err(format!("{name} deve ser maior que zero"));
        }
        Ok(value)
    })
}

fn env_f64(name: &str) -> Result<Option<f64>, String> {
    env_value(name, |raw| {
        let value = raw
            .parse::<f64>()
            .map_err(|_| format!("{name} deve ser um número positivo"))?;
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("{name} deve ser um número positivo"));
        }
        Ok(value)
    })
}

fn env_present(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

fn env_value<T>(
    name: &str,
    parse: impl FnOnce(&str) -> Result<T, String>,
) -> Result<Option<T>, String> {
    std::env::var(name)
        .ok()
        .map(|value| parse(value.trim()))
        .transpose()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_exhaustion_covers_calls_cost_and_duration() {
        let budget = LoopBudget {
            max_model_calls: Some(3),
            max_cost_usd: Some(1.0),
            max_duration_minutes: Some(2),
        };
        let mut usage = LoopUsage::default();
        assert!(budget.exhausted(&usage).is_none());

        usage.model_calls = 3;
        assert!(
            budget
                .exhausted(&usage)
                .expect("chamadas")
                .contains("3 chamadas")
        );

        usage.model_calls = 0;
        usage.known_cost_usd = 1.0;
        assert!(
            budget
                .exhausted(&usage)
                .expect("custo")
                .contains("US$ 1.0000")
        );

        usage.known_cost_usd = 0.0;
        usage.cost_complete = false;
        assert!(
            budget
                .exhausted(&usage)
                .expect("custo incompleto")
                .contains("não reportou custo")
        );

        usage.cost_complete = true;
        usage.duration_ms = 120_000;
        assert!(
            budget
                .exhausted(&usage)
                .expect("duração")
                .contains("2 minutos")
        );
    }

    #[test]
    fn old_loop_state_loads_with_default_budget_and_usage() {
        let state: LoopState = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "id": "loop-old",
            "workspace": "/tmp/project",
            "created_at_unix_ms": 1,
            "updated_at_unix_ms": 2,
            "current_task": 0,
            "tasks": [{
                "id": "one",
                "request": "Fazer algo",
                "status": "queued",
                "run_id": null,
                "error": null
            }]
        }))
        .expect("estado antigo");

        assert_eq!(state.budget.summary(), "sem limite global");
        assert_eq!(state.usage.model_calls, 0);
        assert!(state.usage.cost_complete);
    }

    #[test]
    fn status_shows_progress_budget_and_pause_reason() {
        let state = LoopState {
            schema_version: LOOP_SCHEMA_VERSION,
            id: "loop-status".to_owned(),
            workspace: PathBuf::from("/tmp/project"),
            created_at_unix_ms: 1,
            updated_at_unix_ms: 2,
            current_task: 1,
            tasks: vec![
                LoopTask {
                    id: "one".to_owned(),
                    request: "Primeira".to_owned(),
                    status: "done".to_owned(),
                    run_id: Some("run-1".to_owned()),
                    error: None,
                },
                LoopTask {
                    id: "two".to_owned(),
                    request: "Segunda".to_owned(),
                    status: "paused".to_owned(),
                    run_id: None,
                    error: Some("aguardando aprovação".to_owned()),
                },
            ],
            budget: LoopBudget {
                max_model_calls: Some(4),
                max_cost_usd: Some(1.0),
                max_duration_minutes: None,
            },
            usage: LoopUsage {
                model_calls: 2,
                known_cost_usd: 0.12,
                cost_complete: true,
                duration_ms: 30_000,
            },
        };

        let text = format_status(&state);
        assert!(text.contains("1/2 tarefas concluídas"));
        assert!(text.contains("até 4 chamadas"));
        assert!(text.contains("2 chamadas · US$ 0.1200 · 30s"));
        assert!(text.contains("[paused] two"));
        assert!(text.contains("aguardando aprovação"));
    }
}
