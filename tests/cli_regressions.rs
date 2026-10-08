use bimap::control::channel_from_tls_stream;
use bimap::control::msg::Message;
use bimap::control::tls::{generate_ephemeral_cert, make_tls_acceptor, server_tls_accept};
use std::process::Command;
use std::time::Duration;
use tokio::net::TcpListener;

#[test]
fn cli_tcp_invalid_port_ranges_report_config_error_before_connecting() {
    for specification in [
        "tcp/18449-18448",
        "tcp/1-99999",
        "tcp/65536",
        "tcp/not-a-port",
        "tcp/1-nope",
        "tcp/1-",
        "tcp/-1",
        "tcp/1-2-3",
        "tcp/",
        "tcp/any",
        "udp/icmp",
        "sctp/1234",
        "tcp",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_bimap"))
            .args([
                "client",
                "--control-server",
                "127.0.0.1:19999",
                "--test",
                "open",
                "--port-range",
                specification,
            ])
            .output()
            .expect("run client");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{specification}: {stderr}");
        assert!(stderr.contains("invalid port-range"), "{stderr}");
        assert!(!stderr.contains("cannot connect"), "{stderr}");
    }
}

#[tokio::test]
async fn cli_tls_spoofed_hello_fingerprint_rejected_before_configure(
) -> Result<(), Box<dyn std::error::Error>> {
    let expected = format!("SHA256:{}", "0".repeat(64));
    let (certificate, key, actual) = generate_ephemeral_cert()?;
    assert_ne!(actual, expected);
    let acceptor = make_tls_acceptor(certificate, key)?;
    let listener = TcpListener::bind("127.0.0.1:17980").await?;
    let server_address = listener.local_addr()?;
    let advertised = expected.clone();
    let server = tokio::spawn(async move {
        let connection = match server_tls_accept(&acceptor, &listener).await {
            Ok((connection, _)) => connection,
            Err(_) => return Ok::<_, String>(false),
        };
        let mut channel = channel_from_tls_stream(connection, 0);
        if channel
            .send(&Message::Hello {
                version: 1,
                fingerprint: advertised,
            })
            .await
            .is_err()
        {
            return Ok(false);
        }
        if matches!(channel.recv().await, Ok(Message::Configure { .. })) {
            channel
                .send(&Message::Ack {
                    ok: false,
                    message: Some("spoof accepted".to_string()),
                })
                .await?;
            return Ok(true);
        }
        Ok(false)
    });
    let mut client = tokio::process::Command::new(env!("CARGO_BIN_EXE_bimap"));
    client.kill_on_drop(true).args([
        "client",
        "--control-server",
        &server_address.to_string(),
        "--fingerprint",
        &expected,
        "--test",
        "open",
        "--port-range",
        "tcp/17981",
    ]);
    let output = tokio::time::timeout(Duration::from_secs(5), client.output()).await??;
    assert_eq!(output.status.code(), Some(3));
    assert!(!tokio::time::timeout(Duration::from_secs(2), server).await???,
        "client must reject actual certificate before sending Configure, even when Hello advertises trusted hash");
    Ok(())
}
