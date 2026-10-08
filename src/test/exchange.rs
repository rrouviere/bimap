use crate::orchestrator::ProtocolResult;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::debug;

pub(super) fn kilobyte_payload() -> [u8; 1024] {
    std::array::from_fn(|index| (index / 4) as u8)
}

pub(super) fn failure(reason: impl Into<String>, sent: usize, received: usize) -> ProtocolResult {
    ProtocolResult::Fail {
        reason: reason.into(),
        sent_bytes: sent as u64,
        received_bytes: received as u64,
    }
}

pub(super) async fn perform<T>(
    operation: impl Future<Output = std::io::Result<T>>,
    timeout: Duration,
    stage: &str,
    sent: usize,
    received: usize,
) -> Result<T, ProtocolResult> {
    match tokio::time::timeout(timeout, operation).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(failure(format!("{stage}: {error}"), sent, received)),
        Err(_) => Err(failure(format!("{stage}: timeout"), sent, received)),
    }
}

pub(super) async fn connect(
    target: SocketAddr,
    timeout: Duration,
) -> Result<TcpStream, ProtocolResult> {
    let mut reason = String::new();
    for attempt in 0..20 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        debug!("connecting to {target} (timeout={}ms)", timeout.as_millis());
        match tokio::time::timeout(timeout, TcpStream::connect(target)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                reason = "refused".into();
            }
            Ok(Err(error)) => return Err(failure(format!("connect: {error}"), 0, 0)),
            Err(_) => reason = "timeout".into(),
        }
    }
    Err(failure(reason, 0, 0))
}

pub(super) async fn accept(
    address: SocketAddr,
    timeout: Duration,
) -> Result<TcpStream, ProtocolResult> {
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| ProtocolResult::Error {
            reason: format!("bind: {error}"),
        })?;
    debug!("waiting for connection on {address}");
    let (stream, _) = perform(listener.accept(), timeout, "accept", 0, 0).await?;
    Ok(stream)
}

pub(super) async fn roundtrip<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    payload: &[u8],
    timeout: Duration,
) -> ProtocolResult {
    let length = payload.len();
    if let Err(result) = perform(stream.write_all(payload), timeout, "write", 0, 0).await {
        return result;
    }
    let mut received = vec![0; length];
    if let Err(result) = perform(stream.read_exact(&mut received), timeout, "read", length, 0).await
    {
        return result;
    }
    if received != payload {
        return failure("mismatch", length, length);
    }
    ProtocolResult::Pass {
        sent_bytes: length as u64,
        received_bytes: length as u64,
    }
}

pub(super) async fn echo<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    payload: &[u8],
    timeout: Duration,
) -> ProtocolResult {
    let length = payload.len();
    let mut received = vec![0; length];
    if let Err(result) = perform(stream.read_exact(&mut received), timeout, "read", 0, 0).await {
        return result;
    }
    if received != payload {
        return failure(
            "mismatch: received payload differs from expected data",
            0,
            length,
        );
    }
    if let Err(result) = perform(stream.write_all(&received), timeout, "write", 0, length).await {
        return result;
    }
    ProtocolResult::Pass {
        sent_bytes: length as u64,
        received_bytes: length as u64,
    }
}
