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
