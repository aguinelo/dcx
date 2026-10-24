# Harness

CLI for running software tasks through a contract, acceptance sensors, an isolated executor, and an independent verifier.

The MVP is being built in milestones. The first target is the vertical `cli-01` flow described in `docs/milestones/m0.md`.

## Development

The core and CLI are written in Rust. The pilot tasks and their project preset use Python. User-facing documentation and messages use pt-BR; identifiers and code follow the English names defined by the specification.

```sh
cargo build --workspace
cargo test --workspace
cargo run -p harness-cli -- schemas --check
cargo run -p harness-cli -- run --demo
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

The pilot currently targets the local Codex CLI adapter. Other providers remain future adapters behind the engine role interfaces.

The engine derives `step_report.files_changed` from workspace snapshots around each executor attempt, instead of trusting the executor's own list. Snapshots skip symlinks and enforce 16 MiB per file and 64 MiB per workspace limits.

Acceptance sensors are frozen with a SHA-256 `SensorLock` before execution. The demo and pilot also run every sensor against an empty baseline and refuse a sensor that already passes, which catches checks written against the implementation instead of the contract.

Start an interactive understanding and contract approval flow for the current project:

```sh
cargo run -p harness-cli -- run --request "Uma ferramenta de linha de comando pra organizar minha pasta de downloads."
```

The analyst inspects the current directory read-only, asks blocking questions in the terminal, prepares a contract, and revises it after a rejection. On approval, the command saves the understanding, contract, and sensor suite under `.harness/runs/`. M1 stops at approval; task execution for arbitrary requests is not wired yet.
