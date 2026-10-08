use async_trait::async_trait;
use bimap::control::tls::{
    client_tls_connect, generate_ephemeral_cert, make_tls_acceptor, make_tls_connector,
    server_tls_accept,
};
use bimap::control::{
    channel_from_client_tls, channel_from_tls_stream,
    msg::{Message, PROTOCOL_VERSION},
    ControlChannel,
};
use bimap::orchestrator;
use bimap::orchestrator::ProtocolResult;
use bimap::test::{build_registry, Layer, TestContext, TestProtocol, TestRegistry, Transport};
use tokio::net::TcpListener;

async fn setup_both_channels(port: u16) -> (ControlChannel, ControlChannel, String) {
    let (cert, key, fingerprint) = generate_ephemeral_cert().expect("cert generation");
    let acceptor = make_tls_acceptor(cert, key).expect("acceptor");
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("bind");

    let server_handle = tokio::spawn(async move {
        let (tls_stream, _) = server_tls_accept(&acceptor, &listener)
            .await
            .expect("accept");
        channel_from_tls_stream(tls_stream, 0)
    });

    let connector = make_tls_connector().expect("connector");
    let control_target: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    let client_tls = client_tls_connect(&connector, control_target)
        .await
        .expect("connect");
    let client_channel = channel_from_client_tls(client_tls, 0);
    let server_channel = server_handle.await.expect("server spawn");

    (server_channel, client_channel, fingerprint)
}

#[tokio::test]
async fn hello_roundtrip() {
    let (mut server, mut client, fingerprint) = setup_both_channels(16001).await;

    let hello = Message::Hello {
        version: PROTOCOL_VERSION,
        fingerprint: fingerprint.clone(),
    };
    server.send(&hello).await.expect("send hello");
    let received = client.recv().await.expect("recv hello");
    match received {
        Message::Hello {
            version,
            fingerprint: fp,
        } => {
            assert_eq!(version, PROTOCOL_VERSION);
            assert!(!fp.is_empty());
        }
        _ => panic!("expected hello"),
    }
}

#[tokio::test]
async fn configure_ack_roundtrip() {
    let (mut server, mut client, _) = setup_both_channels(16002).await;

    let hello = Message::Hello {
        version: PROTOCOL_VERSION,
        fingerprint: "test".into(),
    };
    server.send(&hello).await.expect("send hello");
    assert!(matches!(
        client.recv().await.expect("hello"),
        Message::Hello { .. }
    ));

    let configure = Message::Configure {
        target: None,
        timeout_ms: 500,
        client_version: bimap::control::msg::PROTOCOL_VERSION,
        parallel: 1,
    };
    client.send(&configure).await.expect("send configure");

    let msg = server.recv().await.expect("recv");
    assert!(matches!(msg, Message::Configure { .. }));
    server
        .send(&Message::Ack {
            ok: true,
            message: None,
        })
        .await
        .expect("send ack");
    assert!(matches!(
        client.recv().await.expect("ack"),
        Message::Ack { ok: true, .. }
    ));
}

#[tokio::test]
async fn full_open_test_loopback() {
    let (mut server, mut client, _) = setup_both_channels(16010).await;

    server
        .send(&Message::Hello {
            version: PROTOCOL_VERSION,
            fingerprint: "test".into(),
        })
        .await
        .expect("send hello");

    // Client must consume the Hello before run_client
    let msg = client.recv().await.expect("recv hello");
    assert!(matches!(msg, Message::Hello { .. }));

    // Start server in background, give it time to be ready
    let registry = build_registry();
    let server_handle =
        tokio::spawn(async move { orchestrator::run_server(server, &registry).await });

    // Small delay to let server start processing
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let registry = build_registry();
    let config = orchestrator::ClientConfig {
        tests: vec!["open".to_string()],
        port_ranges: vec![("tcp".to_string(), 25000, 25000)],
        bidir: false,
        timeout_ms: 5000,
        parallel: 1,
        target_addr: "127.0.0.1:0".parse().expect("target address"),
        json: false,
        json_export: false,
        quiet: false,
    };
    let client_summary = orchestrator::run_client(client, &registry, &config)
        .await
        .expect("client run");

    assert!(client_summary.passed > 0);

    let server_summary = server_handle
        .await
        .expect("server join")
        .expect("server run");
    assert!(server_summary.passed > 0);
}

#[tokio::test]
async fn server_rejects_unknown_protocol() {
    let (server, mut client, _) = setup_both_channels(16011).await;
    let registry = build_registry();
    let server_handle =
        tokio::spawn(async move { orchestrator::run_server(server, &registry).await });
    client
        .send(&Message::Configure {
            target: None,
            timeout_ms: 500,
            client_version: PROTOCOL_VERSION,
            parallel: 1,
        })
        .await
        .expect("configure");
    assert!(matches!(
        client.recv().await.expect("ack"),
        Message::Ack { ok: true, .. }
    ));
    client
        .send(&Message::Test {
            id: 0,
            protocol: "nonexistent".into(),
            transport: "tcp".into(),
            port: 10000,
            direction: "->".into(),
        })
        .await
        .expect("unknown test");
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), server_handle)
        .await
        .expect("server must reject promptly")
        .expect("server task")
        .expect_err("unknown protocol must be rejected");
    assert!(error.contains("unknown protocol: nonexistent"), "{error}");
}

struct MarkedPass {
    name: &'static str,
    marker: u64,
}

#[tokio::test]
async fn control_client_legacy_protocol_rejected() {
    let (server, mut client, _) = setup_both_channels(16013).await;
    let registry = build_registry();
    let server_task =
        tokio::spawn(async move { orchestrator::run_server(server, &registry).await });
    client
        .send(&Message::Configure {
            target: None,
            timeout_ms: 500,
            client_version: 2,
            parallel: 1,
        })
        .await
        .expect("configure");
    assert!(matches!(
        client.recv().await.expect("rejection"),
        Message::Ack { ok: false, message: Some(message) } if message.contains("protocol version 2")
    ));
    assert!(server_task.await.expect("server task").is_err());
}

#[async_trait]
impl TestProtocol for MarkedPass {
    fn name(&self) -> &'static str {
        self.name
    }

    fn layer(&self) -> Layer {
        Layer::L4
    }

    fn transports(&self) -> &[Transport] {
        &[Transport::Tcp]
    }

    async fn run(&self, _context: TestContext) -> ProtocolResult {
        ProtocolResult::Pass {
            sent_bytes: self.marker,
            received_bytes: 0,
        }
    }
}

#[tokio::test]
async fn server_runs_the_protocol_named_by_each_batched_test() {
    let (server, mut client, _) = setup_both_channels(16012).await;
    let mut registry = TestRegistry::new();
    registry.register(Box::new(MarkedPass {
        name: "alpha",
        marker: 11,
    }));
    registry.register(Box::new(MarkedPass {
        name: "beta",
        marker: 22,
    }));
    let server_task =
        tokio::spawn(async move { orchestrator::run_server(server, &registry).await });

    client
        .send(&Message::Configure {
            target: Some("127.0.0.1:0".into()),
            timeout_ms: 500,
            client_version: bimap::control::msg::PROTOCOL_VERSION,
            parallel: 2,
        })
        .await
        .expect("send configure");
    assert!(matches!(
        client.recv().await.expect("ack"),
        Message::Ack { ok: true, .. }
    ));

    for (id, protocol) in [(1, "alpha"), (2, "beta")] {
        client
            .send(&Message::Test {
                id,
                protocol: protocol.into(),
                transport: "tcp".into(),
                port: 0,
                direction: "->".into(),
            })
            .await
            .expect("send test");
    }
    client.send(&Message::Done).await.expect("send done");

    let mut reports = std::collections::HashMap::new();
    for _ in 0..2 {
        if let Message::Report {
            id,
            sent: Some(report),
            ..
        } = client.recv().await.expect("report")
        {
            reports.insert(id, report.bytes);
        }
    }
    assert_eq!(reports.get(&1), Some(&11));
    assert_eq!(reports.get(&2), Some(&22));
    assert!(matches!(
        client.recv().await.expect("bye"),
        Message::Bye { .. }
    ));
    server_task
        .await
        .expect("server task")
        .expect("server result");
}
