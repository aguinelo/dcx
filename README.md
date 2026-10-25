# Harness

CLI for running software tasks through a contract, acceptance sensors, an isolated executor, and an independent verifier.

The MVP is built in rounds described in `docs/roadmap.md`; `docs/milestones/` records each milestone.

## Development

The core and CLI are written in Rust. The pilot tasks and their project preset use Python. User-facing documentation and messages use pt-BR; identifiers and code follow the English names defined by the specification.

```sh
cargo build --workspace
cargo test --workspace
cargo run -p harness-cli -- schemas --check
cargo run -p harness-cli -- run --demo
cargo run -p harness-cli -- run --job-02
```

Run the live `cli-01` pilot through the installed Codex CLI:

```sh
cargo run -p harness-cli -- run --pilot
```

This sends the contract and workspace context to the Codex provider selected by the user's Codex CLI configuration. Set `HARNESS_CODEX_MODEL` to choose the executor model and `HARNESS_VERIFIER_MODEL` to choose the verifier model. The executor runs with `workspace-write`; the verifier runs with `read-only`.

Regenerate versioned JSON Schemas after changing a Rust artifact:

```sh
cargo run -p harness-cli -- schemas --write
```

## Project status

`harness run --demo` exercises a deterministic smoke path with a built-in worker and acceptance sensors. It does not call a model provider or provide an OS security sandbox, so it is not an evaluation of the MVP thesis and must not be used with untrusted code. `harness run --pilot` uses the installed Codex CLI for the executor and verifier; it applies the CLI's workspace-write and read-only sandbox modes.

The harness supports three providers behind the same role interfaces: `codex` (default), `claude`, and `minimax`. Select one with `HARNESS_PROVIDER=claude` or `HARNESS_PROVIDER=minimax`; `HARNESS_PROVIDER=auto` tries Claude first and falls back to Codex. Each role keeps its own CLI sandbox or permission mode.

MiniMax uses its Anthropic-compatible endpoint through the installed Claude CLI, so the same read-only and workspace-write permission modes apply. Configure `MINIMAX_API_KEY` and, optionally, `HARNESS_MINIMAX_MODEL` (default `MiniMax-M3`) and `HARNESS_MINIMAX_BASE_URL` (default `https://api.minimax.io/anthropic`; use `https://api.minimax.cn/anthropic` in China). The `.env.example` file documents every variable.

The engine derives `step_report.files_changed` from workspace snapshots around each executor attempt, instead of trusting the executor's own list. Snapshots skip symlinks and enforce 16 MiB per file and 64 MiB per workspace limits.

Acceptance sensors are frozen with a SHA-256 `SensorLock` before execution. The demo and pilot also run every sensor against an empty baseline and refuse a sensor that already passes, which catches checks written against the implementation instead of the contract.

The demo and pilot execute acceptance sensors as Python scripts through `harness-sensors`. Scripts are materialized outside the task workspace, made read-only, checked against their locked source before every run, and time out after 60 seconds. They receive `HARNESS_WORKSPACE`, `HARNESS_SENSOR_ID` and `HARNESS_CRITERION_ID`; exit code 0 means pass. The runner clears the inherited environment and passes only a minimal allowlist. Set `HARNESS_SENSOR_SANDBOX=auto` or `required` to use `sandbox-exec` on macOS or bubblewrap (`bwrap`) on Linux, denying network and writes outside the workspace. `auto` falls back to an unsandboxed run when the helper is unavailable; `required` fails instead.

Start an interactive understanding and contract approval flow for the current project:

```sh
cargo run -p harness-cli -- run --request "Uma ferramenta de linha de comando pra organizar minha pasta de downloads."
```

The analyst inspects the current directory read-only, asks blocking questions in the terminal, prepares a contract, and revises it after a rejection. On approval, the command saves the understanding, contract, and sensor suite under `.harness/runs/`.

It then asks the acceptance author for executable Python sensors, validates them against the approved suite, and runs them against the current workspace. Every sensor must fail before implementation; a sensor that already passes is treated as suspect and sent back for correction. Script sources are kept outside the project under `HARNESS_STATE_DIR` (default `~/.local/state/harness`); `.harness/runs/<run_id>/sensor_manifest.json` records the locked checksums.

After the sensors are locked, the planner produces a versioned plan. The executor implements it with `workspace-write` in an isolated copy under `HARNESS_STATE_DIR/runs/<run_id>/workspace`, and the verifier checks that copy with `read-only`. Visible sensors guide retries; hidden sensors run only during verification. The run stops at `awaiting_merge` and writes `plan.json`, `step_reports.json`, `sensor_results.json`, `verdict.json` and `delivery_report.json` under `.harness/runs/<run_id>/`.

If the run is approved, only the changed files are copied back into the project for review and merge. The apply is transactional: originals are backed up and restored if any file fails. If the run is rejected, the project is left untouched and the execution copy is preserved for inspection. Independent plan steps run in parallel in isolated step workspaces; each step gets the criteria and sensors it covers, and the integration step validates the full contract. Changes from parallel steps in the same group are merged into the run workspace and fail on file conflicts. Each run writes `events.jsonl`, and `harness state prune --older-than-days <N>` removes old run directories. A project lock prevents two runs from applying changes at the same time. After an interruption, `harness resume --run <run_id>` reloads the saved contract, plan, sensors, and isolated workspace and skips steps already completed and merged (except the integration step).

If a sensor looks wrong, `harness dispute --run <run_id> --sensor <sensor_id> --reason "<motivo>"` asks the human to accept or reject the dispute. Acceptance re-authors only that sensor, regenerates its lock and implementation artifacts, and leaves the rest of the suite untouched; run `harness resume` afterwards.

## Conversa read-only

Start a free conversation about the current project, without a contract, sensors, or writes:

```sh
cargo run -p harness-cli -- chat
```

The chat keeps a persistent session under `.harness/sessions/<session_id>/session.json`, streams the assistant response, and uses the Claude CLI in plan mode with only `Read`, `Glob`, and `Grep`; `Bash`, `Edit`, `Write`, and `NotebookEdit` are explicitly denied. It can discuss architecture and inspect files, but it cannot change the project. Use `/help`, `/session`, and `/exit` inside the chat.

Resume a session or list the project's sessions:

```sh
cargo run -p harness-cli -- chat --session <session_id>
cargo run -p harness-cli -- chat --list
```

Use `--workspace <dir>` to point at another project. `HARNESS_CHAT_PROVIDER=minimax` switches the chat to MiniMax 3; if it is omitted, `HARNESS_PROVIDER=minimax` also selects MiniMax. `HARNESS_CHAT_MODEL` overrides the model for either provider; otherwise use `HARNESS_CLAUDE_MODEL` or `HARNESS_MINIMAX_MODEL`. `HARNESS_CLAUDE_BIN` overrides the Claude executable. To implement something discussed in the chat, start a `run --request`; the chat itself is intentionally read-only.

To promote a saved conversation into the normal implementation flow, run this from the same project directory:

```sh
cargo run -p harness-cli -- run --from-session <session_id>
```

This transfers only the user's decisions into a new request. The analyst still asks blocking questions, the contract and sensors still require approval, and no project file is changed before the regular merge gate.

`harness run --job-02` is the adversarial benchmark: a shortcut passes the visible step but fails the hidden sensors in the integration step. The roadmap for all rounds is in `docs/roadmap.md`.

Set `HARNESS_ACCEPTANCE_MODEL` to choose the acceptance author model, `HARNESS_PLANNER_MODEL` for the planner, and `HARNESS_VERIFIER_MODEL` for the verifier. With Codex, each falls back to `HARNESS_CODEX_MODEL` or the CLI default; with Claude, to `HARNESS_CLAUDE_MODEL`; with MiniMax, to `HARNESS_MINIMAX_MODEL` or `MiniMax-M3`. Set `HARNESS_CLAUDE_BIN` to override the Claude executable.

## Ambiente e segredos

`harness` carrega `.env` do diretório atual antes de ler a configuração; `harness chat --workspace <dir>` também carrega o `.env` daquele projeto. Use `HARNESS_ENV_FILE=<arquivo>` para apontar outro arquivo explicitamente. As chaves e configurações de provider lidas do `.env` são aplicadas diretamente aos processos do Claude/MiniMax, então não precisam ser exportadas no shell.

Para abrir a API local que um app desktop pode consumir:

```sh
cargo run -q -p harness-cli -- serve --bind 127.0.0.1:8787
```

O daemon expõe `GET /health`, `GET /sessions?workspace=<dir>`, `POST /chat`, `POST /runs/from-session`, `GET /runs/from-session/status`, `POST /runs/from-session/approve`, `POST /runs/from-session/start` e `GET /runs/status`. O chat recebe `workspace`, `session_id` opcional e `message` em JSON, retornando eventos SSE `session`, `token`, `done` ou `error`. Após a aprovação, `POST /runs/from-session/start` cria um job persistido com `status: queued`; o status pode ser consultado pela API. Nesta etapa o job ainda não executa o fluxo interativo sozinho.

O `.env` é ignorado pelo Git, não é copiado para o workspace isolado de um run e não pode ser aplicado de volta pelo merge gate, então chaves como `ANTHROPIC_API_KEY` e `MINIMAX_API_KEY` podem ficar nele sem serem enviadas ao executor. O `.env.example` versionado documenta os providers, modelos e chaves.
