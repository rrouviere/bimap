use super::exchange::{accept, connect, echo, failure, kilobyte_payload, roundtrip};
use crate::orchestrator::ProtocolResult;
use crate::test::{Direction, Layer, TestContext, TestProtocol, Transport};
use async_trait::async_trait;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tracing::debug;

pub struct OpenTest;
pub struct KbTest;

const ONE_BYTE_PAYLOAD: u8 = 0xAA;
#[cfg(test)]
const KB: usize = 1024;

async fn tcp_initiator(target: SocketAddr, payload: &[u8], timeout: Duration) -> ProtocolResult {
    match connect(target, timeout).await {
        Ok(mut stream) => roundtrip(&mut stream, payload, timeout).await,
        Err(result) => result,
    }
}

async fn tcp_target(address: SocketAddr, payload: &[u8], timeout: Duration) -> ProtocolResult {
    match accept(address, timeout).await {
        Ok(mut stream) => echo(&mut stream, payload, timeout).await,
        Err(result) => result,
    }
}

async fn udp_initiator(target: SocketAddr, payload: &[u8], timeout: Duration) -> ProtocolResult {
    let address = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = match UdpSocket::bind(address).await {
        Ok(socket) => socket,
        Err(error) => {
            return ProtocolResult::Error {
                reason: format!("udp bind: {error}"),
            }
        }
    };
    let mut reason = String::new();
    let mut received = vec![0; payload.len() + 1];
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        debug!("udp sending {} bytes to {target}", payload.len());
        match tokio::time::timeout(timeout, socket.send_to(payload, target)).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                reason = format!("send: {error}");
                continue;
            }
            Err(_) => {
                reason = "send timeout".into();
                continue;
            }
        }
        match tokio::time::timeout(timeout, socket.recv_from(&mut received)).await {
            Ok(Ok((length, source))) if source == target => {
                if length != payload.len() {
                    return failure(
                        format!("recv len: {length} != {}", payload.len()),
                        payload.len(),
                        length,
                    );
                }
                if received[..length] != *payload {
                    return failure("mismatch", payload.len(), length);
                }
                return ProtocolResult::Pass {
                    sent_bytes: payload.len() as u64,
                    received_bytes: length as u64,
                };
            }
            Ok(Ok(_)) => reason = "recv from unexpected source".into(),
            Ok(Err(error)) => reason = format!("recv: {error}"),
            Err(_) => reason = "timeout".into(),
        }
    }
    failure(reason, payload.len(), 0)
}

async fn udp_target(address: SocketAddr, payload: &[u8], timeout: Duration) -> ProtocolResult {
    let socket = match UdpSocket::bind(address).await {
        Ok(socket) => socket,
        Err(error) => {
            return ProtocolResult::Error {
                reason: format!("udp bind: {error}"),
            }
        }
    };
    let mut received = vec![0; payload.len() + 1];
    let mut reason = String::new();
    for _ in 0..5 {
        match tokio::time::timeout(Duration::from_secs(1), socket.recv_from(&mut received)).await {
            Ok(Ok((length, source))) => {
                if length != payload.len() {
                    return failure(
                        format!("recv len: {length} != {}", payload.len()),
                        0,
                        length,
                    );
                }
                if received[..length] != *payload {
                    return failure(
                        "mismatch: received payload differs from expected data",
                        0,
                        length,
                    );
                }
                match tokio::time::timeout(timeout, socket.send_to(&received[..length], source))
                    .await
                {
                    Ok(Ok(_)) => {
                        return ProtocolResult::Pass {
                            sent_bytes: length as u64,
                            received_bytes: length as u64,
                        }
                    }
                    Ok(Err(error)) => return failure(format!("send: {error}"), 0, length),
                    Err(_) => return failure("send timeout", 0, length),
                }
            }
            Ok(Err(error)) => reason = format!("recv: {error}"),
            Err(_) => reason = "recv timeout".into(),
        }
    }
    failure(reason, 0, 0)
}

async fn run_exchange(context: TestContext, payload: &[u8]) -> ProtocolResult {
    match (context.transport, context.direction) {
        (Transport::Tcp, Direction::ClientToServer) => {
            tcp_initiator(context.target_addr, payload, context.timeout).await
        }
        (Transport::Tcp, Direction::ServerToClient) => {
            tcp_target(context.target_addr, payload, context.timeout).await
        }
        (Transport::Udp, Direction::ClientToServer) => {
            udp_initiator(context.target_addr, payload, context.timeout).await
        }
        (Transport::Udp, Direction::ServerToClient) => {
            udp_target(context.target_addr, payload, context.timeout).await
        }
        (Transport::Icmp, _) => ProtocolResult::Error {
            reason: "ICMP not supported by port tests".into(),
        },
    }
}

#[async_trait]
impl TestProtocol for OpenTest {
    fn name(&self) -> &'static str {
        "open"
    }
    fn layer(&self) -> Layer {
        Layer::L4
    }
    fn transports(&self) -> &[Transport] {
        &[Transport::Tcp, Transport::Udp]
    }
    async fn run(&self, context: TestContext) -> ProtocolResult {
        run_exchange(context, &[ONE_BYTE_PAYLOAD]).await
    }
}

#[async_trait]
impl TestProtocol for KbTest {
    fn name(&self) -> &'static str {
        "1kb"
    }
    fn layer(&self) -> Layer {
        Layer::L4
    }
    fn transports(&self) -> &[Transport] {
        &[Transport::Tcp, Transport::Udp]
    }
    async fn run(&self, context: TestContext) -> ProtocolResult {
        run_exchange(context, &kilobyte_payload()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};

    #[tokio::test]
    async fn tcp_open_target_rejects_payload_changed_in_transit() {
        let reservation = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let address = reservation.local_addr().expect("local address");
        drop(reservation);

        let target = tokio::spawn(tcp_target(
            address,
            &[ONE_BYTE_PAYLOAD],
            std::time::Duration::from_secs(1),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let mut client = TcpStream::connect(address).await.expect("connect");
        client
            .write_all(&[ONE_BYTE_PAYLOAD ^ 0xff])
            .await
            .expect("send altered payload");

        assert!(matches!(
            target.await.expect("target task"),
            ProtocolResult::Fail { ref reason, .. } if reason.contains("mismatch")
        ));
    }

    #[tokio::test]
    async fn udp_open_initiator_rejects_reply_from_wrong_source() {
        let expected = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind expected endpoint");
        let target = expected.local_addr().expect("expected address");
        let unexpected = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind unexpected endpoint");
        let responder = tokio::spawn(async move {
            let mut request = [0u8; 2];
            loop {
                let (_, source) = expected
                    .recv_from(&mut request)
                    .await
                    .expect("receive request");
                unexpected
                    .send_to(&[ONE_BYTE_PAYLOAD], source)
                    .await
                    .expect("send spoofed response");
            }
        });

        let result = udp_initiator(
            target,
            &[ONE_BYTE_PAYLOAD],
            std::time::Duration::from_millis(20),
        )
        .await;
        responder.abort();
        assert!(matches!(
            result,
            ProtocolResult::Fail { ref reason, .. } if reason.contains("unexpected source")
        ));
    }

    #[tokio::test]
    async fn udp_1kb_initiator_rejects_oversized_reply() {
        let responder = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind responder");
        let target = responder.local_addr().expect("responder address");
        let responder = tokio::spawn(async move {
            let mut request = vec![0u8; KB + 1];
            loop {
                let (_, source) = responder
                    .recv_from(&mut request)
                    .await
                    .expect("receive request");
                responder
                    .send_to(&vec![0xAA; KB + 1], source)
                    .await
                    .expect("send oversized response");
            }
        });

        let result = udp_initiator(
            target,
            &kilobyte_payload(),
            std::time::Duration::from_millis(50),
        )
        .await;
        responder.abort();
        assert!(matches!(
            result,
            ProtocolResult::Fail { ref reason, .. } if reason.contains("recv len")
        ));
    }
}
