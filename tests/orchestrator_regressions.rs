use std::process::{Command, Stdio};

fn scan(control_port: u16, test_port: u16, target: &str, bidir: bool) -> serde_json::Value {
    let binary = env!("CARGO_BIN_EXE_bimap");
    let mut server = Command::new(binary)
        .args(["server", "--bind", &format!("127.0.0.1:{control_port}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("server");
    std::thread::sleep(std::time::Duration::from_millis(200));
    let mut client = Command::new(binary);
    client.args([
        "client",
        "--server",
        "127.0.0.1",
        "--port",
        &control_port.to_string(),
        "--target",
        target,
        "--test",
        "open",
        "--port-range",
        &format!("tcp/{test_port}"),
        "--parallel",
        "4",
        "--timeout",
        "500",
        "--json-export",
    ]);
    if bidir {
        client.arg("--bidir");
    }
    let output = client.output().expect("client");
    server.kill().expect("stop server");
    server.wait().expect("reap server");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("json export")
}

#[test]
fn open_tcp_parallel_export_preserves_results() {
    let report = scan(17771, 17772, "127.0.0.1", false);
    assert_eq!(report["summary"]["passed"], 1);
    assert_eq!(report["results"].as_array().expect("results").len(), 1);
}

#[test]
fn open_tcp_parallel_bidir_loopback_passes_both_directions() {
    let report = scan(17773, 17774, "127.0.0.1", true);
    assert_eq!(report["summary"]["passed"], 2);
    assert_eq!(report["results"].as_array().expect("results").len(), 2);
}

#[test]
fn open_tcp_reverse_uses_client_address_when_target_differs() {
    let report = scan(17775, 17776, "127.0.0.2", true);
    assert_eq!(report["summary"]["passed"], 2);
    assert_eq!(report["results"].as_array().expect("results").len(), 2);
}

use async_trait::async_trait;
use bimap::control::tls::{
    client_tls_connect, generate_ephemeral_cert, make_tls_acceptor, make_tls_connector,
    server_tls_accept,
};
use bimap::control::{
    channel_from_client_tls, channel_from_tls_stream, msg::Message, ControlChannel,
};
use bimap::orchestrator::{run_client, ClientConfig, ProtocolResult};
use bimap::test::{Layer, TestContext, TestProtocol, TestRegistry, Transport};

struct LocalPass;

#[async_trait]
impl TestProtocol for LocalPass {
    fn name(&self) -> &'static str {
        "local-pass"
    }
    fn layer(&self) -> Layer {
        Layer::L4
    }
    fn transports(&self) -> &[Transport] {
        &[Transport::Tcp]
    }
    async fn run(&self, _context: TestContext) -> ProtocolResult {
        ProtocolResult::Pass {
            sent_bytes: 1,
            received_bytes: 1,
        }
    }
}

async fn channels() -> (ControlChannel, ControlChannel) {
    let (certificate, key, _) = generate_ephemeral_cert().expect("certificate");
    let acceptor = make_tls_acceptor(certificate, key).expect("acceptor");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (stream, _) = server_tls_accept(&acceptor, &listener)
            .await
            .expect("TLS accept");
        channel_from_tls_stream(stream, 0)
    });
    let connector = make_tls_connector().expect("connector");
    let stream = client_tls_connect(&connector, address)
        .await
        .expect("TLS connect");
    (
        server.await.expect("server"),
        channel_from_client_tls(stream, 0),
    )
}

fn mock_config() -> ClientConfig {
    ClientConfig {
        tests: vec!["local-pass".into()],
        port_ranges: vec![("tcp".into(), 17800, 17800)],
        bidir: false,
        timeout_ms: 100,
        parallel: 4,
        server_addr: "127.0.0.1".parse().expect("IP"),
        target_addr: "127.0.0.1:0".parse().expect("target address"),
        json: false,
        json_export: true,
        verbose: 0,
        quiet: true,
    }
}

#[tokio::test]
async fn open_tcp_parallel_control_eof_returns_error() {
    let (certificate, key, _) = generate_ephemeral_cert().expect("certificate");
    let acceptor = make_tls_acceptor(certificate, key).expect("acceptor");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server_task = tokio::spawn(async move {
        let (stream, _) = server_tls_accept(&acceptor, &listener)
            .await
            .expect("accept");
        let mut server = channel_from_tls_stream(stream, 0);
        server
            .send(&Message::Hello {
                version: bimap::control::msg::PROTOCOL_VERSION,
                fingerprint: "test".into(),
            })
            .await
            .expect("hello");
        assert!(matches!(
            server.recv().await.expect("configure"),
            Message::Configure { .. }
        ));
        server
            .send(&Message::Ack {
                ok: true,
                message: None,
            })
            .await
            .expect("ack");
        assert!(matches!(
            server.recv().await.expect("test"),
            Message::Test { .. }
        ));
    });
    let mut client = Command::new(env!("CARGO_BIN_EXE_bimap"))
        .args([
            "client",
            "--server",
            "127.0.0.1",
            "--port",
            &address.port().to_string(),
            "--test",
            "open",
            "--port-range",
            "tcp/17801",
            "--timeout",
            "100",
            "--parallel",
            "4",
            "--json-export",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("client");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let status = loop {
        if let Some(status) = client.try_wait().expect("client status") {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            client.kill().expect("kill hung client");
            client.wait().expect("reap client");
            break None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    };
    server_task.await.expect("server task");
    assert_eq!(
        status.expect("client must terminate on EOF").code(),
        Some(3)
    );
}

#[tokio::test]
async fn open_tcp_parallel_server_error_overrides_local_pass() {
    let (mut server, client) = channels().await;
    let server_task = tokio::spawn(async move {
        assert!(matches!(
            server.recv().await.expect("configure"),
            Message::Configure { .. }
        ));
        server
            .send(&Message::Ack {
                ok: true,
                message: None,
            })
            .await
            .expect("ack");
        let Message::Test { id, .. } = server.recv().await.expect("test") else {
            panic!("expected test")
        };
        server
            .send(&Message::Report {
                id,
                sent: None,
                received: None,
                error: Some("listener failed".into()),
            })
            .await
            .expect("report");
        assert!(matches!(server.recv().await.expect("done"), Message::Done));
        server
            .send(&Message::Bye {
                summary: bimap::control::msg::TestSummary {
                    passed: 0,
                    failed: 0,
                    errors: 1,
                },
            })
            .await
            .expect("bye");
    });
    let mut registry = TestRegistry::new();
    registry.register(Box::new(LocalPass));
    let summary = run_client(client, &registry, &mock_config())
        .await
        .expect("run client");
    server_task.await.expect("server task");
    assert_eq!(summary.passed, 0);
    assert_eq!(summary.errors, 1);
}

type RecordedAddresses =
    std::sync::Arc<std::sync::Mutex<Vec<(bimap::test::Direction, std::net::IpAddr)>>>;

struct RecordAddress(RecordedAddresses);

#[async_trait]
impl TestProtocol for RecordAddress {
    fn name(&self) -> &'static str {
        "local-pass"
    }
    fn layer(&self) -> Layer {
        Layer::L4
    }
    fn transports(&self) -> &[Transport] {
        &[Transport::Tcp]
    }
    async fn run(&self, context: TestContext) -> ProtocolResult {
        self.0
            .lock()
            .expect("record context")
            .push((context.direction, context.target_addr.ip()));
        ProtocolResult::Pass {
            sent_bytes: 1,
            received_bytes: 1,
        }
    }
}

#[tokio::test]
async fn open_tcp_reverse_context_uses_control_client_address() {
    let (server, client) = channels().await;
    let server_addresses = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let client_addresses = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut server_registry = TestRegistry::new();
    server_registry.register(Box::new(RecordAddress(server_addresses.clone())));
    let server_task =
        tokio::spawn(
            async move { bimap::orchestrator::run_server(server, &server_registry).await },
        );
    let mut client_registry = TestRegistry::new();
    client_registry.register(Box::new(RecordAddress(client_addresses.clone())));
    let mut config = mock_config();
    config.bidir = true;
    config.target_addr = "127.0.0.2:0".parse().expect("target address");
    run_client(client, &client_registry, &config)
        .await
        .expect("run client");
    server_task.await.expect("server task").expect("run server");
    let client_ip: std::net::IpAddr = "127.0.0.1".parse().expect("client IP");
    let target_ip = config.target_addr.ip();
    assert_eq!(
        *client_addresses.lock().expect("client addresses"),
        vec![
            (bimap::test::Direction::ClientToServer, target_ip),
            (bimap::test::Direction::ServerToClient, client_ip)
        ]
    );
    assert_eq!(
        *server_addresses.lock().expect("server addresses"),
        vec![
            (bimap::test::Direction::ServerToClient, target_ip),
            (bimap::test::Direction::ClientToServer, client_ip)
        ]
    );
}
