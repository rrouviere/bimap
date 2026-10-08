use bimap::control::{channel_from_client_tls, tls::*};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
type TestResult = Result<(), Box<dyn std::error::Error>>;
#[tokio::test]
async fn control_tls_fragmented_message_survives_cancel() -> TestResult {
    let (certificate, key, _) = generate_ephemeral_cert()?;
    let acceptor = make_tls_acceptor(certificate, key)?;
    let listener = TcpListener::bind("127.0.0.1:17981").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut connection, _) = server_tls_accept(&acceptor, &listener).await?;
        connection
            .get_ref()
            .0
            .set_nodelay(true)
            .map_err(|e| e.to_string())?;
        connection
            .write_all(b"{\"type\":")
            .await
            .map_err(|e| e.to_string())?;
        connection.flush().await.map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        connection
            .write_all(b"\"done\"}\n")
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    });
    let connection = client_tls_connect(&make_tls_connector()?, address).await?;
    let mut channel = channel_from_client_tls(connection, 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), channel.recv())
            .await
            .is_err()
    );
    assert!(matches!(
        channel.recv().await?,
        bimap::control::msg::Message::Done
    ));
    server.await??;
    Ok(())
}

#[tokio::test]
async fn control_tls_oversized_unterminated_frame_rejected() -> TestResult {
    let (certificate, key, _) = generate_ephemeral_cert()?;
    let acceptor = make_tls_acceptor(certificate, key)?;
    let listener = TcpListener::bind("127.0.0.1:17982").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut connection, _) = server_tls_accept(&acceptor, &listener).await?;
        connection
            .get_ref()
            .0
            .set_nodelay(true)
            .map_err(|e| e.to_string())?;
        connection
            .write_all(&vec![b' '; 1024 * 1024 + 1])
            .await
            .map_err(|e| e.to_string())?;
        connection.flush().await.map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok::<_, String>(())
    });
    let connection = client_tls_connect(&make_tls_connector()?, address).await?;
    let mut channel = channel_from_client_tls(connection, 0);
    let result = tokio::time::timeout(Duration::from_secs(1), channel.recv()).await?;
    assert!(result.is_err_and(|error| error.contains("frame too large")));
    server.await??;
    Ok(())
}

#[tokio::test]
async fn control_tls_matching_certificate_pin_accepts() -> TestResult {
    let (certificate, key, fingerprint) = generate_ephemeral_cert()?;
    let acceptor = make_tls_acceptor(certificate, key)?;
    let listener = TcpListener::bind("127.0.0.1:17983").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut connection, _) = server_tls_accept(&acceptor, &listener).await?;
        connection
            .write_all(b"{\"type\":\"done\"}\n")
            .await
            .map_err(|e| e.to_string())?;
        connection.flush().await.map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    });
    let unprefixed_uppercase = fingerprint
        .trim_start_matches("SHA256:")
        .to_ascii_uppercase();
    let connection = client_tls_connect(
        &make_pinned_tls_connector(Some(&unprefixed_uppercase))?,
        address,
    )
    .await?;
    let mut channel = channel_from_client_tls(connection, 0);
    assert!(matches!(
        channel.recv().await?,
        bimap::control::msg::Message::Done
    ));
    server.await??;
    Ok(())
}

#[derive(Debug)]
struct InvalidSigningKey(std::sync::Arc<dyn rustls::sign::SigningKey>);

impl rustls::sign::SigningKey for InvalidSigningKey {
    fn choose_scheme(
        &self,
        offered: &[rustls::SignatureScheme],
    ) -> Option<Box<dyn rustls::sign::Signer>> {
        self.0
            .choose_scheme(offered)
            .map(|signer| Box::new(InvalidSigner(signer)) as Box<dyn rustls::sign::Signer>)
    }
    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        self.0.algorithm()
    }
}

#[derive(Debug)]
struct InvalidSigner(Box<dyn rustls::sign::Signer>);

impl rustls::sign::Signer for InvalidSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        let mut signature = self.0.sign(message)?;
        if let Some(byte) = signature.last_mut() {
            *byte ^= 1;
        }
        Ok(signature)
    }
    fn scheme(&self) -> rustls::SignatureScheme {
        self.0.scheme()
    }
}

async fn invalid_signature_rejected(
    version: &'static rustls::SupportedProtocolVersion,
    port: u16,
    probe: bool,
) -> TestResult {
    use std::sync::Arc;
    let (certificate, key, _) = generate_ephemeral_cert()?;
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let signing_key = provider.key_provider.load_private_key(key)?;
    let certified_key = rustls::sign::CertifiedKey::new(
        vec![certificate],
        Arc::new(InvalidSigningKey(signing_key)),
    );
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[version])?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(
            certified_key,
        )));
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut connection, _) = server_tls_accept(&acceptor, &listener).await?;
        if probe {
            let mut payload = [0; 1024];
            connection
                .read_exact(&mut payload)
                .await
                .map_err(|error| error.to_string())?;
            connection
                .write_all(&payload)
                .await
                .map_err(|error| error.to_string())?;
            connection
                .flush()
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok::<_, String>(())
    });
    if probe {
        use bimap::test::{Direction, TestContext, TestProtocol, Transport};
        let result = bimap::test::tls_test::TlsTest
            .run(TestContext {
                direction: Direction::ClientToServer,
                transport: Transport::Tcp,
                target_addr: address,
                timeout: Duration::from_secs(1),
            })
            .await;
        assert!(
            matches!(result, bimap::orchestrator::ProtocolResult::Fail { reason, .. } if reason.starts_with("tls-handshake:")),
            "forged handshake must fail before payload"
        );
    } else {
        let result = client_tls_connect(&make_tls_connector()?, address).await;
        assert!(result.is_err(), "forged handshake signature accepted");
    }
    assert!(server.await?.is_err());
    Ok(())
}

#[tokio::test]
async fn control_tls12_invalid_handshake_signature_rejected() -> TestResult {
    invalid_signature_rejected(&rustls::version::TLS12, 17984, false).await
}

#[tokio::test]
async fn control_tls13_invalid_handshake_signature_rejected() -> TestResult {
    invalid_signature_rejected(&rustls::version::TLS13, 17985, false).await
}

#[tokio::test]
async fn probe_tls13_invalid_handshake_signature_rejected() -> TestResult {
    invalid_signature_rejected(&rustls::version::TLS13, 17986, true).await
}

#[tokio::test]
async fn probe_tls12_invalid_handshake_signature_rejected() -> TestResult {
    invalid_signature_rejected(&rustls::version::TLS12, 17987, true).await
}
