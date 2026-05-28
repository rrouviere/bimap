use bimap::control::tls::{
    client_tls_connect, generate_ephemeral_cert, make_tls_acceptor, make_tls_connector,
    server_tls_accept,
};
use bimap::control::{
    channel_from_client_tls, channel_from_tls_stream, msg::Message, ControlChannel,
};
use bimap::orchestrator;
use bimap::test::build_registry;
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
    let client_tls = client_tls_connect(&connector, &format!("127.0.0.1:{port}"))
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
        version: 1,
        fingerprint: fingerprint.clone(),
    };
    server.send(&hello).await.expect("send hello");
    let received = client.recv().await.expect("recv hello");
    match received {
        Message::Hello {
            version,
            fingerprint: fp,
        } => {
            assert_eq!(version, 1);
            assert!(!fp.is_empty());
        }
        _ => panic!("expected hello"),
    }
}

#[tokio::test]
async fn configure_ack_roundtrip() {
    let (mut server, mut client, _) = setup_both_channels(16002).await;

    let hello = Message::Hello {
        version: 1,
        fingerprint: "test".into(),
    };
    server.send(&hello).await.expect("send hello");

    let configure = Message::Configure {
        tests: vec!["open".into()],
        port_ranges: vec![bimap::control::msg::PortRangeSpec {
            transport: "tcp".into(),
            start: 10000,
            end: 10001,
        }],
        bidir: false,
        target: None,
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
}

#[tokio::test]
async fn full_open_test_loopback() {
    let (mut server, mut client, _) = setup_both_channels(16010).await;

    server
        .send(&Message::Hello {
            version: 1,
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
        tokio::spawn(async move { orchestrator::run_server(server, &registry, 5000).await });

    // Small delay to let server start processing
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let registry = build_registry();
    let config = orchestrator::ClientConfig {
        tests: vec!["open".to_string()],
        port_ranges: vec![("tcp".to_string(), 25000, 25000)],
        bidir: false,
        timeout_ms: 5000,
        server_addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        target_addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        json: false,
        json_export: false,
        verbose: false,
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
    let (mut server, mut client, _) = setup_both_channels(16011).await;

    server
        .send(&Message::Hello {
            version: 1,
            fingerprint: "test".into(),
        })
        .await
        .expect("send hello");

    client
        .send(&Message::Configure {
            tests: vec!["open".into()],
            port_ranges: vec![bimap::control::msg::PortRangeSpec {
                transport: "tcp".into(),
                start: 30000,
                end: 30000,
            }],
            bidir: false,
            target: None,
        })
        .await
        .expect("send configure");

    // Server should process the configure + test without hanging
    let registry = build_registry();
    let server_handle =
        tokio::spawn(async move { orchestrator::run_server(server, &registry, 5000).await });

    // Give server time to process configure, ack, process test, send report
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    // Server should complete its loop when we send Done
    client.send(&Message::Done).await.expect("send done");

    let result = server_handle.await.expect("server join");
    assert!(result.is_ok(), "server should complete without error");
}
