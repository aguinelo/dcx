use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::events::EventLog;
use crate::lock::ProjectLock;
use crate::metrics;
use crate::providers::{self, Analyst};
use harness_artifacts::{
    Answer, ContractBundle, DeliveryReport, DisputeDecision, Plan, SCHEMA_VERSION, SensorDispute,
    SensorImplementation, SensorLock, SensorVisibility, Understanding, resolve_sensor_dispute,
};
use harness_engine::{
    RunCoordinator, RunExecution, RunLifecycle, RunParticipants, RunRequest, RunState,
    check_sensors_fail,
};
use harness_sensors::ScriptSensorRunner;

use crate::workspace;

type InteractiveResult<T> = Result<T, Box<dyn Error>>;

pub fn run(request: &str) -> InteractiveResult<()> {
    if request.trim().is_empty() {
        return Err("O pedido não pode estar vazio.".into());
    }
    metrics::reset();

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
    let lock = SensorLock::from_suite(&approved_bundle.sensor_suite)?;
    lock.verify(&approved_bundle.sensor_suite)?;
    let run_root = run_workspace
        .parent()
        .ok_or("Cópia isolada sem diretório pai.")?;
    let step_executor = ProviderStepExecutor {
        implementation: &implementation,
        sensor_dir: &sensor_dir,
    };
    let execution = execute_plan(PlanExecution {
        base: &run_workspace,
        run_root,
        plan: &plan,
        bundle: &approved_bundle,
        skip: &BTreeSet::new(),
        executor: &step_executor,
        events: &mut events,
    })?;
    save_execution(
        &artifact_dir,
        &run_id,
        &execution,
        approved_bundle.contract.criteria.len(),
    )?;
    metrics::write(&artifact_dir.join("metrics.json"))?;
    println!(
        "\nExecução concluída. Estado: {}",
        execution.states.last().unwrap()
    );
    println!(
        "Relatório: {}",
        artifact_dir.join("delivery_report.json").display()
    );
    if execution.verdict.approved {
        let changed = events.completed_files().into_iter().collect::<Vec<_>>();
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
    validate_run_id(run_id)?;
    metrics::reset();
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
    let workspace_existed = run_workspace.is_dir();
    if !workspace_existed {
        workspace::copy_project(&workspace, &run_workspace, &[state_root()])
            .map_err(|error| format!("Não foi possível preparar a cópia isolada: {error}"))?;
    }
    let sensor_dir = sensor_directory(run_id);
    let lock = SensorLock::from_suite(&bundle.sensor_suite)?;
    lock.verify(&bundle.sensor_suite)?;
    let mut events = EventLog::open(&artifact_dir, run_id)?;
    events.append(
        "run.resumed",
        serde_json::json!({"workspace": run_workspace.display().to_string()}),
    )?;
    let mut skip = if workspace_existed {
        events.completed_steps()
    } else {
        BTreeSet::new()
    };
    skip.remove(&plan.integration_step_id);
    println!(
        "Retomando run {} em {} ({} passo(s) concluído(s) serão pulados)",
        run_id,
        run_workspace.display(),
        skip.len()
    );
    let run_root = run_workspace
        .parent()
        .ok_or("Cópia isolada sem diretório pai.")?;
    let step_executor = ProviderStepExecutor {
        implementation: &implementation,
        sensor_dir: &sensor_dir,
    };
    let execution = execute_plan(PlanExecution {
        base: &run_workspace,
        run_root,
        plan: &plan,
        bundle: &bundle,
        skip: &skip,
        executor: &step_executor,
        events: &mut events,
    })?;
    save_execution(
        &artifact_dir,
        run_id,
        &execution,
        bundle.contract.criteria.len(),
    )?;
    metrics::write(&artifact_dir.join("metrics.json"))?;
    println!(
        "Execução concluída. Estado: {}",
        execution.states.last().unwrap()
    );
    if execution.verdict.approved {
        let changed = events.completed_files().into_iter().collect::<Vec<_>>();
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

pub fn dispute(run_id: &str, sensor_id: &str, reason: &str) -> InteractiveResult<()> {
    validate_run_id(run_id)?;
    metrics::reset();
    if sensor_id.trim().is_empty() || reason.trim().is_empty() {
        return Err("Informe run, sensor e motivo da disputa.".into());
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
    let mut bundle: ContractBundle = read_json(&artifact_dir.join("contract_bundle.json"))?;
    let mut implementation: SensorImplementation =
        read_json(&artifact_dir.join("sensor_implementation.json"))?;
    let understanding: Understanding = read_json(&artifact_dir.join("understanding.json"))?;
    let lock: SensorLock = read_json(&artifact_dir.join("sensor_lock.json"))?;
    lock.verify(&bundle.sensor_suite)?;
    let sensor = bundle
        .sensor_suite
        .sensors
        .iter()
        .find(|sensor| sensor.id == sensor_id)
        .cloned()
        .ok_or_else(|| format!("Sensor {} não pertence ao run {}.", sensor_id, run_id))?;

    println!("Disputa do sensor {}: {}", sensor.id, reason.trim());
    println!("Check atual: {}", sensor.check);
    let answer = read_line("Aceitar a disputa e reescrever apenas este sensor? [s/N]: ")?;
    let accepted = matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "sim" | "y" | "yes"
    );
    let decision = DisputeDecision {
        accepted,
        replacement: accepted.then(|| sensor.clone()),
        note: reason.trim().to_owned(),
    };
    let dispute = SensorDispute {
        sensor_id: sensor.id.clone(),
        reason: reason.trim().to_owned(),
        requested_by: "executor".to_owned(),
    };
    let updated_suite = resolve_sensor_dispute(&bundle.sensor_suite, &dispute, &decision)?;
    let mut events = EventLog::open(&artifact_dir, run_id)?;
    fs::write(
        artifact_dir.join("sensor_dispute.json"),
        serde_json::to_vec_pretty(&dispute)?,
    )?;
    if !accepted {
        fs::write(
            artifact_dir.join("dispute_decision.json"),
            serde_json::to_vec_pretty(&decision)?,
        )?;
        events.append(
            "sensor.dispute_rejected",
            serde_json::json!({"sensor_id": sensor.id, "reason": reason.trim()}),
        )?;
        metrics::write(&artifact_dir.join("metrics.json"))?;
        println!("Disputa rejeitada; a suíte e a implementação não foram alteradas.");
        return Ok(());
    }

    let script = reauthor_disputed_sensor(
        &workspace,
        &bundle,
        &implementation,
        &sensor,
        &understanding.locale,
        reason,
        run_id,
    )?;
    let index = implementation
        .scripts
        .iter()
        .position(|candidate| candidate.sensor_id == sensor.id)
        .ok_or_else(|| format!("Sensor {} não tem script implementado.", sensor.id))?;
    implementation.scripts[index] = script;
    implementation.validate_for(&updated_suite)?;
    bundle.sensor_suite = updated_suite;
    let sensor_dir = sensor_directory(run_id);
    let _ = fs::remove_dir_all(&sensor_dir);
    ScriptSensorRunner::materialize(&implementation, &bundle.sensor_suite, &sensor_dir)?;
    fs::write(
        artifact_dir.join("contract_bundle.json"),
        serde_json::to_vec_pretty(&bundle)?,
    )?;
    fs::write(
        artifact_dir.join("sensor_suite.json"),
        serde_json::to_vec_pretty(&bundle.sensor_suite)?,
    )?;
    save_sensor_manifest(&artifact_dir, run_id, &bundle, &implementation, &sensor_dir)?;
    fs::write(
        artifact_dir.join("dispute_decision.json"),
        serde_json::to_vec_pretty(&decision)?,
    )?;
    events.append(
        "sensor.dispute_accepted",
        serde_json::json!({
            "sensor_id": sensor.id,
            "reason": reason.trim(),
            "implementation_checksum": implementation.checksum()?,
        }),
    )?;
    metrics::write(&artifact_dir.join("metrics.json"))?;
    println!(
        "Sensor {} reescrito. Rode `harness resume --run {}` para reexecutar o plano.",
        sensor.id, run_id
    );
    Ok(())
}

fn reauthor_disputed_sensor(
    workspace: &Path,
    bundle: &ContractBundle,
    implementation: &SensorImplementation,
    sensor: &harness_artifacts::SensorSpec,
    locale: &str,
    reason: &str,
    run_id: &str,
) -> InteractiveResult<harness_artifacts::SensorScript> {
    let suite = harness_artifacts::SensorSuite {
        schema_version: SCHEMA_VERSION,
        sensors: vec![sensor.clone()],
    };
    let scoped_bundle = ContractBundle {
        schema_version: SCHEMA_VERSION,
        contract: bundle
            .contract
            .for_criteria(std::slice::from_ref(&sensor.criterion_id)),
        sensor_suite: suite.clone(),
    };
    let mut previous = implementation
        .scripts
        .iter()
        .find(|script| script.sensor_id == sensor.id)
        .map(|script| SensorImplementation {
            schema_version: SCHEMA_VERSION,
            scripts: vec![script.clone()],
        });
    let mut feedback = Some(format!(
        "Reescreva somente o sensor {}. Motivo da disputa: {}",
        sensor.id,
        reason.trim()
    ));
    let mut author = providers::acceptance_author()?;
    let root = state_root().join("disputes").join(run_id);
    let baseline = root.join("baseline");
    let materialized = root.join("sensors");
    let result = (|| {
        for attempt in 1..=3 {
            let candidate = author
                .implement(
                    workspace,
                    &scoped_bundle,
                    locale,
                    previous.as_ref(),
                    feedback.as_deref(),
                )
                .map_err(|error| format!("Acceptance author não conseguiu reescrever: {error}"))?;
            if let Err(error) = candidate.validate_for(&suite) {
                feedback = Some(format!("Implementação inválida: {error}"));
                previous = Some(candidate);
                continue;
            }
            let _ = fs::remove_dir_all(&baseline);
            fs::create_dir_all(&baseline)?;
            let _ = fs::remove_dir_all(&materialized);
            let mut runner = ScriptSensorRunner::materialize(&candidate, &suite, &materialized)
                .map_err(|error| format!("Não foi possível materializar o sensor: {error}"))?;
            match check_sensors_fail(&suite, &mut runner, &baseline) {
                Ok(results) => {
                    println!(
                        "Tentativa {attempt}: {} sensor(es) falharam no baseline.",
                        results.len()
                    );
                    return candidate
                        .scripts
                        .into_iter()
                        .find(|script| script.sensor_id == sensor.id)
                        .ok_or_else(|| {
                            format!("Author não devolveu script para {}.", sensor.id).into()
                        });
                }
                Err(error) => {
                    feedback = Some(error);
                    previous = Some(candidate);
                }
            }
        }
        Err("Acceptance author não produziu um sensor válido após 3 tentativas.".into())
    })();
    let _ = fs::remove_dir_all(root);
    result
}

fn validate_run_id(run_id: &str) -> InteractiveResult<()> {
    if run_id.trim().is_empty()
        || run_id.contains('/')
        || run_id.contains('\\')
        || run_id == "."
        || run_id.contains("..")
    {
        return Err("Id de run inválido.".into());
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> InteractiveResult<T> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Não foi possível ler {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("JSON inválido em {}: {error}", path.display()).into())
}

struct PlanExecution<'a> {
    base: &'a Path,
    run_root: &'a Path,
    plan: &'a Plan,
    bundle: &'a ContractBundle,
    skip: &'a BTreeSet<String>,
    executor: &'a dyn StepExecutor,
    events: &'a mut EventLog,
}

trait StepExecutor: Send + Sync {
    fn execute(
        &self,
        step: &harness_artifacts::PlanStep,
        contract: &harness_artifacts::Contract,
        suite: &harness_artifacts::SensorSuite,
        workspace: &Path,
    ) -> Result<RunExecution, String>;
}

struct ProviderStepExecutor<'a> {
    implementation: &'a SensorImplementation,
    sensor_dir: &'a Path,
}

impl StepExecutor for ProviderStepExecutor<'_> {
    fn execute(
        &self,
        step: &harness_artifacts::PlanStep,
        contract: &harness_artifacts::Contract,
        suite: &harness_artifacts::SensorSuite,
        workspace: &Path,
    ) -> Result<RunExecution, String> {
        let step_sensor_dir = self.sensor_dir.join("steps").join(&step.id);
        let mut runner =
            ScriptSensorRunner::materialize(self.implementation, suite, &step_sensor_dir)?
                .sandbox_from_env()?;
        let mut executor = providers::executor()?;
        let mut verifier = providers::verifier()?;
        RunCoordinator::with_verification_limit(3, 3)?
            .stop_at_merge(true)
            .execute_step(
                RunRequest {
                    contract,
                    suite,
                    step_id: &step.id,
                    workspace,
                },
                RunParticipants {
                    executor: executor.as_mut(),
                    sensor_runner: &mut runner,
                    verifier: verifier.as_mut(),
                },
            )
            .map_err(|error| error.to_string())
    }
}

struct StepOutcome {
    step_id: String,
    execution: RunExecution,
    workspace: PathBuf,
}

fn execute_plan(context: PlanExecution<'_>) -> InteractiveResult<RunExecution> {
    let PlanExecution {
        base,
        run_root,
        plan,
        bundle,
        skip,
        executor,
        events,
    } = context;
    let groups = plan.execution_groups()?;
    let mut step_reports = Vec::new();
    let mut sensor_results = Vec::new();
    let mut states = Vec::new();
    let mut verdict: Option<harness_artifacts::Verdict> = None;
    let mut merged_paths = std::collections::BTreeMap::<String, String>::new();

    for step in &plan.steps {
        if skip.contains(&step.id) {
            merge_verdict(
                &mut verdict,
                harness_artifacts::Verdict {
                    schema_version: SCHEMA_VERSION,
                    approved: true,
                    criteria: step
                        .criteria
                        .iter()
                        .map(|criterion| harness_artifacts::CriterionVerdict {
                            criterion_id: criterion.clone(),
                            passed: true,
                            evidence: vec!["concluído em execução anterior".into()],
                        })
                        .collect(),
                    issues: vec![],
                    notes: vec![format!("Passo {} retomado sem reexecução.", step.id)],
                },
            );
        }
    }

    for group in groups {
        let active = group
            .iter()
            .copied()
            .filter(|step| !skip.contains(&step.id))
            .collect::<Vec<_>>();
        if active.is_empty() {
            continue;
        }
        for step in &active {
            println!("Executando passo {}: {}", step.id, step.title);
        }
        let mut outcomes = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for step in &active {
                let step = *step;
                handles.push(scope.spawn(move || run_step(executor, base, run_root, bundle, step)));
            }
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| "um passo entrou em pânico".to_owned())?
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .map_err(|error| format!("Falha ao executar passos: {error}"))?;
        outcomes.sort_by_key(|outcome| {
            active
                .iter()
                .position(|step| step.id == outcome.step_id)
                .unwrap_or(usize::MAX)
        });

        let group_failed = outcomes
            .iter()
            .any(|outcome| !outcome.execution.verdict.approved);
        if group_failed {
            for outcome in &outcomes {
                events.append(
                    "step.completed",
                    serde_json::json!({
                        "step_id": outcome.step_id,
                        "approved": outcome.execution.verdict.approved,
                        "merged": false,
                        "attempts": outcome.execution.step_reports.len(),
                    }),
                )?;
            }
        } else {
            for outcome in &outcomes {
                let changed = changed_paths(&outcome.execution);
                for path in &changed {
                    if let Some(owner) = merged_paths.get(path) {
                        return Err(format!(
                            "passos {} e {} alteraram o mesmo arquivo: {}",
                            owner, outcome.step_id, path
                        )
                        .into());
                    }
                    merged_paths.insert(path.clone(), outcome.step_id.clone());
                }
                workspace::apply_changes(base, &outcome.workspace, &changed).map_err(|error| {
                    format!(
                        "Não foi possível integrar o passo {}: {error}",
                        outcome.step_id
                    )
                })?;
                events.append(
                    "step.completed",
                    serde_json::json!({
                        "step_id": outcome.step_id,
                        "approved": true,
                        "merged": true,
                        "attempts": outcome.execution.step_reports.len(),
                        "files": changed,
                    }),
                )?;
            }
        }
        for outcome in outcomes {
            step_reports.extend(outcome.execution.step_reports);
            sensor_results.extend(outcome.execution.sensor_results);
            states = outcome.execution.states;
            merge_verdict(&mut verdict, outcome.execution.verdict);
        }
        if group_failed {
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

fn run_step(
    executor: &dyn StepExecutor,
    base: &Path,
    run_root: &Path,
    bundle: &ContractBundle,
    step: &harness_artifacts::PlanStep,
) -> Result<StepOutcome, String> {
    let step_workspace = run_root.join("steps").join(&step.id);
    if step_workspace.exists() {
        fs::remove_dir_all(&step_workspace).map_err(|error| error.to_string())?;
    }
    workspace::copy_project(base, &step_workspace, &[])?;
    let contract = bundle.contract.for_criteria(&step.criteria);
    let suite = bundle.sensor_suite.for_step(step);
    let execution = executor.execute(step, &contract, &suite, &step_workspace)?;
    Ok(StepOutcome {
        step_id: step.id.clone(),
        execution,
        workspace: step_workspace,
    })
}

fn changed_paths(execution: &RunExecution) -> Vec<String> {
    execution
        .step_reports
        .iter()
        .flat_map(|report| report.files_changed.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn merge_verdict(
    combined: &mut Option<harness_artifacts::Verdict>,
    step_verdict: harness_artifacts::Verdict,
) {
    match combined.as_mut() {
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
        None => *combined = Some(step_verdict),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use harness_artifacts::{
        Contract, Criterion, CriterionVerdict, PlanStep, SCHEMA_VERSION, SensorKind, SensorSpec,
        SensorSuite, SensorType, SensorVisibility, StepReport, Verdict,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct FakeStepExecutor {
        calls: AtomicUsize,
        active: AtomicUsize,
        max_active: AtomicUsize,
        conflict: bool,
    }

    impl FakeStepExecutor {
        fn new(conflict: bool) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                conflict,
            }
        }
    }

    impl StepExecutor for FakeStepExecutor {
        fn execute(
            &self,
            step: &PlanStep,
            contract: &Contract,
            _suite: &SensorSuite,
            workspace: &Path,
        ) -> Result<RunExecution, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            self.active.fetch_sub(1, Ordering::SeqCst);
            let filename = if self.conflict {
                "same.txt".to_owned()
            } else {
                format!("{}.txt", step.id)
            };
            fs::write(workspace.join(&filename), step.id.as_bytes())
                .map_err(|error| error.to_string())?;
            let verdict = Verdict {
                schema_version: SCHEMA_VERSION,
                approved: true,
                criteria: contract
                    .criteria
                    .iter()
                    .map(|criterion| CriterionVerdict {
                        criterion_id: criterion.id.clone(),
                        passed: true,
                        evidence: vec!["fake".into()],
                    })
                    .collect(),
                issues: vec![],
                notes: vec![],
            };
            Ok(RunExecution {
                step_reports: vec![StepReport {
                    schema_version: SCHEMA_VERSION,
                    step_id: step.id.clone(),
                    attempts: 1,
                    summary: "fake".into(),
                    files_changed: vec![filename],
                    decisions: vec![],
                    disputes: vec![],
                }],
                sensor_results: vec![],
                verdict,
                states: vec![RunState::AwaitingMerge],
            })
        }
    }

    fn test_bundle() -> ContractBundle {
        let criteria = vec![
            Criterion {
                id: "one".into(),
                description: "primeiro".into(),
            },
            Criterion {
                id: "two".into(),
                description: "segundo".into(),
            },
        ];
        let sensors = criteria
            .iter()
            .flat_map(|criterion| {
                ["visible", "hidden"].map(|visibility| SensorSpec {
                    id: format!("{}-{}", criterion.id, visibility),
                    criterion_id: criterion.id.clone(),
                    sensor_type: SensorType::Test,
                    kind: SensorKind::Computational,
                    visibility: if visibility == "visible" {
                        SensorVisibility::Visible
                    } else {
                        SensorVisibility::Hidden
                    },
                    purpose: "acceptance".into(),
                    check: "check".into(),
                })
            })
            .collect();
        ContractBundle {
            schema_version: SCHEMA_VERSION,
            contract: Contract {
                schema_version: SCHEMA_VERSION,
                objective: "objetivo".into(),
                assumptions: vec![],
                criteria,
                interface: serde_json::Value::Null,
                constraints: vec![],
                out_of_scope: vec![],
            },
            sensor_suite: SensorSuite {
                schema_version: SCHEMA_VERSION,
                sensors,
            },
        }
    }

    fn test_plan() -> Plan {
        let step = |id: &str, criterion: &str| PlanStep {
            id: id.into(),
            title: id.into(),
            description: id.into(),
            criteria: vec![criterion.into()],
            sensors: vec![
                format!("{criterion}-visible"),
                format!("{criterion}-hidden"),
            ],
            depends_on: vec![],
            files_hint: vec![],
        };
        Plan {
            schema_version: SCHEMA_VERSION,
            contract_version: SCHEMA_VERSION,
            steps: vec![step("one", "one"), step("two", "two")],
            integration_step_id: "two".into(),
            rationale: "teste".into(),
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "harness-parallel-{}-{nanos}-{label}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[test]
    fn independent_steps_run_in_parallel_and_merge_changes() {
        let base = temp_dir("base");
        let run_root = temp_dir("run");
        let event_dir = temp_dir("events");
        fs::write(base.join("base.txt"), "base").expect("base");
        let bundle = test_bundle();
        let plan = test_plan();
        let executor = FakeStepExecutor::new(false);
        let mut events = EventLog::create(&event_dir, "run").expect("events");

        let execution = execute_plan(PlanExecution {
            base: &base,
            run_root: &run_root,
            plan: &plan,
            bundle: &bundle,
            skip: &BTreeSet::new(),
            executor: &executor,
            events: &mut events,
        })
        .expect("execução paralela");

        assert!(execution.verdict.approved);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
        assert!(executor.max_active.load(Ordering::SeqCst) >= 2);
        assert!(base.join("one.txt").is_file());
        assert!(base.join("two.txt").is_file());
        let events = fs::read_to_string(event_dir.join("events.jsonl")).expect("eventos");
        assert_eq!(events.lines().count(), 2);
        let _ = fs::remove_dir_all(base);
        let _ = fs::remove_dir_all(run_root);
        let _ = fs::remove_dir_all(event_dir);
    }

    #[test]
    fn skipped_steps_are_not_executed() {
        let base = temp_dir("skip-base");
        let run_root = temp_dir("skip-run");
        let event_dir = temp_dir("skip-events");
        let bundle = test_bundle();
        let plan = test_plan();
        let executor = FakeStepExecutor::new(false);
        let mut events = EventLog::create(&event_dir, "run").expect("events");
        let skip = BTreeSet::from(["one".to_owned()]);

        let execution = execute_plan(PlanExecution {
            base: &base,
            run_root: &run_root,
            plan: &plan,
            bundle: &bundle,
            skip: &skip,
            executor: &executor,
            events: &mut events,
        })
        .expect("execução com skip");

        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        assert!(
            execution
                .verdict
                .criteria
                .iter()
                .any(|criterion| criterion.criterion_id == "one" && criterion.passed)
        );
        assert!(!base.join("one.txt").exists());
        assert!(base.join("two.txt").is_file());
        let _ = fs::remove_dir_all(base);
        let _ = fs::remove_dir_all(run_root);
        let _ = fs::remove_dir_all(event_dir);
    }

    #[test]
    fn completed_files_accumulate_across_resume() {
        let base = temp_dir("resume-base");
        let run_root = temp_dir("resume-run");
        let event_dir = temp_dir("resume-events");
        let bundle = test_bundle();
        let plan = test_plan();
        let executor = FakeStepExecutor::new(false);
        let mut events = EventLog::create(&event_dir, "run").expect("events");
        execute_plan(PlanExecution {
            base: &base,
            run_root: &run_root,
            plan: &plan,
            bundle: &bundle,
            skip: &BTreeSet::new(),
            executor: &executor,
            events: &mut events,
        })
        .expect("primeira execução");

        let mut resumed = EventLog::open(&event_dir, "run").expect("reabrir eventos");
        let skip = BTreeSet::from(["one".to_owned()]);
        execute_plan(PlanExecution {
            base: &base,
            run_root: &run_root,
            plan: &plan,
            bundle: &bundle,
            skip: &skip,
            executor: &executor,
            events: &mut resumed,
        })
        .expect("retomada");

        let files = resumed.completed_files();
        assert!(files.contains("one.txt"));
        assert!(files.contains("two.txt"));
        let _ = fs::remove_dir_all(base);
        let _ = fs::remove_dir_all(run_root);
        let _ = fs::remove_dir_all(event_dir);
    }

    #[test]
    fn parallel_steps_cannot_change_the_same_file() {
        let base = temp_dir("conflict-base");
        let run_root = temp_dir("conflict-run");
        let event_dir = temp_dir("conflict-events");
        let bundle = test_bundle();
        let plan = test_plan();
        let executor = FakeStepExecutor::new(true);
        let mut events = EventLog::create(&event_dir, "run").expect("events");

        let error = execute_plan(PlanExecution {
            base: &base,
            run_root: &run_root,
            plan: &plan,
            bundle: &bundle,
            skip: &BTreeSet::new(),
            executor: &executor,
            events: &mut events,
        })
        .expect_err("conflito deve falhar");

        assert!(error.to_string().contains("mesmo arquivo"));
        let _ = fs::remove_dir_all(base);
        let _ = fs::remove_dir_all(run_root);
        let _ = fs::remove_dir_all(event_dir);
    }
}
