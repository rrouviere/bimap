use super::exchange::{accept, connect, echo, kilobyte_payload, perform, roundtrip};
use crate::control::tls::{generate_ephemeral_cert, make_tls_acceptor, make_tls_connector};
use crate::orchestrator::ProtocolResult;
use crate::test::{Direction, Layer, TestContext, TestProtocol, Transport};
use async_trait::async_trait;
use std::net::SocketAddr;
use std::time::Duration;

async fn tls_initiator(
    target: SocketAddr,
    timeout: Duration,
) -> Result<ProtocolResult, ProtocolResult> {
    let connector = make_tls_connector().map_err(|reason| ProtocolResult::Error { reason })?;
    let domain = rustls::pki_types::ServerName::try_from("localhost").map_err(|error| {
        ProtocolResult::Error {
            reason: format!("server name: {error}"),
        }
    })?;
    let connection = connect(target, timeout).await?;
    let mut stream = perform(
        connector.connect(domain, connection),
        timeout,
        "tls-handshake",
        0,
        0,
    )
    .await?;
    Ok(roundtrip(&mut stream, &kilobyte_payload(), timeout).await)
}

async fn tls_target(
    address: SocketAddr,
    timeout: Duration,
) -> Result<ProtocolResult, ProtocolResult> {
    let (certificate, key, _) =
        generate_ephemeral_cert().map_err(|reason| ProtocolResult::Error { reason })?;
    let acceptor =
        make_tls_acceptor(certificate, key).map_err(|reason| ProtocolResult::Error { reason })?;
    let connection = accept(address, timeout).await?;
    let mut stream = perform(acceptor.accept(connection), timeout, "tls-handshake", 0, 0).await?;
    Ok(echo(&mut stream, &kilobyte_payload(), timeout).await)
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
    async fn run(&self, context: TestContext) -> ProtocolResult {
        let result = match context.direction {
            Direction::ClientToServer => tls_initiator(context.target_addr, context.timeout).await,
            Direction::ServerToClient => tls_target(context.target_addr, context.timeout).await,
        };
        match result {
            Ok(result) | Err(result) => result,
        }
    }
}
