use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude::{Backend, configure_backend, resolve_model};

const SESSION_SCHEMA_VERSION: u32 = 1;
const MAX_HISTORY_MESSAGES: usize = 40;
const MAX_HISTORY_CHARS: usize = 48_000;
const MAX_MESSAGE_CHARS: usize = 12_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub created_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatSession {
    pub schema_version: u32,
    pub session_id: String,
    pub workspace: PathBuf,
    pub mode: String,
    pub provider: String,
    pub model: Option<String>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interrupted_calls: Vec<InterruptedCall>,
}

/// Usage received before a provider call was stopped; cost is unknown until
/// the provider sends its final result event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InterruptedCall {
    pub created_at_unix_ms: u64,
    pub provider: String,
    pub model: Option<String>,
    pub reason: String,
    pub cost_usd: Option<f64>,
    pub cost_complete: bool,
    pub duration_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,
    pub workspace: PathBuf,
    pub provider: String,
    pub model: Option<String>,
    pub message_count: usize,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromotionApproval {
    pub schema_version: u32,
    pub session_id: String,
    pub request: String,
    pub approved_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PromotionStatus {
    pub session_id: String,
    pub status: String,
    pub approved_at_unix_ms: Option<u64>,
}

#[derive(Debug)]
pub struct ChatReply {
    pub text: String,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Debug)]
pub enum ChatEvent {
    Text(String),
    Progress(String),
    ToolStart {
        id: String,
        name: String,
        input: Value,
    },
    ToolEnd {
        id: String,
        ok: bool,
        output: String,
    },
}

pub trait ChatProvider {
    fn send(
        &mut self,
        workspace: &Path,
        prompt: &str,
        on_event: &mut dyn FnMut(ChatEvent),
    ) -> Result<ChatReply, String>;
}

pub struct ClaudeChatProvider {
    binary: OsString,
    backend: Backend,
    model: Option<String>,
    web_enabled: bool,
    max_tool_calls: usize,
    interrupted_usage: Option<InterruptedCall>,
}

impl ClaudeChatProvider {
    pub fn new(workspace: &Path) -> Result<Self, String> {
        let provider = crate::env_file::value(workspace, "DCX_CHAT_PROVIDER")
            .or_else(
                || match crate::env_file::value(workspace, "DCX_PROVIDER").as_deref() {
                    Some("claude") => Some("claude".to_owned()),
                    Some("minimax") => Some("minimax".to_owned()),
                    _ => None,
                },
            )
            .unwrap_or_else(|| "claude".to_owned())
            .to_ascii_lowercase();
        let backend = match provider.as_str() {
            "claude" => Backend::Anthropic,
            "minimax" => Backend::Minimax,
            other => {
                return Err(format!(
                    "DCX_CHAT_PROVIDER desconhecido: {other}; use claude ou minimax"
                ));
            }
        };
        let web_enabled = crate::env_file::value(workspace, "DCX_CHAT_WEB")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        Ok(Self {
            binary: std::env::var_os("DCX_CLAUDE_BIN").unwrap_or_else(|| "claude".into()),
            backend,
            model: resolve_model(workspace, backend, None, true),
            // Web research is performed by the dcode and injected into the
            // prompt, so every model receives the same provider-neutral context.
            web_enabled,
            max_tool_calls: crate::env_file::value(workspace, "DCX_CHAT_MAX_TOOL_CALLS")
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(16),
            interrupted_usage: None,
        })
    }

    pub fn provider_name(&self) -> &'static str {
        self.backend.provider_name()
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn take_interrupted_usage(&mut self) -> Option<InterruptedCall> {
        self.interrupted_usage.take()
    }
}

#[derive(Debug, Serialize)]
pub struct ChatConfig {
    pub provider: String,
    pub model: Option<String>,
    pub web_enabled: bool,
    pub max_tool_calls: usize,
}

/// Reads the chat configuration for `workspace` without requiring the
/// provider binary to be installed, so the settings UI can display it.
pub fn config(workspace: &Path) -> ChatConfig {
    let provider = crate::env_file::value(workspace, "DCX_CHAT_PROVIDER")
        .unwrap_or_else(|| "claude".to_owned())
        .to_ascii_lowercase();
    let backend = match provider.as_str() {
        "minimax" => Backend::Minimax,
        _ => Backend::Anthropic,
    };
    let web_enabled = crate::env_file::value(workspace, "DCX_CHAT_WEB")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    let max_tool_calls = crate::env_file::value(workspace, "DCX_CHAT_MAX_TOOL_CALLS")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(16);
    ChatConfig {
        provider: backend.provider_name().to_owned(),
        model: resolve_model(workspace, backend, None, true),
        web_enabled,
        max_tool_calls,
    }
}

/// Persists chat configuration into `workspace/.env`. `None` fields are left
/// untouched.
pub fn set_config(
    workspace: &Path,
    provider: Option<String>,
    web_enabled: Option<bool>,
    max_tool_calls: Option<usize>,
) -> Result<(), String> {
    let mut updates: Vec<(&str, String)> = Vec::new();
    if let Some(provider) = provider {
        let normalized = provider.to_ascii_lowercase();
        if normalized != "claude" && normalized != "minimax" {
            return Err(format!(
                "provider desconhecido: {normalized}; use claude ou minimax"
            ));
        }
        updates.push(("DCX_CHAT_PROVIDER", normalized));
    }
    if let Some(web_enabled) = web_enabled {
        updates.push((
            "DCX_CHAT_WEB",
            if web_enabled { "1" } else { "0" }.to_owned(),
        ));
    }
    if let Some(max_tool_calls) = max_tool_calls {
        if max_tool_calls == 0 {
            return Err("max_tool_calls deve ser maior que zero".to_owned());
        }
        updates.push(("DCX_CHAT_MAX_TOOL_CALLS", max_tool_calls.to_string()));
    }
    crate::env_file::set_values(workspace, &updates)
}

impl ChatProvider for ClaudeChatProvider {
    fn send(
        &mut self,
        workspace: &Path,
        prompt: &str,
        on_event: &mut dyn FnMut(ChatEvent),
    ) -> Result<ChatReply, String> {
        self.send_with_context(workspace, prompt, &[], on_event)
    }
}

impl ClaudeChatProvider {
    fn send_with_context(
        &mut self,
        workspace: &Path,
        prompt: &str,
        context_workspaces: &[PathBuf],
        on_event: &mut dyn FnMut(ChatEvent),
    ) -> Result<ChatReply, String> {
        let mut command = Command::new(&self.binary);
        command
            .current_dir(workspace)
            .args([
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--no-session-persistence",
                "--permission-mode",
                "plan",
            ])
            .arg("--tools")
            .arg("Read,Glob,Grep")
            .arg("--strict-mcp-config")
            .arg("--disable-slash-commands")
            .arg("--allowedTools=Read,Glob,Grep")
            .arg("--disallowedTools=Bash,Edit,Write,NotebookEdit,WebSearch,WebFetch,ToolSearch,Agent")
            .arg("--append-system-prompt")
            .arg(system_prompt(workspace, self.web_enabled, context_workspaces));
        for context_workspace in context_workspaces {
            command.arg("--add-dir").arg(context_workspace);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(model) = &self.model {
            command.args(["--model", model]);
        }
        configure_backend(&mut command, workspace, self.backend, self.model.as_deref())?;
        command.arg(prompt);

        let started = Instant::now();
        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "Claude CLI não encontrado. Instale em claude.ai/code e faça login com `claude auth login`.".to_owned()
            } else {
                format!("não foi possível iniciar Claude CLI: {error}")
            }
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Claude CLI não forneceu stdout")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Claude CLI não forneceu stderr")?;
        let stderr_handle = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut text);
            text
        });

        let mut streamed = String::new();
        // Only the last assistant message is the answer; earlier ones are the
        // narration the model emits before a tool call.
        let mut last_assistant_text = String::new();
        let mut saw_assistant_text = false;
        let mut result_text = None;
        let mut saw_delta = false;
        let mut is_error = false;
        let mut error_message = None;
        let mut cost_usd = None;
        let mut duration_ms = None;
        let mut input_tokens = None;
        let mut output_tokens = None;
        let mut cache_creation_input_tokens = None;
        let mut cache_read_input_tokens = None;
        let mut parse_error = None;
        let mut tool_calls = 0_usize;
        let mut reported_model = None;

        for line in BufReader::new(stdout).lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    parse_error = Some(format!("não foi possível ler Claude CLI: {error}"));
                    break;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(error) => {
                    parse_error = Some(format!("Claude CLI retornou evento inválido: {error}"));
                    break;
                }
            };
            match value.get("type").and_then(Value::as_str) {
                Some("stream_event") => {
                    if let Some(text) = delta_text(&value) {
                        saw_delta = true;
                        streamed.push_str(text);
                        on_event(ChatEvent::Text(text.to_owned()));
                    }
                }
                Some("assistant") => {
                    if let Some(model) = value
                        .get("message")
                        .and_then(|message| message.get("model"))
                        .and_then(Value::as_str)
                    {
                        reported_model = Some(model.to_owned());
                    }
                    for block in value
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                            if let (Some(id), Some(name)) = (
                                block.get("id").and_then(Value::as_str),
                                block.get("name").and_then(Value::as_str),
                            ) {
                                tool_calls += 1;
                                if tool_calls > self.max_tool_calls {
                                    self.interrupted_usage = Some(InterruptedCall {
                                        created_at_unix_ms: now_ms(),
                                        provider: self.backend.provider_name().to_owned(),
                                        model: reported_model
                                            .clone()
                                            .or_else(|| self.model.clone()),
                                        reason: "tool_limit".to_owned(),
                                        cost_usd: None,
                                        cost_complete: false,
                                        duration_ms: started.elapsed().as_millis() as u64,
                                        input_tokens: usage_value(&value, "input_tokens"),
                                        output_tokens: usage_value(&value, "output_tokens"),
                                        cache_creation_input_tokens: usage_value(
                                            &value,
                                            "cache_creation_input_tokens",
                                        ),
                                        cache_read_input_tokens: usage_value(
                                            &value,
                                            "cache_read_input_tokens",
                                        ),
                                    });
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    return Err(format!(
                                        "limite de chamadas de ferramenta atingido ({}); uso parcial salvo, custo exato indisponível porque o provider não concluiu a resposta",
                                        self.max_tool_calls
                                    ));
                                }
                                on_event(ChatEvent::Progress(format!("Usando {name}…")));
                                on_event(ChatEvent::ToolStart {
                                    id: id.to_owned(),
                                    name: name.to_owned(),
                                    input: block.get("input").cloned().unwrap_or(Value::Null),
                                });
                            }
                        }
                    }
                    if let Some(text) = message_text(&value) {
                        saw_assistant_text = true;
                        last_assistant_text.clone_from(&text);
                        if !saw_delta {
                            on_event(ChatEvent::Text(text));
                        }
                    }
                }
                Some("user") => {
                    if let Some(content) = value
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_array)
                    {
                        for block in content {
                            if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                                if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                                    on_event(ChatEvent::ToolEnd {
                                        id: id.to_owned(),
                                        ok: !block
                                            .get("is_error")
                                            .and_then(Value::as_bool)
                                            .unwrap_or(false),
                                        output: block
                                            .get("content")
                                            .map(Value::to_string)
                                            .unwrap_or_default(),
                                    });
                                }
                            }
                        }
                    }
                }
                Some("result") => {
                    if value.get("is_error").and_then(Value::as_bool) == Some(true) {
                        is_error = true;
                        error_message = value
                            .get("result")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                    }
                    result_text = value
                        .get("result")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    cost_usd = value.get("total_cost_usd").and_then(Value::as_f64);
                    duration_ms = value.get("duration_ms").and_then(Value::as_u64);
                    input_tokens = value
                        .get("usage")
                        .and_then(|usage| usage.get("input_tokens"))
                        .and_then(Value::as_u64);
                    output_tokens = value
                        .get("usage")
                        .and_then(|usage| usage.get("output_tokens"))
                        .and_then(Value::as_u64);
                    cache_creation_input_tokens = value
                        .get("usage")
                        .and_then(|usage| usage.get("cache_creation_input_tokens"))
                        .and_then(Value::as_u64);
                    cache_read_input_tokens = value
                        .get("usage")
                        .and_then(|usage| usage.get("cache_read_input_tokens"))
                        .and_then(Value::as_u64);
                }
                _ => {}
            }
        }

        let status = child
            .wait()
            .map_err(|error| format!("não foi possível aguardar Claude CLI: {error}"))?;
        let stderr = stderr_handle.join().unwrap_or_default();
        if let Some(error) = parse_error {
            return Err(error);
        }
        if !status.success() {
            return Err(format!(
                "Claude CLI encerrou com {status}: {}",
                summarize_stderr(&stderr)
            ));
        }
        if is_error {
            return Err(format!(
                "Claude CLI recusou a conversa: {}",
                error_message.as_deref().unwrap_or("sem detalhes")
            ));
        }

        // The CLI's `result` is the final answer on its own. Concatenating every
        // assistant message instead glued pre-tool narration onto the answer
        // ("I'll read the file.DCX_EVAL_MARKER_42").
        let text = match result_text.filter(|text| !text.trim().is_empty()) {
            Some(result) => result,
            None if !last_assistant_text.trim().is_empty() => last_assistant_text,
            None => streamed,
        };
        if text.trim().is_empty() {
            return Err("Claude CLI não retornou texto para a conversa.".to_owned());
        }
        if !saw_delta && !saw_assistant_text {
            on_event(ChatEvent::Text(text.clone()));
        }

        Ok(ChatReply {
            text,
            model: reported_model.or_else(|| self.model.clone()),
            cost_usd,
            duration_ms: duration_ms.unwrap_or_else(|| started.elapsed().as_millis() as u64),
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
        })
    }
}

fn system_prompt(workspace: &Path, web_enabled: bool, context_workspaces: &[PathBuf]) -> String {
    let mut prompt = format!(
        "Você é o dcode em modo conversa read-only.\n\
         Projeto atual: {}.\n\
         É obrigatório responder em português do Brasil (pt-BR), mesmo que o histórico, os arquivos ou a pergunta estejam em outro idioma. Não responda em inglês, salvo quando o usuário pedir explicitamente. Isso vale também para as frases curtas que você escreve antes de usar uma ferramenta.\n\
         Você pode ler arquivos e buscar no projeto com Read, Glob e Grep. {} Nunca edite, crie, apague ou execute comandos; não use Bash ou subagentes. Não altere o projeto. Faça buscas direcionadas, não repita uma busca que não encontrou resultados e leia apenas os arquivos necessários para responder.\n\
         Converse sobre o projeto, tire dúvidas de arquitetura e ajude o usuário a decidir o próximo passo. Se o usuário pedir para criar, editar, apagar arquivos ou implementar algo, recuse imediatamente, sem usar ferramentas e sem investigar o projeto para justificar: diga em uma ou duas frases que a conversa é somente leitura e que, para fazer a mudança, ele pode usar \"Promover para execução\" (ou `dcx run`), que cria um contrato com critérios de aceite antes de alterar o projeto.\n\
         Responda de forma direta. Se precisar de contexto, leia os arquivos relevantes antes de responder. Não invente conteúdo de arquivos.\n\
         Nunca se apresente nem use cumprimentos automáticos como abertura. Se a primeira mensagem do usuário for apenas uma saudação curta (\"oi\", \"hey\" ou similar), responda com uma única pergunta concisa perguntando no que pode ajudar, como \"O que posso fazer por você?\". Adapte o tom ao do usuário: se ele for informal ou bem-humorado, seja informal e retribua o humor; se for sério e técnico, seja cordial e direto. Nunca force um tom formal quando o usuário é descontraído.",
        workspace.display(),
        if web_enabled {
            "Quando houver contexto marcado como PESQUISA WEB DO DCODE, use somente essas fontes, cite as URLs e não tente pesquisar novamente. Se esse contexto não existir, diga que a informação atual não foi verificada; nunca invente dados."
        } else {
            "A pesquisa externa está desabilitada nesta sessão; não alegue ter pesquisado e não invente dados atuais."
        }
    );
    if !context_workspaces.is_empty() {
        prompt.push_str("\nRepositórios adicionais autorizados para leitura: ");
        prompt.push_str(
            &context_workspaces
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );
        prompt.push_str(". Consulte-os somente quando forem relevantes.");
    }
    prompt
}

fn usage_value(value: &Value, name: &str) -> Option<u64> {
    value
        .get("message")
        .and_then(|message| message.get("usage"))
        .and_then(|usage| usage.get(name))
        .and_then(Value::as_u64)
}

fn delta_text(value: &Value) -> Option<&str> {
    let event = value.get("event")?;
    if event.get("type")?.as_str()? != "content_block_delta" {
        return None;
    }
    let delta = event.get("delta")?;
    if delta.get("type")?.as_str()? != "text_delta" {
        return None;
    }
    delta.get("text")?.as_str()
}

fn message_text(value: &Value) -> Option<String> {
    let content = value.get("message")?.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
}

fn summarize_stderr(stderr: &str) -> String {
    let diagnostics = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join("\n");
    if diagnostics.is_empty() {
        "diagnóstico indisponível".to_owned()
    } else {
        diagnostics
    }
}

pub fn run(workspace_arg: Option<PathBuf>, session_id: Option<String>) -> Result<(), String> {
    let workspace = resolve_workspace(workspace_arg)?;
    crate::env_file::load_for_workspace(&workspace)?;
    let mut session = match session_id {
        Some(session_id) => load_session(&workspace, &session_id)?,
        None => new_session(&workspace)?,
    };
    let mut provider = ClaudeChatProvider::new(&workspace)?;
    session.workspace = workspace.clone();
    session.mode = "explore".to_owned();
    session.provider = provider.provider_name().to_owned();
    session.model = provider.model().map(str::to_owned);
    save_session(&workspace, &mut session)?;

    println!("dcx chat — modo read-only");
    println!("Projeto: {}", workspace.display());
    println!("Sessão: {}", session.session_id);
    println!("Digite /help para comandos e /exit para sair.\n");

    while let Some(input) = read_line("você> ")? {
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        match input {
            "/exit" | "/quit" | "/sair" => break,
            "/help" => {
                println!(
                    "/exit  encerra a sessão\n\
                     /session  mostra o id e o caminho da sessão\n\
                     /help  mostra esta ajuda"
                );
                continue;
            }
            "/session" => {
                println!("Sessão: {}", session.session_id);
                println!(
                    "Arquivo: {}",
                    session_path(&workspace, &session.session_id).display()
                );
                continue;
            }
            command if command.starts_with('/') => {
                println!("Comando desconhecido: {command}. Use /help.");
                continue;
            }
            _ => {}
        }

        session.messages.push(ChatMessage {
            role: "user".to_owned(),
            content: input.to_owned(),
            created_at_unix_ms: now_ms(),
            cost_usd: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        });
        save_session(&workspace, &mut session)?;

        let prompt = build_prompt(&session.messages);
        print!("\nassistente> ");
        let _ = io::stdout().flush();
        match provider.send(&workspace, &prompt, &mut |event| {
            if let ChatEvent::Text(text) = event {
                print!("{text}");
            }
            let _ = io::stdout().flush();
        }) {
            Ok(reply) => {
                println!();
                if let Some(model) = &reply.model {
                    session.model = Some(model.clone());
                }
                if let Some(cost) = reply.cost_usd {
                    eprintln!("[chat: {} ms, US${cost:.4}]", reply.duration_ms);
                } else {
                    eprintln!("[chat: {} ms]", reply.duration_ms);
                }
                session.messages.push(ChatMessage {
                    role: "assistant".to_owned(),
                    content: reply.text,
                    created_at_unix_ms: now_ms(),
                    cost_usd: reply.cost_usd,
                    duration_ms: Some(reply.duration_ms),
                    input_tokens: reply.input_tokens,
                    output_tokens: reply.output_tokens,
                    cache_creation_input_tokens: reply.cache_creation_input_tokens,
                    cache_read_input_tokens: reply.cache_read_input_tokens,
                });
                save_session(&workspace, &mut session)?;
            }
            Err(error) => {
                if let Some(call) = provider.take_interrupted_usage() {
                    session.interrupted_calls.push(call);
                    save_session(&workspace, &mut session)?;
                }
                println!();
                eprintln!("Erro: {error}");
            }
        }
    }

    save_session(&workspace, &mut session)?;
    println!(
        "\nSessão salva em {}",
        session_path(&workspace, &session.session_id).display()
    );
    Ok(())
}

pub fn list(workspace_arg: Option<PathBuf>) -> Result<(), String> {
    let workspace = resolve_workspace(workspace_arg)?;
    let sessions = list_sessions(&workspace)?;
    if sessions.is_empty() {
        let root = sessions_root(&workspace);
        println!("Nenhuma sessão em {}", root.display());
        return Ok(());
    }
    println!("Sessões em {}:", sessions_root(&workspace).display());
    for session in sessions {
        println!(
            "  {}  {} mensagem(ns)  {}",
            session.session_id,
            session.message_count,
            format_age(session.updated_at_unix_ms)
        );
    }
    Ok(())
}

pub fn list_sessions(workspace: &Path) -> Result<Vec<SessionSummary>, String> {
    let root = sessions_root(workspace);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let pinned_ids = load_pinned(workspace);
    let entries = fs::read_dir(&root)
        .map_err(|error| format!("não foi possível ler {}: {error}", root.display()))?;
    let mut sessions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("sessão inválida: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("sessão inválida: {error}"))?
            .is_dir()
        {
            continue;
        }
        let path = entry.path().join("session.json");
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(session) = serde_json::from_slice::<ChatSession>(&bytes) else {
            continue;
        };
        sessions.push(SessionSummary {
            pinned: pinned_ids.contains(&session.session_id),
            title: session_title(&session),
            session_id: session.session_id,
            workspace: session.workspace,
            provider: session.provider,
            model: session.model,
            message_count: session.messages.len(),
            created_at_unix_ms: session.created_at_unix_ms,
            updated_at_unix_ms: session.updated_at_unix_ms,
        });
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at_unix_ms));
    Ok(sessions)
}

fn pinned_path(workspace: &Path) -> PathBuf {
    workspace.join(".dcx").join("pinned.json")
}

fn load_pinned(workspace: &Path) -> Vec<String> {
    let Ok(bytes) = fs::read(pinned_path(workspace)) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Pins or unpins a session. Errors only on an unwritable `.dcx`
/// directory; pinning a session that no longer exists is harmless (it just
/// won't show up until a session with that id exists again).
pub fn set_pinned(workspace: &Path, session_id: &str, pinned: bool) -> Result<(), String> {
    validate_session_id(session_id)?;
    let mut ids = load_pinned(workspace);
    let already_pinned = ids.iter().any(|id| id == session_id);
    if pinned && !already_pinned {
        ids.push(session_id.to_owned());
    } else if !pinned {
        ids.retain(|id| id != session_id);
    }
    let path = pinned_path(workspace);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("não foi possível criar {}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec(&ids).map_err(|error| error.to_string())?;
    fs::write(&path, bytes)
        .map_err(|error| format!("não foi possível gravar {}: {error}", path.display()))
}

fn session_title(session: &ChatSession) -> String {
    let source = session
        .messages
        .iter()
        .find(|message| message.role == "user")
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .unwrap_or("Nova conversa");
    let title = source.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() > 48 {
        format!("{}…", title.chars().take(47).collect::<String>())
    } else {
        title
    }
}

pub fn open_session(workspace: &Path, session_id: Option<&str>) -> Result<ChatSession, String> {
    match session_id {
        Some(id) => load_session(workspace, id),
        None => new_session(workspace),
    }
}

pub fn session(workspace: &Path, session_id: &str) -> Result<ChatSession, String> {
    load_session(workspace, session_id)
}

/// Converts an exploratory session into the request consumed by the normal
/// contract/sensor/run flow. This does not execute anything or modify files.
pub fn request_from_session(workspace: &Path, session_id: &str) -> Result<String, String> {
    let session = load_session(workspace, session_id)?;
    let request = session
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .map(|message| message.content.trim())
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>();
    if request.is_empty() {
        return Err("a sessão não contém mensagens do usuário para promover".to_owned());
    }
    let mut prompt =
        "Promover esta conversa exploratória para um projeto implementável. ".to_owned();
    prompt.push_str(
        "Preserve as decisões e restrições discutidas; valide o que ainda estiver ambíguo.\n\n",
    );
    prompt.push_str("Mensagens do usuário:\n");
    for (index, message) in request.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", index + 1, message));
    }
    Ok(prompt)
}

pub fn approve_session(workspace: &Path, session_id: &str) -> Result<PromotionApproval, String> {
    let request = request_from_session(workspace, session_id)?;
    let approval = PromotionApproval {
        schema_version: SESSION_SCHEMA_VERSION,
        session_id: session_id.to_owned(),
        request,
        approved_at_unix_ms: now_ms(),
    };
    let path = sessions_root(workspace)
        .join(session_id)
        .join("promotion_approval.json");
    let bytes = serde_json::to_vec_pretty(&approval)
        .map_err(|error| format!("não foi possível serializar aprovação: {error}"))?;
    fs::write(&path, bytes).map_err(|error| {
        format!(
            "não foi possível salvar aprovação {}: {error}",
            path.display()
        )
    })?;
    Ok(approval)
}

pub fn promotion_status(workspace: &Path, session_id: &str) -> Result<PromotionStatus, String> {
    validate_session_id(session_id)?;
    let path = sessions_root(workspace)
        .join(session_id)
        .join("promotion_approval.json");
    if !path.is_file() {
        let _ = request_from_session(workspace, session_id)?;
        return Ok(PromotionStatus {
            session_id: session_id.to_owned(),
            status: "ready_for_approval".to_owned(),
            approved_at_unix_ms: None,
        });
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("não foi possível ler aprovação {}: {error}", path.display()))?;
    let approval: PromotionApproval = serde_json::from_slice(&bytes)
        .map_err(|error| format!("aprovação inválida {}: {error}", path.display()))?;
    if approval.session_id != session_id {
        return Err("aprovação contém sessão divergente".to_owned());
    }
    Ok(PromotionStatus {
        session_id: session_id.to_owned(),
        status: "approved".to_owned(),
        approved_at_unix_ms: Some(approval.approved_at_unix_ms),
    })
}

pub fn approved_request(workspace: &Path, session_id: &str) -> Result<String, String> {
    let path = sessions_root(workspace)
        .join(session_id)
        .join("promotion_approval.json");
    let bytes = fs::read(&path).map_err(|error| format!("sessão ainda não aprovada: {error}"))?;
    let approval: PromotionApproval =
        serde_json::from_slice(&bytes).map_err(|error| format!("aprovação inválida: {error}"))?;
    if approval.session_id != session_id {
        return Err("aprovação contém sessão divergente".to_owned());
    }
    Ok(approval.request)
}

#[allow(dead_code)]
pub fn send_message(
    workspace: &Path,
    session: &mut ChatSession,
    message: &str,
    on_text: &mut dyn FnMut(&str),
) -> Result<ChatReply, String> {
    let mut on_event = |event| {
        if let ChatEvent::Text(text) = event {
            on_text(&text);
        }
    };
    send_message_with_events(workspace, session, message, &mut on_event)
}

pub fn send_message_with_events(
    workspace: &Path,
    session: &mut ChatSession,
    message: &str,
    on_event: &mut dyn FnMut(ChatEvent),
) -> Result<ChatReply, String> {
    send_message_with_context(workspace, session, message, &[], on_event)
}

pub fn send_message_with_context(
    workspace: &Path,
    session: &mut ChatSession,
    message: &str,
    context_workspaces: &[String],
    on_event: &mut dyn FnMut(ChatEvent),
) -> Result<ChatReply, String> {
    if message.trim().is_empty() {
        return Err("mensagem vazia".to_owned());
    }
    let context_workspaces = validate_context_workspaces(workspace, context_workspaces)?;
    session.messages.push(ChatMessage {
        role: "user".to_owned(),
        content: message.to_owned(),
        created_at_unix_ms: now_ms(),
        cost_usd: None,
        duration_ms: None,
        input_tokens: None,
        output_tokens: None,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    });
    let mut provider = ClaudeChatProvider::new(workspace)?;
    session.provider = provider.provider_name().to_owned();
    session.model = provider.model().map(str::to_owned);
    save_session(workspace, session)?;
    let mut prompt = build_prompt(&session.messages);
    if !context_workspaces.is_empty() {
        prompt.push_str("\n\n[REPOSITÓRIOS ADICIONAIS AUTORIZADOS]\n");
        for repo in &context_workspaces {
            prompt.push_str("- ");
            prompt.push_str(&repo.display().to_string());
            prompt.push('\n');
        }
        prompt.push_str("Consulte esses repositórios quando forem relevantes para a pergunta.\n[FIM DOS REPOSITÓRIOS AUTORIZADOS]\n");
    }
    if crate::env_file::value(workspace, "DCX_CHAT_WEB")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
        && crate::web::requested(message)
    {
        on_event(ChatEvent::Progress("Pesquisando na web…".to_owned()));
        let context = crate::web::search(message)?;
        on_event(ChatEvent::Progress(
            "Pesquisa concluída; analisando fontes…".to_owned(),
        ));
        prompt.push_str("\n\n[CONTEXTO DE PESQUISA WEB DO DCODE]\n");
        prompt.push_str(&context);
        prompt
            .push_str("\n[FIM DO CONTEXTO WEB]\nUse as fontes acima e cite as URLs na resposta.\n");
    }
    let reply = match provider.send_with_context(workspace, &prompt, &context_workspaces, on_event)
    {
        Ok(reply) => reply,
        Err(error) => {
            if let Some(call) = provider.take_interrupted_usage() {
                session.interrupted_calls.push(call);
                save_session(workspace, session)?;
            }
            return Err(error);
        }
    };
    if let Some(model) = &reply.model {
        session.model = Some(model.clone());
    }
    session.messages.push(ChatMessage {
        role: "assistant".to_owned(),
        content: reply.text.clone(),
        created_at_unix_ms: now_ms(),
        cost_usd: reply.cost_usd,
        duration_ms: Some(reply.duration_ms),
        input_tokens: reply.input_tokens,
        output_tokens: reply.output_tokens,
        cache_creation_input_tokens: reply.cache_creation_input_tokens,
        cache_read_input_tokens: reply.cache_read_input_tokens,
    });
    save_session(workspace, session)?;
    Ok(reply)
}

fn validate_context_workspaces(
    workspace: &Path,
    requested: &[String],
) -> Result<Vec<PathBuf>, String> {
    if requested.len() > 8 {
        return Err("no máximo 8 repositórios adicionais podem entrar no contexto".to_owned());
    }
    let primary = fs::canonicalize(workspace)
        .map_err(|error| format!("workspace principal inválido: {error}"))?;
    let mut approved = Vec::new();
    for raw in requested {
        let selected = fs::canonicalize(raw)
            .map_err(|error| format!("repositório de contexto indisponível: {error}"))?;
        if selected == primary || approved.contains(&selected) {
            continue;
        }
        if !selected.is_dir() {
            return Err(format!(
                "repositório de contexto não é uma pasta: {}",
                selected.display()
            ));
        }
        let output = Command::new("git")
            .args([
                "-C",
                selected.to_string_lossy().as_ref(),
                "rev-parse",
                "--show-toplevel",
            ])
            .output()
            .map_err(|error| format!("não foi possível validar repositório Git: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pasta autorizada não é um repositório Git: {}",
                selected.display()
            ));
        }
        let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        let root = fs::canonicalize(root).map_err(|error| format!("raiz Git inválida: {error}"))?;
        if root == primary || approved.contains(&root) {
            continue;
        }
        approved.push(root);
    }
    Ok(approved)
}

fn resolve_workspace(workspace_arg: Option<PathBuf>) -> Result<PathBuf, String> {
    let path = match workspace_arg {
        Some(path) => path,
        None => std::env::current_dir()
            .map_err(|error| format!("não foi possível obter o diretório atual: {error}"))?,
    };
    let workspace = fs::canonicalize(&path)
        .map_err(|error| format!("não foi possível abrir {}: {error}", path.display()))?;
    if !workspace.is_dir() {
        return Err(format!("{} não é um diretório", workspace.display()));
    }
    Ok(workspace)
}

fn new_session(workspace: &Path) -> Result<ChatSession, String> {
    let timestamp = now_ms();
    Ok(ChatSession {
        schema_version: SESSION_SCHEMA_VERSION,
        session_id: new_session_id(),
        workspace: workspace.to_path_buf(),
        mode: "explore".to_owned(),
        provider: "claude".to_owned(),
        model: None,
        created_at_unix_ms: timestamp,
        updated_at_unix_ms: timestamp,
        messages: Vec::new(),
        interrupted_calls: Vec::new(),
    })
}

fn load_session(workspace: &Path, session_id: &str) -> Result<ChatSession, String> {
    validate_session_id(session_id)?;
    let path = session_path(workspace, session_id);
    let bytes = fs::read(&path)
        .map_err(|error| format!("sessão {} não encontrada: {error}", path.display()))?;
    let mut session: ChatSession = serde_json::from_slice(&bytes)
        .map_err(|error| format!("sessão {} inválida: {error}", path.display()))?;
    if session.schema_version != SESSION_SCHEMA_VERSION {
        return Err(format!(
            "sessão {} usa schema_version {}; esperado {}",
            session.session_id, session.schema_version, SESSION_SCHEMA_VERSION
        ));
    }
    if session.session_id != session_id {
        return Err(format!(
            "sessão {} contém id divergente {}",
            path.display(),
            session.session_id
        ));
    }
    if let Some(message) = session
        .messages
        .iter()
        .find(|message| message.role != "user" && message.role != "assistant")
    {
        return Err(format!("sessão contém papel inválido: {}", message.role));
    }
    session.workspace = workspace.to_path_buf();
    Ok(session)
}

fn save_session(workspace: &Path, session: &mut ChatSession) -> Result<(), String> {
    session.updated_at_unix_ms = now_ms();
    let directory = sessions_root(workspace).join(&session.session_id);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("não foi possível criar {}: {error}", directory.display()))?;
    let path = directory.join("session.json");
    let temporary = directory.join("session.json.tmp");
    let bytes = serde_json::to_vec_pretty(session)
        .map_err(|error| format!("não foi possível serializar a sessão: {error}"))?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("não foi possível salvar {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("não foi possível publicar {}: {error}", path.display()))
}

fn sessions_root(workspace: &Path) -> PathBuf {
    workspace.join(".dcx").join("sessions")
}

fn session_path(workspace: &Path, session_id: &str) -> PathBuf {
    sessions_root(workspace)
        .join(session_id)
        .join("session.json")
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty()
        || session_id.len() > 128
        || session_id == "."
        || session_id == ".."
        || !session_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err(format!("id de sessão inválido: {session_id}"));
    }
    Ok(())
}

fn new_session_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("chat-{}-{timestamp}", std::process::id())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn format_age(updated_at_unix_ms: u64) -> String {
    let seconds = now_ms().saturating_sub(updated_at_unix_ms) / 1_000;
    if seconds < 60 {
        format!("há {seconds}s")
    } else if seconds < 3_600 {
        format!("há {}min", seconds / 60)
    } else if seconds < 86_400 {
        format!("há {}h", seconds / 3_600)
    } else {
        format!("há {}d", seconds / 86_400)
    }
}

fn read_line(prompt: &str) -> Result<Option<String>, String> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|error| format!("não foi possível escrever no terminal: {error}"))?;
    let mut line = String::new();
    let read = io::stdin()
        .read_line(&mut line)
        .map_err(|error| format!("não foi possível ler do terminal: {error}"))?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\n', '\r']).to_owned()))
}

fn build_prompt(messages: &[ChatMessage]) -> String {
    let mut selected = Vec::new();
    let mut characters = 0;
    for message in messages.iter().rev() {
        if selected.len() >= MAX_HISTORY_MESSAGES {
            break;
        }
        let content = truncate_chars(&message.content, MAX_MESSAGE_CHARS);
        let size = content.chars().count() + 16;
        if !selected.is_empty() && characters + size > MAX_HISTORY_CHARS {
            break;
        }
        characters += size;
        selected.push((role_label(&message.role).to_owned(), content));
    }
    selected.reverse();

    let mut prompt = String::from("Histórico da conversa:\n");
    if selected.len() < messages.len() {
        prompt.push_str("(mensagens antigas omitidas)\n");
    }
    for (role, content) in selected {
        prompt.push_str(&role);
        prompt.push_str(": ");
        prompt.push_str(&content);
        prompt.push('\n');
    }
    prompt.push_str("\nResponda à última mensagem do usuário.");
    prompt
}

fn role_label(role: &str) -> &str {
    if role == "assistant" {
        "Assistente"
    } else {
        "Usuário"
    }
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        return value.to_owned();
    }
    let mut truncated = value.chars().take(maximum).collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_reject_path_traversal() {
        assert!(validate_session_id("../escape").is_err());
        assert!(validate_session_id("a/b").is_err());
        assert!(validate_session_id(".").is_err());
        assert!(validate_session_id("chat-123").is_ok());
    }

    #[test]
    fn context_workspaces_must_be_external_git_repositories() {
        let root = std::env::temp_dir().join(format!("dcx-context-{}", new_session_id()));
        let primary = root.join("primary");
        let external = root.join("external");
        fs::create_dir_all(&primary).expect("workspace principal");
        fs::create_dir_all(&external).expect("repo externo");
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&external)
            .status()
            .expect("git init");
        assert!(status.success());

        let granted =
            validate_context_workspaces(&primary, &[external.to_string_lossy().into_owned()])
                .expect("repo externo autorizado");
        assert_eq!(
            granted,
            vec![fs::canonicalize(&external).expect("caminho real")]
        );

        let primary_grant =
            validate_context_workspaces(&primary, &[primary.to_string_lossy().into_owned()])
                .expect("workspace principal ignorado");
        assert!(primary_grant.is_empty());

        let non_repo = root.join("non-repo");
        fs::create_dir_all(&non_repo).expect("pasta sem git");
        assert!(
            validate_context_workspaces(&primary, &[non_repo.to_string_lossy().into_owned()])
                .is_err()
        );
        fs::remove_dir_all(root).expect("limpar teste");
    }

    #[test]
    fn prompt_keeps_recent_messages_and_labels_roles() {
        let messages = vec![
            ChatMessage {
                role: "user".to_owned(),
                content: "primeira".to_owned(),
                created_at_unix_ms: 1,
                cost_usd: None,
                duration_ms: None,
                input_tokens: None,
                output_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
            ChatMessage {
                role: "assistant".to_owned(),
                content: "segunda".to_owned(),
                created_at_unix_ms: 2,
                cost_usd: None,
                duration_ms: None,
                input_tokens: None,
                output_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        ];
        let prompt = build_prompt(&messages);
        assert!(prompt.contains("Usuário: primeira"));
        assert!(prompt.contains("Assistente: segunda"));
        assert!(prompt.contains("Responda à última mensagem do usuário."));
    }

    #[test]
    fn request_from_session_keeps_user_decisions_without_executing() {
        let workspace = std::env::temp_dir().join(format!("dcx-promote-{}", new_session_id()));
        fs::create_dir_all(&workspace).expect("workspace");
        let mut session = new_session(&workspace).expect("sessão");
        for (role, content) in [
            ("user", "Construir um conversor CSV para JSON"),
            ("assistant", "Use streaming"),
            ("user", "Preservar campos entre aspas"),
        ] {
            session.messages.push(ChatMessage {
                role: role.to_owned(),
                content: content.to_owned(),
                created_at_unix_ms: now_ms(),
                cost_usd: None,
                duration_ms: None,
                input_tokens: None,
                output_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            });
        }
        let session_id = session.session_id.clone();
        save_session(&workspace, &mut session).expect("salvar sessão");

        let request = request_from_session(&workspace, &session_id).expect("promover");
        assert!(request.contains("Construir um conversor CSV para JSON"));
        assert!(request.contains("Preservar campos entre aspas"));
        assert!(!request.contains("Use streaming"));

        let approval = approve_session(&workspace, &session_id).expect("aprovar promoção");
        assert_eq!(approval.session_id, session_id);
        assert!(
            workspace
                .join(".dcx/sessions")
                .join(&session_id)
                .join("promotion_approval.json")
                .is_file()
        );
        assert_eq!(
            promotion_status(&workspace, &session_id)
                .expect("status")
                .status,
            "approved"
        );

        fs::remove_dir_all(workspace).expect("limpar workspace");
    }

    #[test]
    fn system_prompt_does_not_introduce_itself() {
        let workspace = Path::new("/tmp/dcx-chat-test");
        let prompt = system_prompt(workspace, false, &[]);
        assert!(
            !prompt.contains("Olá"),
            "system prompt must not contain a greeting"
        );
        assert!(
            !prompt.contains("Sou o dcode"),
            "system prompt must not introduce the assistant by name"
        );
        assert!(
            prompt.contains("Nunca se apresente"),
            "system prompt must instruct the model not to introduce itself"
        );
        assert!(
            prompt.contains("Adapte o tom ao do usuário"),
            "system prompt must instruct the model to adapt tone to user"
        );
    }
}
