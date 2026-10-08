mod support;

use std::process::Command;

const E2E_SERVER_PORT: u16 = 14333;
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
fn l4_tests_without_port_ranges_are_config_errors() {
    for protocol in ["open", "1kb"] {
        let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
            .args(["client", "--server", "127.0.0.1", "--test", protocol])
            .output()
            .expect("run client");
        assert_eq!(output.status.code(), Some(2));
    }
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
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("cannot connect"), "{stderr}");
}

#[test]
fn protocols_roundtrip_and_reverse_on_one_server() {
    let server = support::Server::start();
    let tcp = format!("tcp/{}", support::available_port());
    let udp = format!("udp/{}", support::available_port());
    for bidirectional in [false, true] {
        let mut arguments = vec![
            "--test",
            "open",
            "--test",
            "1kb",
            "--test",
            "tls",
            "--test",
            "dns",
            "--port-range",
            &tcp,
            "--port-range",
            &udp,
            "--timeout",
            "3000",
            "--json",
            "--fingerprint",
            &server.fingerprint,
        ];
        if bidirectional {
            arguments.push("--bidir");
        }
        let output = server.client(&arguments).output();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let records: Vec<serde_json::Value> = String::from_utf8(output.stdout)
            .expect("UTF-8 results")
            .lines()
            .map(|line| serde_json::from_str(line).expect("JSON result"))
            .collect();
        let mut observed = std::collections::BTreeSet::new();
        for record in &records {
            assert_eq!(record["status"], "pass", "{record}");
            let protocol = record["protocol"].as_str().expect("protocol");
            let transport = record["transport"].as_str().expect("transport");
            let direction = record["direction"].as_str().expect("direction");
            assert!(
                observed.insert((protocol, transport, direction)),
                "duplicate result: {record}"
            );
            match protocol {
                "open" => {
                    assert_eq!(record["tx"], 1);
                    assert_eq!(record["rx"], 1);
                }
                "1kb" | "tls" => {
                    assert_eq!(record["tx"], 1024);
                    assert_eq!(record["rx"], 1024);
                }
                "dns" => {
                    assert!(record["tx"].as_u64().expect("tx") > 0);
                    assert!(record["rx"].as_u64().expect("rx") > 0);
                }
                _ => panic!("unexpected protocol: {record}"),
            }
        }
        let directions: &[&str] = if bidirectional {
            &["->", "<-"]
        } else {
            &["->"]
        };
        let expected: std::collections::BTreeSet<_> = [
            ("open", "tcp"),
            ("open", "udp"),
            ("1kb", "tcp"),
            ("1kb", "udp"),
            ("tls", "tcp"),
            ("dns", "tcp"),
            ("dns", "udp"),
        ]
        .into_iter()
        .flat_map(|(protocol, transport)| {
            directions
                .iter()
                .map(move |direction| (protocol, transport, *direction))
        })
        .collect();
        assert_eq!(observed, expected);
    }
}

#[test]
fn open_tcp_server_killed_during_scan_exits_connection_error() {
    let mut server = support::Server::start();
    let first_port = support::available_port();
    let ports = format!("tcp/{first_port}-{}", first_port.saturating_add(31));
    let client = server.client(&[
        "--test",
        "open",
        "--port-range",
        &ports,
        "--timeout",
        "3000",
        "--parallel",
        "1",
        "--json",
    ]);
    server.wait_for("waiting for connection on");
    server.process.0.kill().expect("kill active server");
    server.process.0.wait().expect("reap server");
    let output = client.output();
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unknown_test_name_reports_error() {
    let server = support::Server::start();
    let output = server
        .client(&["--test", "nonexistent", "--port-range", "tcp/10000"])
        .output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("unknown protocol: nonexistent"), "{stderr}");
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

#[test]
fn server_and_client_with_hostnames_e2e() {
    let server = support::Server::start_on("localhost");
    let ports = format!("tcp/{}", support::available_port());
    let output = server
        .client(&[
            "--target",
            "localhost",
            "--test",
            "open",
            "--port-range",
            &ports,
            "--timeout",
            "3000",
            "--json",
        ])
        .output();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON result");
    assert_eq!(result["status"], "pass");
    assert_eq!(result["tx"], 1);
    assert_eq!(result["rx"], 1);
}
