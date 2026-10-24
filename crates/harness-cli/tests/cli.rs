use std::process::Command;

#[test]
fn states_command_lists_mvp_states() {
    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg("states")
        .output()
        .expect("harness CLI deve iniciar");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("saída UTF-8");
    assert!(stdout.lines().any(|line| line == "understanding"));
    assert!(stdout.lines().any(|line| line == "verifying"));
    assert!(stdout.lines().any(|line| line == "awaiting_merge"));
}

#[test]
fn demo_command_runs_the_vertical_smoke_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["run", "--demo"])
        .output()
        .expect("harness CLI deve iniciar");

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
