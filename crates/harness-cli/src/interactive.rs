use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::events::EventLog;
use crate::lock::ProjectLock;
use crate::providers::{self, Analyst};
use harness_artifacts::{
    Answer, ContractBundle, DeliveryReport, Plan, SCHEMA_VERSION, SensorImplementation, SensorLock,
    SensorVisibility, Understanding,
};
use harness_engine::{
    Executor, RunCoordinator, RunExecution, RunLifecycle, RunParticipants, RunRequest, RunState,
    Verifier, check_sensors_fail,
};
use harness_sensors::ScriptSensorRunner;

use crate::workspace;

type InteractiveResult<T> = Result<T, Box<dyn Error>>;

pub fn run(request: &str) -> InteractiveResult<()> {
    if request.trim().is_empty() {
        return Err("O pedido não pode estar vazio.".into());
    }

    let workspace = std::env::current_dir()?;
    let _lock = ProjectLock::acquire(&workspace)?;
    let mut analyst = providers::analyst()?;
    let mut lifecycle = RunLifecycle::new();
    lifecycle.transition(RunState::Understanding)?;
    println!("Entendendo o pedido em {}", workspace.display());

    let understanding =
        collect_understanding(request, &workspace, analyst.as_mut(), &mut lifecycle)?;
    lifecycle.transition(RunState::Contracting)?;
    println!("\nPreparando contrato para aprovação.");

    let (approved_bundle, revisions) = collect_approved_contract(
        request,
        &workspace,
        &understanding,
        analyst.as_mut(),
        &mut lifecycle,
    )?;
    lifecycle.transition(RunState::AcceptanceAuthoring)?;
    let run_id = new_run_id();
    let artifact_dir = save_artifacts(
        &workspace,
        &run_id,
        &understanding,
        &approved_bundle,
        &revisions,
        lifecycle.history(),
    )?;
    println!(
        "\nContrato aprovado. Artefatos salvos em {}",
        artifact_dir.display()
    );
    let mut events = EventLog::create(&artifact_dir, &run_id)?;
    events.append(
        "contract.approved",
        serde_json::json!({
            "criteria": approved_bundle.contract.criteria.len(),
            "sensors": approved_bundle.sensor_suite.sensors.len(),
            "revisions": revisions.len(),
        }),
    )?;

    let (implementation, sensor_dir) =
        author_sensors(&workspace, &approved_bundle, &understanding.locale, &run_id)?;
    save_sensor_manifest(
        &artifact_dir,
        &run_id,
        &approved_bundle,
        &implementation,
        &sensor_dir,
    )?;
    println!("\nSensores de aceite implementados, travados e verificados contra o baseline.");
    println!(
        "Manifesto: {}",
        artifact_dir.join("sensor_manifest.json").display()
    );
    println!(
        "Fontes dos sensores (fora do workspace): {}",
        sensor_dir.display()
    );
    events.append(
        "sensors.locked",
        serde_json::json!({
            "suite_checksum": approved_bundle.sensor_suite.checksum()?,
            "implementation_checksum": implementation.checksum()?,
        }),
    )?;

    lifecycle.transition(RunState::Planning)?;
    let mut planner = providers::planner()?;
    let plan = planner
        .plan(&workspace, &approved_bundle, &understanding.locale)
        .map_err(|error| format!("Planner não conseguiu produzir o plano: {error}"))?;
    plan.validate_for(&approved_bundle.contract, &approved_bundle.sensor_suite)
        .map_err(|error| format!("Plano inválido: {error}"))?;
    fs::write(
        artifact_dir.join("plan.json"),
        serde_json::to_vec_pretty(&plan)?,
    )?;
    println!(
        "\nPlano salvo em {}",
        artifact_dir.join("plan.json").display()
    );
    events.append(
        "plan.created",
        serde_json::json!({
            "steps": plan.steps.iter().map(|step| step.id.clone()).collect::<Vec<_>>(),
            "integration_step_id": plan.integration_step_id,
        }),
    )?;

    let run_workspace = state_root().join("runs").join(&run_id).join("workspace");
    workspace::copy_project(&workspace, &run_workspace, &[state_root()])
        .map_err(|error| format!("Não foi possível preparar a cópia isolada: {error}"))?;
    println!("Executando em cópia isolada: {}", run_workspace.display());

    lifecycle.transition(RunState::Executing)?;
    let mut runner = ScriptSensorRunner::materialize(
        &implementation,
        &approved_bundle.sensor_suite,
        &sensor_dir,
    )
    .map_err(|error| format!("Não foi possível materializar os sensores: {error}"))?
    .sandbox_from_env()?;
    let lock = SensorLock::from_suite(&approved_bundle.sensor_suite)?;
    lock.verify(&approved_bundle.sensor_suite)?;
    let mut executor = providers::executor()?;
    let mut verifier = providers::verifier()?;
    let execution = execute_plan(
        &run_workspace,
        &plan,
        &approved_bundle,
        &mut runner,
        executor.as_mut(),
        verifier.as_mut(),
        &mut events,
    )?;
    save_execution(
        &artifact_dir,
        &run_id,
        &execution,
        approved_bundle.contract.criteria.len(),
    )?;
    println!(
        "\nExecução concluída. Estado: {}",
        execution.states.last().unwrap()
    );
    println!(
        "Relatório: {}",
        artifact_dir.join("delivery_report.json").display()
    );
    if execution.verdict.approved {
        let changed = execution
            .step_reports
            .iter()
            .flat_map(|report| report.files_changed.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let applied = workspace::apply_changes(&workspace, &run_workspace, &changed)
            .map_err(|error| format!("Não foi possível aplicar as mudanças aprovadas: {error}"))?;
        events.append(
            "changes.applied",
            serde_json::json!({"files": applied.len()}),
        )?;
        println!(
            "{} arquivo(s) aplicados ao projeto após a aprovação.",
            applied.len()
        );
        println!("Revise as mudanças no workspace e faça o merge.");
        Ok(())
    } else {
        events.append(
            "run.rejected",
            serde_json::json!({"issues": execution.verdict.issues.len()}),
        )?;
        println!(
            "A cópia com a execução reprovada foi preservada em {}",
            run_workspace.display()
        );
        Err("A execução não foi aprovada; veja o veredito e as issues no relatório.".into())
    }
}

pub fn resume(run_id: &str) -> InteractiveResult<()> {
    if run_id.trim().is_empty() {
        return Err("Informe o id do run para retomar.".into());
    }
    let workspace = std::env::current_dir()?;
    let _lock = ProjectLock::acquire(&workspace)?;
    let artifact_dir = workspace.join(".harness").join("runs").join(run_id);
    if !artifact_dir.is_dir() {
        return Err(format!(
            "Run {} não encontrado em {}.",
            run_id,
            artifact_dir.display()
        )
        .into());
    }
    let bundle: ContractBundle = read_json(&artifact_dir.join("contract_bundle.json"))?;
    let plan: Plan = read_json(&artifact_dir.join("plan.json"))?;
    let implementation: SensorImplementation =
        read_json(&artifact_dir.join("sensor_implementation.json"))?;
    plan.validate_for(&bundle.contract, &bundle.sensor_suite)
        .map_err(|error| format!("Plano inválido: {error}"))?;
    implementation
        .validate_for(&bundle.sensor_suite)
        .map_err(|error| format!("Sensores inválidos: {error}"))?;

    let run_workspace = state_root().join("runs").join(run_id).join("workspace");
    if !run_workspace.is_dir() {
        workspace::copy_project(&workspace, &run_workspace, &[state_root()])
            .map_err(|error| format!("Não foi possível preparar a cópia isolada: {error}"))?;
    }
    let sensor_dir = sensor_directory(run_id);
    let mut runner =
        ScriptSensorRunner::materialize(&implementation, &bundle.sensor_suite, &sensor_dir)
            .map_err(|error| format!("Não foi possível materializar os sensores: {error}"))?
            .sandbox_from_env()?;
    let lock = SensorLock::from_suite(&bundle.sensor_suite)?;
    lock.verify(&bundle.sensor_suite)?;
    let mut executor = providers::executor()?;
    let mut verifier = providers::verifier()?;
    let mut events = EventLog::open(&artifact_dir, run_id)?;
    events.append(
        "run.resumed",
        serde_json::json!({"workspace": run_workspace.display().to_string()}),
    )?;
    println!("Retomando run {} em {}", run_id, run_workspace.display());
    let execution = execute_plan(
        &run_workspace,
        &plan,
        &bundle,
        &mut runner,
        executor.as_mut(),
        verifier.as_mut(),
        &mut events,
    )?;
    save_execution(
        &artifact_dir,
        run_id,
        &execution,
        bundle.contract.criteria.len(),
    )?;
    println!(
        "Execução concluída. Estado: {}",
        execution.states.last().unwrap()
    );
    if execution.verdict.approved {
        let changed = execution
            .step_reports
            .iter()
            .flat_map(|report| report.files_changed.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let applied = workspace::apply_changes(&workspace, &run_workspace, &changed)
            .map_err(|error| format!("Não foi possível aplicar as mudanças aprovadas: {error}"))?;
        events.append(
            "changes.applied",
            serde_json::json!({"files": applied.len()}),
        )?;
        println!("{} arquivo(s) aplicados ao projeto.", applied.len());
        Ok(())
    } else {
        events.append(
            "run.rejected",
            serde_json::json!({"issues": execution.verdict.issues.len()}),
        )?;
        Err("A execução retomada não foi aprovada; veja o relatório.".into())
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> InteractiveResult<T> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Não foi possível ler {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("JSON inválido em {}: {error}", path.display()).into())
}

fn execute_plan(
    workspace: &Path,
    plan: &Plan,
    bundle: &ContractBundle,
    runner: &mut ScriptSensorRunner,
    executor: &mut dyn Executor,
    verifier: &mut dyn Verifier,
    events: &mut EventLog,
) -> InteractiveResult<RunExecution> {
    let coordinator = RunCoordinator::with_verification_limit(3, 3)?.stop_at_merge(true);
    let mut step_reports = Vec::new();
    let mut sensor_results = Vec::new();
    let mut states = Vec::new();
    let mut verdict: Option<harness_artifacts::Verdict> = None;
    for step in plan.ordered_steps()? {
        let contract = bundle.contract.for_criteria(&step.criteria);
        let suite = bundle.sensor_suite.for_step(step);
        println!("Executando passo {}: {}", step.id, step.title);
        let execution = coordinator
            .execute_step(
                RunRequest {
                    contract: &contract,
                    suite: &suite,
                    step_id: &step.id,
                    workspace,
                },
                RunParticipants {
                    executor: &mut *executor,
                    sensor_runner: &mut *runner,
                    verifier: &mut *verifier,
                },
            )
            .map_err(std::io::Error::other)?;
        let attempts = execution.step_reports.len();
        let approved = execution.verdict.approved;
        step_reports.extend(execution.step_reports);
        sensor_results.extend(execution.sensor_results);
        states = execution.states;
        events.append(
            "step.completed",
            serde_json::json!({
                "step_id": step.id,
                "approved": approved,
                "attempts": attempts,
            }),
        )?;
        let step_verdict = execution.verdict;
        match verdict.as_mut() {
            Some(combined) => {
                combined.approved &= step_verdict.approved;
                for criterion in step_verdict.criteria {
                    if let Some(existing) = combined
                        .criteria
                        .iter_mut()
                        .find(|existing| existing.criterion_id == criterion.criterion_id)
                    {
                        *existing = criterion;
                    } else {
                        combined.criteria.push(criterion);
                    }
                }
                for issue in step_verdict.issues {
                    if !combined.issues.iter().any(|existing| {
                        existing.criterion_id == issue.criterion_id
                            && existing.description == issue.description
                    }) {
                        combined.issues.push(issue);
                    }
                }
                for note in step_verdict.notes {
                    if !combined.notes.contains(&note) {
                        combined.notes.push(note);
                    }
                }
            }
            None => verdict = Some(step_verdict),
        }
        if !approved {
            break;
        }
    }
    Ok(RunExecution {
        step_reports,
        sensor_results,
        verdict: verdict.ok_or("Plano sem passos executáveis.")?,
        states,
    })
}

fn save_execution(
    artifact_dir: &Path,
    run_id: &str,
    execution: &RunExecution,
    criteria_total: usize,
) -> InteractiveResult<()> {
    let criteria_passed = execution
        .verdict
        .criteria
        .iter()
        .filter(|criterion| criterion.passed)
        .count();
    let report = DeliveryReport {
        schema_version: SCHEMA_VERSION,
        run_id: run_id.to_owned(),
        summary: if execution.verdict.approved {
            "Implementação aprovada; aguardando revisão e merge.".to_owned()
        } else {
            "Execução reprovada; veja o veredito e as issues.".to_owned()
        },
        criteria_passed,
        criteria_total,
        sensor_results: execution.sensor_results.clone(),
        states: execution
            .states
            .iter()
            .map(|state| state.as_str().to_owned())
            .collect(),
    };
    fs::write(
        artifact_dir.join("step_reports.json"),
        serde_json::to_vec_pretty(&execution.step_reports)?,
    )?;
    fs::write(
        artifact_dir.join("sensor_results.json"),
        serde_json::to_vec_pretty(&execution.sensor_results)?,
    )?;
    fs::write(
        artifact_dir.join("verdict.json"),
        serde_json::to_vec_pretty(&execution.verdict)?,
    )?;
    fs::write(
        artifact_dir.join("delivery_report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    let mut states = fs::read(artifact_dir.join("run.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|run| run["states"].as_array().cloned())
        .map(|states| {
            states
                .into_iter()
                .filter_map(|state| state.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let start = execution
        .states
        .iter()
        .position(|state| *state == RunState::Planning)
        .unwrap_or(0);
    states.extend(
        execution.states[start..]
            .iter()
            .map(|state| state.as_str().to_owned()),
    );
    fs::write(
        artifact_dir.join("run.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "states": states,
        }))?,
    )?;
    Ok(())
}

fn author_sensors(
    workspace: &Path,
    bundle: &ContractBundle,
    locale: &str,
    run_id: &str,
) -> InteractiveResult<(SensorImplementation, PathBuf)> {
    let mut author = providers::acceptance_author()?;
    let mut previous: Option<SensorImplementation> = None;
    let mut feedback: Option<String> = None;
    let directory = sensor_directory(run_id);
    for attempt in 1..=3 {
        let implementation = author
            .implement(
                workspace,
                bundle,
                locale,
                previous.as_ref(),
                feedback.as_deref(),
            )
            .map_err(|error| {
                format!("Acceptance author não conseguiu implementar sensores: {error}")
            })?;
        if let Err(error) = implementation.validate_for(&bundle.sensor_suite) {
            println!("Sensores fora das regras ({error}); solicitando correção.");
            feedback = Some(format!("Implementação inválida: {error}"));
            previous = Some(implementation);
            continue;
        }
        let _ = fs::remove_dir_all(&directory);
        let mut runner =
            ScriptSensorRunner::materialize(&implementation, &bundle.sensor_suite, &directory)
                .map_err(|error| format!("Não foi possível materializar os sensores: {error}"))?;
        match check_sensors_fail(&bundle.sensor_suite, &mut runner, workspace) {
            Ok(results) => {
                println!(
                    "Tentativa {attempt}: {} sensores falharam no baseline como esperado.",
                    results.len()
                );
                return Ok((implementation, directory));
            }
            Err(error) => {
                println!("Sensores não passaram na checagem de baseline; solicitando correção.");
                feedback = Some(error);
                previous = Some(implementation);
            }
        }
    }
    Err("Acceptance author não produziu sensores válidos após 3 tentativas.".into())
}

fn save_sensor_manifest(
    artifact_dir: &Path,
    run_id: &str,
    bundle: &ContractBundle,
    implementation: &SensorImplementation,
    sensor_dir: &Path,
) -> InteractiveResult<()> {
    let lock = SensorLock::from_suite(&bundle.sensor_suite)?;
    let scripts = implementation
        .scripts
        .iter()
        .map(|script| {
            Ok(serde_json::json!({
                "sensor_id": script.sensor_id,
                "filename": script.filename,
                "checksum": script.checksum()?,
            }))
        })
        .collect::<Result<Vec<_>, harness_artifacts::ValidationError>>()?;
    let manifest = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "suite_checksum": bundle.sensor_suite.checksum()?,
        "implementation_checksum": implementation.checksum()?,
        "sensor_directory": sensor_dir.display().to_string(),
        "scripts": scripts,
    });
    fs::write(
        artifact_dir.join("sensor_manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    fs::write(
        artifact_dir.join("sensor_lock.json"),
        serde_json::to_vec_pretty(&lock)?,
    )?;
    fs::write(
        artifact_dir.join("sensor_implementation.json"),
        serde_json::to_vec_pretty(implementation)?,
    )?;
    Ok(())
}

fn new_run_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("run-{}-{timestamp}", std::process::id())
}

pub(crate) fn state_root() -> PathBuf {
    if let Some(directory) = std::env::var_os("HARNESS_STATE_DIR") {
        return PathBuf::from(directory);
    }
    #[cfg(unix)]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/state/harness");
    }
    std::env::temp_dir().join("harness-state")
}

fn sensor_directory(run_id: &str) -> PathBuf {
    state_root().join("runs").join(run_id).join("sensors")
}

fn collect_understanding(
    request: &str,
    workspace: &Path,
    analyst: &mut dyn Analyst,
    lifecycle: &mut RunLifecycle,
) -> InteractiveResult<Understanding> {
    let mut previous: Option<Understanding> = None;
    loop {
        let mut understanding = analyst
            .understand(request, workspace, previous.as_ref())
            .map_err(|error| format!("Analyst não conseguiu produzir entendimento: {error}"))?;
        preserve_question_history(&mut understanding, previous.as_ref());
        understanding
            .validate()
            .map_err(|error| format!("Understanding inválido: {error}"))?;

        let unanswered = understanding
            .unanswered_blocking_questions()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if unanswered.is_empty() {
            return Ok(understanding);
        }

        lifecycle.transition(RunState::AwaitingHuman)?;
        for question in unanswered {
            println!("\nPergunta bloqueante: {}", question.question);
            println!("Por que isso muda o resultado: {}", question.why_blocking);
            for (index, option) in question.options.iter().enumerate() {
                println!("  {}. {}", index + 1, option);
            }
            let answer = read_line("Sua resposta: ")?;
            if answer.trim().is_empty() {
                return Err("A resposta não pode ficar vazia.".into());
            }
            understanding.answers.push(Answer {
                question_id: question.id,
                answer,
                answered_by: "user".to_owned(),
            });
        }
        understanding
            .validate()
            .map_err(|error| format!("Understanding inválido após as respostas: {error}"))?;
        previous = Some(understanding);
        lifecycle.transition(RunState::Understanding)?;
    }
}

fn preserve_question_history(current: &mut Understanding, previous: Option<&Understanding>) {
    let Some(previous) = previous else {
        return;
    };
    for question in &previous.blocking_questions {
        if let Some(existing) = current
            .blocking_questions
            .iter_mut()
            .find(|candidate| candidate.id == question.id)
        {
            *existing = question.clone();
        } else {
            current.blocking_questions.push(question.clone());
        }
    }
    for answer in &previous.answers {
        if let Some(existing) = current
            .answers
            .iter_mut()
            .find(|candidate| candidate.question_id == answer.question_id)
        {
            *existing = answer.clone();
        } else {
            current.answers.push(answer.clone());
        }
    }
}

fn collect_approved_contract(
    request: &str,
    workspace: &Path,
    understanding: &Understanding,
    analyst: &mut dyn Analyst,
    lifecycle: &mut RunLifecycle,
) -> InteractiveResult<(ContractBundle, Vec<ContractRevision>)> {
    let mut previous: Option<ContractBundle> = None;
    let mut last_valid: Option<ContractBundle> = None;
    let mut rejection_feedback: Option<String> = None;
    let mut validation_feedback: Option<String> = None;
    let mut revisions = Vec::new();
    loop {
        let bundle = analyst
            .contract(
                request,
                workspace,
                understanding,
                previous.as_ref(),
                rejection_feedback.as_deref(),
                validation_feedback.as_deref(),
            )
            .map_err(|error| format!("Analyst não conseguiu produzir contrato: {error}"))?;
        let validation = bundle.validate().and_then(|()| match last_valid.as_ref() {
            Some(last_valid) => bundle.validate_revision_from(last_valid),
            None => Ok(()),
        });
        if let Err(error) = validation {
            validation_feedback = Some(error.to_string());
            previous = Some(bundle);
            println!("Contrato fora das regras ({error}); solicitando correção.");
            continue;
        }
        validation_feedback = None;
        last_valid = Some(bundle.clone());
        revisions.push(ContractRevision {
            version: revisions.len() as u32 + 1,
            bundle: bundle.clone(),
            revision_reason: rejection_feedback.clone(),
        });
        show_contract(&bundle);

        lifecycle.transition(RunState::AwaitingHuman)?;
        let decision = read_line("Aprovar contrato? [s]im / [r]ejeitar / [c]ancelar: ")?;
        match decision.trim().to_lowercase().as_str() {
            "s" | "sim" | "y" | "yes" | "aprovar" => {
                return Ok((bundle, revisions));
            }
            "c" | "cancelar" | "cancel" => {
                lifecycle.transition(RunState::Cancelled)?;
                return Err("Execução cancelada pelo usuário.".into());
            }
            _ => {
                let feedback = read_line("O que precisa mudar no contrato? ")?;
                if feedback.trim().is_empty() {
                    return Err("Informe o motivo da rejeição para refazer o contrato.".into());
                }
                lifecycle.transition(RunState::Contracting)?;
                rejection_feedback = Some(feedback);
                previous = Some(bundle);
            }
        }
    }
}

fn show_contract(bundle: &ContractBundle) {
    println!("\nObjetivo: {}", bundle.contract.objective);
    println!("Critérios:");
    for criterion in &bundle.contract.criteria {
        println!("  - [{}] {}", criterion.id, criterion.description);
    }
    println!("Interface: {}", bundle.contract.interface);
    println!("Restrições:");
    for constraint in &bundle.contract.constraints {
        println!("  - {constraint}");
    }
    println!("Fora de escopo:");
    for item in &bundle.contract.out_of_scope {
        println!("  - {item}");
    }
    println!("Sensores de aceite:");
    for sensor in &bundle.sensor_suite.sensors {
        let visibility = match sensor.visibility {
            SensorVisibility::Visible => "visible",
            SensorVisibility::Hidden => "hidden",
        };
        println!(
            "  - [{} / {}] {}: {}",
            visibility, sensor.criterion_id, sensor.id, sensor.check
        );
    }
}

fn save_artifacts(
    workspace: &Path,
    run_id: &str,
    understanding: &Understanding,
    bundle: &ContractBundle,
    revisions: &[ContractRevision],
    states: &[RunState],
) -> InteractiveResult<PathBuf> {
    let directory = workspace.join(".harness").join("runs").join(run_id);
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join("understanding.json"),
        serde_json::to_vec_pretty(understanding)?,
    )?;
    fs::write(
        directory.join("contract.json"),
        serde_json::to_vec_pretty(&bundle.contract)?,
    )?;
    fs::write(
        directory.join("sensor_suite.json"),
        serde_json::to_vec_pretty(&bundle.sensor_suite)?,
    )?;
    fs::write(
        directory.join("contract_bundle.json"),
        serde_json::to_vec_pretty(bundle)?,
    )?;
    let revision_dir = directory.join("contract_revisions");
    fs::create_dir_all(&revision_dir)?;
    for revision in revisions {
        fs::write(
            revision_dir.join(format!("contract-v{}.json", revision.version)),
            serde_json::to_vec_pretty(&revision.bundle.contract)?,
        )?;
        fs::write(
            revision_dir.join(format!("sensor_suite-v{}.json", revision.version)),
            serde_json::to_vec_pretty(&revision.bundle.sensor_suite)?,
        )?;
    }
    fs::write(
        directory.join("contract_revisions.json"),
        serde_json::to_vec_pretty(revisions)?,
    )?;
    fs::write(
        directory.join("run.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "states": states.iter().map(|state| state.as_str()).collect::<Vec<_>>(),
        }))?,
    )?;
    Ok(directory)
}

#[derive(serde::Serialize)]
struct ContractRevision {
    version: u32,
    bundle: ContractBundle,
    revision_reason: Option<String>,
}

fn read_line(prompt: &str) -> InteractiveResult<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    if value.is_empty() {
        return Err("Entrada encerrada antes de concluir a etapa.".into());
    }
    Ok(value.trim().to_owned())
}
