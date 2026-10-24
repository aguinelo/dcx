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

Regenerate versioned JSON Schemas after changing a Rust artifact:

```sh
cargo run -p harness-cli -- schemas --write
```

## Project status

`harness run --demo` exercises a deterministic smoke path with a built-in worker and two acceptance sensors. It does not call a model provider or provide an OS security sandbox, so it is not an evaluation of the MVP thesis and must not be used with untrusted code.

The live model provider and exact artifact schemas remain open decisions before the full vertical slice.
