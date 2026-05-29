use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

const E2E_SERVER_PORT: u16 = 14333;
const E2E_FINGERPRINT_PORT: u16 = 14434;
const E2E_FULL_OPEN_PORT: u16 = 14435;
const E2E_UNKNOWN_PORT: u16 = 14436;
const E2E_TARGET_HOSTNAME_PORT: u16 = 14437;
const E2E_IPV6_CTRL_PORT: u16 = 14438;

#[test]
fn binary_help_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .arg("--help")
        .output()
        .expect("run binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("bimap"));
}

#[test]
fn binary_version_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .arg("--version")
        .output()
        .expect("run binary");
    assert!(output.status.success());
}

#[test]
fn server_prints_fingerprint() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "server",
            "--bind",
            &format!("127.0.0.1:{E2E_FINGERPRINT_PORT}"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    std::thread::sleep(std::time::Duration::from_millis(1000));

    let stderr = BufReader::new(child.stderr.take().unwrap());
    let mut found_fingerprint = false;
    for line in stderr.lines().map_while(Result::ok) {
        if line.contains("fingerprint") {
            found_fingerprint = true;
            break;
        }
    }

    child.kill().ok();
    child.wait().ok();
    assert!(
        found_fingerprint,
        "server should print fingerprint on stderr"
    );
}

#[test]
fn client_no_tests_lists_tests() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args(["client", "--server", "127.0.0.1", "--port-range", "tcp/1-1"])
        .output()
        .expect("run client");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("available tests:"), "stdout: {stdout}");
}

#[test]
fn client_no_port_range_is_config_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args(["client", "--server", "127.0.0.1", "--test", "open"])
        .output()
        .expect("run client");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn client_connection_refused_is_error_3() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.1",
            "--port",
            &E2E_SERVER_PORT.to_string(),
            "--test",
            "open",
            "--port-range",
            "tcp/1-1",
            "--timeout",
            "1000",
        ])
        .output()
        .expect("run client");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.code() == Some(3) || stderr.contains("cannot connect"),
        "exit code: {:?}, stderr: {}",
        output.status.code(),
        stderr
    );
}

#[test]
fn full_open_e2e_loopback() {
    let mut server = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "server",
            "--bind",
            &format!("127.0.0.1:{E2E_FULL_OPEN_PORT}"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    std::thread::sleep(std::time::Duration::from_millis(2000));

    let client_output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.1",
            "--port",
            &E2E_FULL_OPEN_PORT.to_string(),
            "--test",
            "open",
            "--port-range",
            "tcp/35000-35001",
            "--timeout",
            "3000",
        ])
        .output()
        .expect("run client");

    server.kill().ok();
    server.wait().ok();

    let stdout = String::from_utf8_lossy(&client_output.stdout);
    let stderr = String::from_utf8_lossy(&client_output.stderr);

    assert!(
        stdout.contains("PASS") || stderr.contains("passed"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
fn unknown_test_name_exit_1_or_3() {
    let mut server = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args(["server", "--bind", &format!("127.0.0.1:{E2E_UNKNOWN_PORT}")])
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
            &E2E_UNKNOWN_PORT.to_string(),
            "--test",
            "nonexistent",
            "--port-range",
            "tcp/1-1",
            "--timeout",
            "1000",
        ])
        .output()
        .expect("run client");

    server.kill().ok();
    server.wait().ok();
    let code = output.status.code().unwrap_or(-1);
    assert!(
        code == 1 || code == 3,
        "exit code should be 1 or 3, got {code}"
    );
}

#[test]
fn icmp_without_port_range_connects_not_config_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.3",
            "--port",
            "14499",
            "--test",
            "icmp-ping",
            "--timeout",
            "500",
        ])
        .output()
        .expect("run client");
    assert_ne!(output.status.code(), Some(2), "should not be config error");
    assert_eq!(output.status.code(), Some(3), "should be connection error");
}

#[test]
fn icmp_with_wrong_port_range_auto_adds_icmp() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.3",
            "--port",
            "14500",
            "--test",
            "icmp-ping",
            "--port-range",
            "tcp/42",
            "--timeout",
            "500",
        ])
        .output()
        .expect("run client");
    assert_ne!(output.status.code(), Some(2), "should not be config error");
    assert_eq!(output.status.code(), Some(3), "should be connection error");
}

#[test]
fn l4_test_without_port_range_is_config_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.1",
            "--test",
            "1kb",
            "--timeout",
            "500",
        ])
        .output()
        .expect("run client");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn target_hostname_resolves_to_connection_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.1",
            "--port",
            &E2E_TARGET_HOSTNAME_PORT.to_string(),
            "--target",
            "localhost",
            "--test",
            "open",
            "--port-range",
            "tcp/1-1",
            "--timeout",
            "1000",
        ])
        .output()
        .expect("run client");
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(
        code, 3,
        "--target localhost should resolve and attempt connection (got exit {code})"
    );
}

#[test]
fn control_server_ipv6_bracket_notation() {
    let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--control-server",
            &format!("[::1]:{}", E2E_IPV6_CTRL_PORT),
            "--target",
            "127.0.0.1",
            "--test",
            "open",
            "--port-range",
            "tcp/1-1",
            "--timeout",
            "1000",
        ])
        .output()
        .expect("run client");
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(
        code, 3,
        "--control-server [::1]:port should parse IPv6 bracket notation (got exit {code})"
    );
}
