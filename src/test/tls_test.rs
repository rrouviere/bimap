use crate::orchestrator::ProtocolResult;
use crate::test::{Direction, Layer, TestContext, TestProtocol, Transport};
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, trace};

use crate::control::tls::generate_ephemeral_cert;
use crate::test::port::compute_sha256;

const KB: usize = 1024;

fn kb_payload() -> Vec<u8> {
    let mut data = Vec::with_capacity(KB);
    for i in 0u8..=255 {
        for _ in 0..4 {
            data.push(i);
        }
        if data.len() >= KB {
            break;
        }
    }
    data.truncate(KB);
    data
}

#[derive(Debug)]
struct NoVerify;

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

async fn tls_initiator(target: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());

    let config = match rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
    {
        Ok(c) => c,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("client config: {e}"),
            };
        }
    };

    let config = config
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth();

    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let domain = match rustls::pki_types::ServerName::try_from("localhost") {
        Ok(d) => d,
        Err(_) => {
            return ProtocolResult::Error {
                reason: "bad server name".into(),
            };
        }
    };

    let mut last_err = String::new();
    let mut tcp_stream = None;
    for attempt in 0..20 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        debug!("tls connecting to {}:{}", target.ip(), target.port());
        match tokio::time::timeout(timeout, TcpStream::connect(target)).await {
            Ok(Ok(s)) => {
                tcp_stream = Some(s);
                break;
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                last_err = "refused".into();
            }
            Ok(Err(e)) => {
                return ProtocolResult::Fail {
                    reason: format!("connect: {e}"),
                    sent_bytes: 0,
                    received_bytes: 0,
                };
            }
            Err(_) => {
                last_err = "timeout".into();
            }
        }
    }
    let tcp_stream = match tcp_stream {
        Some(s) => s,
        None => {
            return ProtocolResult::Fail {
                reason: last_err,
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
    };

    debug!("tls starting handshake with localhost");
    let mut tls_stream =
        match tokio::time::timeout(timeout, connector.connect(domain, tcp_stream)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                return ProtocolResult::Fail {
                    reason: format!("tls-handshake: {e}"),
                    sent_bytes: 0,
                    received_bytes: 0,
                };
            }
            Err(_) => {
                return ProtocolResult::Fail {
                    reason: "tls-handshake: timeout".into(),
                    sent_bytes: 0,
                    received_bytes: 0,
                };
            }
        };

    let payload = kb_payload();

    debug!("tls sending {} bytes", payload.len());
    match tokio::time::timeout(timeout, tls_stream.write_all(&payload)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("write: {e}"),
                sent_bytes: payload.len() as u64,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "write: timeout".into(),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
    }

    let mut buf = vec![0u8; KB];
    trace!("tls waiting for {} bytes", KB);
    match tokio::time::timeout(timeout, tls_stream.read_exact(&mut buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("read: {e}"),
                sent_bytes: payload.len() as u64,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "read: timeout".into(),
                sent_bytes: payload.len() as u64,
                received_bytes: 0,
            };
        }
    }

    let sent_hash = compute_sha256(&payload);
    let recv_hash = compute_sha256(&buf);
    if sent_hash == recv_hash {
        ProtocolResult::Pass {
            sent_bytes: KB as u64,
            received_bytes: KB as u64,
        }
    } else {
        ProtocolResult::Fail {
            reason: "mismatch".into(),
            sent_bytes: KB as u64,
            received_bytes: KB as u64,
        }
    }
}

async fn tls_target(port: u16, timeout: std::time::Duration) -> ProtocolResult {
    let (cert_der, key_der, _fingerprint) = match generate_ephemeral_cert() {
        Ok(c) => c,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("cert: {e}"),
            };
        }
    };

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = match rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
    {
        Ok(c) => c,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("server config: {e}"),
            };
        }
    };

    let config = match config
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
    {
        Ok(c) => c,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("cert config: {e}"),
            };
        }
    };

    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let bind_addr = format!("0.0.0.0:{port}");
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("bind: {e}"),
            };
        }
    };

    debug!("tls waiting for connection on port {}", port);
    let (tcp_stream, _) = match tokio::time::timeout(timeout, listener.accept()).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("accept: {e}"),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "timeout".into(),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
    };

    debug!("tls starting handshake");
    let mut tls_stream = match tokio::time::timeout(timeout, acceptor.accept(tcp_stream)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("tls-handshake: {e}"),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "tls-handshake: timeout".into(),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
    };

    let mut buf = vec![0u8; KB];
    trace!("tls waiting for {} bytes", KB);
    match tokio::time::timeout(timeout, tls_stream.read_exact(&mut buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("read: {e}"),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "read: timeout".into(),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
    }

    debug!("tls sending {} bytes", buf.len());
    match tokio::time::timeout(timeout, tls_stream.write_all(&buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("write: {e}"),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "write: timeout".into(),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
    }

    ProtocolResult::Pass {
        sent_bytes: KB as u64,
        received_bytes: KB as u64,
    }
}

pub struct TlsTest;

#[async_trait]
impl TestProtocol for TlsTest {
    fn name(&self) -> &'static str {
        "tls"
    }

    fn layer(&self) -> Layer {
        Layer::L7
    }

    fn transports(&self) -> &[Transport] {
        &[Transport::Tcp]
    }

    async fn run(&self, ctx: TestContext) -> ProtocolResult {
        match ctx.direction {
            Direction::ClientToServer => tls_initiator(ctx.target_addr, ctx.timeout).await,
            Direction::ServerToClient => tls_target(ctx.port, ctx.timeout).await,
        }
    }
}
