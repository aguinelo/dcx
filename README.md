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

The harness supports two local model providers behind the same role interfaces: `codex` (default) and `claude`. Select one with `HARNESS_PROVIDER=claude`, or use `HARNESS_PROVIDER=auto` to try Claude first and fall back to Codex; each role uses its own CLI sandbox or permission mode.

The engine derives `step_report.files_changed` from workspace snapshots around each executor attempt, instead of trusting the executor's own list. Snapshots skip symlinks and enforce 16 MiB per file and 64 MiB per workspace limits.

Acceptance sensors are frozen with a SHA-256 `SensorLock` before execution. The demo and pilot also run every sensor against an empty baseline and refuse a sensor that already passes, which catches checks written against the implementation instead of the contract.

The demo and pilot execute acceptance sensors as Python scripts through `harness-sensors`. Scripts are materialized outside the task workspace, made read-only, checked against their locked source before every run, and time out after 60 seconds. They receive `HARNESS_WORKSPACE`, `HARNESS_SENSOR_ID` and `HARNESS_CRITERION_ID`; exit code 0 means pass. The runner clears the inherited environment and passes only a minimal allowlist. Set `HARNESS_SENSOR_SANDBOX=auto` or `required` to use `sandbox-exec` on macOS, which denies network and writes outside the workspace; Linux sandboxing is not implemented yet.

Start an interactive understanding and contract approval flow for the current project:

```sh
cargo run -p harness-cli -- run --request "Uma ferramenta de linha de comando pra organizar minha pasta de downloads."
```

The analyst inspects the current directory read-only, asks blocking questions in the terminal, prepares a contract, and revises it after a rejection. On approval, the command saves the understanding, contract, and sensor suite under `.harness/runs/`.

It then asks the acceptance author for executable Python sensors, validates them against the approved suite, and runs them against the current workspace. Every sensor must fail before implementation; a sensor that already passes is treated as suspect and sent back for correction. Script sources are kept outside the project under `HARNESS_STATE_DIR` (default `~/.local/state/harness`); `.harness/runs/<run_id>/sensor_manifest.json` records the locked checksums.

After the sensors are locked, the planner produces a versioned plan. The executor implements it with `workspace-write` in an isolated copy under `HARNESS_STATE_DIR/runs/<run_id>/workspace`, and the verifier checks that copy with `read-only`. Visible sensors guide retries; hidden sensors run only during verification. The run stops at `awaiting_merge` and writes `plan.json`, `step_reports.json`, `sensor_results.json`, `verdict.json` and `delivery_report.json` under `.harness/runs/<run_id>/`.

If the run is approved, only the changed files are copied back into the project for review and merge. The apply is transactional: originals are backed up and restored if any file fails. If the run is rejected, the project is left untouched and the execution copy is preserved for inspection. Plan steps run in dependency order, each with the criteria and sensors it covers; the integration step validates the full contract. Each run writes `events.jsonl`, and `harness state prune --older-than-days <N>` removes old run directories.

`harness run --job-02` is the adversarial benchmark: a shortcut passes the visible step but fails the hidden sensors in the integration step. The roadmap for all rounds is in `docs/roadmap.md`.

Set `HARNESS_ACCEPTANCE_MODEL` to choose the acceptance author model, `HARNESS_PLANNER_MODEL` for the planner, and `HARNESS_VERIFIER_MODEL` for the verifier. With Codex, each falls back to `HARNESS_CODEX_MODEL` or the CLI default; with Claude, to `HARNESS_CLAUDE_MODEL` or the CLI default. Set `HARNESS_CLAUDE_BIN` to override the Claude executable.
