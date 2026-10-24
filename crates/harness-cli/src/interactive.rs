use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::codex::CodexCliAnalyst;
use harness_artifacts::{Answer, ContractBundle, SCHEMA_VERSION, SensorVisibility, Understanding};
use harness_engine::{RunLifecycle, RunState};

type InteractiveResult<T> = Result<T, Box<dyn Error>>;

pub fn run(request: &str) -> InteractiveResult<()> {
    if request.trim().is_empty() {
        return Err("O pedido não pode estar vazio.".into());
    }

    let workspace = std::env::current_dir()?;
    let mut analyst = CodexCliAnalyst::default();
    let mut lifecycle = RunLifecycle::new();
    lifecycle.transition(RunState::Understanding)?;
    println!("Entendendo o pedido em {}", workspace.display());

    let understanding = collect_understanding(request, &workspace, &mut analyst, &mut lifecycle)?;
    lifecycle.transition(RunState::Contracting)?;
    println!("\nPreparando contrato para aprovação.");

    let (approved_bundle, revisions) = collect_approved_contract(
        request,
        &workspace,
        &understanding,
        &mut analyst,
        &mut lifecycle,
    )?;
    lifecycle.transition(RunState::AcceptanceAuthoring)?;
    let artifact_dir = save_artifacts(
        &workspace,
        &understanding,
        &approved_bundle,
        &revisions,
        lifecycle.history(),
    )?;
    println!(
        "\nContrato aprovado. Artefatos salvos em {}",
        artifact_dir.display()
    );
    println!("Próximo estado: {}", lifecycle.current());
    println!("A autoria de sensores e a execução da tarefa pertencem aos próximos marcos.");
    Ok(())
}

fn collect_understanding(
    request: &str,
    workspace: &Path,
    analyst: &mut CodexCliAnalyst,
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
    analyst: &mut CodexCliAnalyst,
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
    understanding: &Understanding,
    bundle: &ContractBundle,
    revisions: &[ContractRevision],
    states: &[RunState],
) -> InteractiveResult<PathBuf> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let directory = workspace
        .join(".harness")
        .join("runs")
        .join(format!("run-{}-{timestamp}", std::process::id()));
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
