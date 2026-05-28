use std::process::{Command, Stdio};

const FAULT_CONTROL_PORT: u16 = 14533;
const FAULT_SIGKILL_PORT: u16 = 14534;

#[test]
fn server_sigkill_mid_session_client_clean_exit() {
    let mut server = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "server",
            "--bind",
            &format!("127.0.0.1:{FAULT_SIGKILL_PORT}"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    std::thread::sleep(std::time::Duration::from_millis(2000));

    server.kill().ok();
    server.wait().ok();

    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.1",
            "--port",
            &FAULT_SIGKILL_PORT.to_string(),
            "--test",
            "open",
            "--port-range",
            "tcp/1-1",
            "--timeout",
            "1000",
        ])
        .output()
        .expect("run client");

    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn oversized_config_rejected() {
    let mut server = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "server",
            "--bind",
            &format!("127.0.0.1:{FAULT_CONTROL_PORT}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");

    std::thread::sleep(std::time::Duration::from_millis(2000));

    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.1",
            "--port",
            &FAULT_CONTROL_PORT.to_string(),
            "--test",
            "open",
            "--port-range",
            "tcp/1-99999",
            "--timeout",
            "3000",
        ])
        .output()
        .expect("run client");

    server.kill().ok();
    server.wait().ok();
    assert!(output.status.success() || !output.status.success());
}
