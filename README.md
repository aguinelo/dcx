# Harness

CLI for running software tasks through a contract, acceptance sensors, an isolated executor, and an independent verifier.

The MVP is being built in milestones. The first target is the vertical `cli-01` flow described in `docs/milestones/m0.md`.

## Development

The core and CLI are written in Rust. The pilot tasks and their project preset use Python. User-facing documentation and messages use pt-BR; identifiers and code follow the English names defined by the specification.

```sh
cargo build --workspace
cargo test --workspace
```

## Project status

This repository is an early MVP scaffold. Model provider, exact artifact schemas, and the first acceptance contract are recorded as open decisions before implementation.
