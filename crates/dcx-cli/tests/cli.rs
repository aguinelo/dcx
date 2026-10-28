use std::process::Command;

#[test]
fn states_command_lists_mvp_states() {
    let output = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .arg("states")
        .output()
        .expect("dcx CLI deve iniciar");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("saída UTF-8");
    assert!(stdout.lines().any(|line| line == "understanding"));
    assert!(stdout.lines().any(|line| line == "verifying"));
    assert!(stdout.lines().any(|line| line == "awaiting_merge"));
}

#[test]
fn demo_command_runs_the_vertical_smoke_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args(["run", "--demo"])
        .output()
        .expect("dcx CLI deve iniciar");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("saída UTF-8");
    assert!(stdout.contains("\"criteria_passed\": 2"));
    assert!(stdout.contains("\"hidden-quoted-fields\""));
    assert!(stdout.contains("\"done\""));
}

#[test]
fn checked_in_schemas_match_rust_artifacts() {
    let output = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args(["schemas", "--check"])
        .current_dir(env!("CARGO_MANIFEST_DIR").to_owned() + "/../..")
        .output()
        .expect("dcx CLI deve iniciar");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn parallel_demo_runs_independent_steps_and_shows_the_merge() {
    let output = Command::new(env!("CARGO_BIN_EXE_dcx"))
        .args(["run", "--demo-parallel"])
        .output()
        .expect("dcx CLI deve iniciar");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("saída UTF-8");
    assert!(stdout.contains("Executando passo one"));
    assert!(stdout.contains("Executando passo two"));
    assert!(stdout.contains("Os dois passos rodaram em paralelo"));
    assert!(stdout.contains("reuniu os 3 arquivos"));
}
