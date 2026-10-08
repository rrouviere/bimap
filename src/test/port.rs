use crate::orchestrator::ProtocolResult;
use crate::test::{Direction, Layer, TestContext, TestProtocol, Transport};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tracing::{debug, trace};

pub struct OpenTest;

const ONE_BYTE_PAYLOAD: u8 = 0xAA;

pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

async fn tcp_open_initiator(target: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let mut last_err = String::new();
    for attempt in 0..20 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        debug!(
            "connecting to {}:{} (timeout={}ms)",
            target.ip(),
            target.port(),
            timeout.as_millis()
        );
        match tokio::time::timeout(timeout, TcpStream::connect(target)).await {
            Ok(Ok(mut stream)) => {
                match tokio::time::timeout(timeout, stream.write_all(&[ONE_BYTE_PAYLOAD])).await {
                    Ok(Ok(())) => {
                        let mut buf = [0u8; 1];
                        match tokio::time::timeout(timeout, stream.read_exact(&mut buf)).await {
                            Ok(Ok(_)) => {
                                if buf[0] == ONE_BYTE_PAYLOAD {
                                    return ProtocolResult::Pass {
                                        sent_bytes: 1,
                                        received_bytes: 1,
                                    };
                                } else {
                                    return ProtocolResult::Fail {
                                        reason: format!(
                                            "mismatch: sent 0x{:02x}, received 0x{:02x}",
                                            ONE_BYTE_PAYLOAD, buf[0]
                                        ),
                                        sent_bytes: 1,
                                        received_bytes: 1,
                                    };
                                }
                            }
                            Ok(Err(e)) => {
                                return ProtocolResult::Fail {
                                    reason: format!("read: {e}"),
                                    sent_bytes: 1,
                                    received_bytes: 0,
                                };
                            }
                            Err(_) => {
                                return ProtocolResult::Fail {
                                    reason: "timeout".into(),
                                    sent_bytes: 1,
                                    received_bytes: 0,
                                };
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        return ProtocolResult::Fail {
                            reason: format!("write: {e}"),
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
                }
            }
            Ok(Err(e)) => {
                if e.kind() == std::io::ErrorKind::ConnectionRefused {
                    last_err = "refused".into();
                    continue;
                } else {
                    return ProtocolResult::Fail {
                        reason: format!("connect: {e}"),
                        sent_bytes: 0,
                        received_bytes: 0,
                    };
                }
            }
            Err(_) => {
                last_err = "timeout".into();
                continue;
            }
        }
    }
    ProtocolResult::Fail {
        reason: last_err,
        sent_bytes: 0,
        received_bytes: 0,
    }
}

async fn tcp_open_target(addr: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("bind: {e}"),
            }
        }
    };

    debug!("waiting for connection on {}", addr);
    match tokio::time::timeout(timeout, listener.accept()).await {
        Ok(Ok((mut stream, _addr))) => {
            let mut buf = [0u8; 1];
            debug!(
                "waiting for 1 bytes from {}",
                stream.peer_addr().map_or("?".into(), |a| a.to_string())
            );
            match tokio::time::timeout(timeout, stream.read_exact(&mut buf)).await {
                Ok(Ok(_)) => {
                    if buf[0] != ONE_BYTE_PAYLOAD {
                        return ProtocolResult::Fail {
                            reason: format!(
                                "mismatch: expected 0x{ONE_BYTE_PAYLOAD:02x}, received 0x{:02x}",
                                buf[0]
                            ),
                            sent_bytes: 0,
                            received_bytes: 1,
                        };
                    }
                    debug!(
                        "sending 1 bytes to {}",
                        stream.peer_addr().map_or("?".into(), |a| a.to_string())
                    );
                    match tokio::time::timeout(timeout, stream.write_all(&buf)).await {
                        Ok(Ok(())) => ProtocolResult::Pass {
                            sent_bytes: 1,
                            received_bytes: 1,
                        },
                        Ok(Err(e)) => ProtocolResult::Fail {
                            reason: format!("write: {e}"),
                            sent_bytes: 1,
                            received_bytes: 1,
                        },
                        Err(_) => ProtocolResult::Fail {
                            reason: "timeout".into(),
                            sent_bytes: 1,
                            received_bytes: 1,
                        },
                    }
                }
                Ok(Err(e)) => ProtocolResult::Fail {
                    reason: format!("read: {e}"),
                    sent_bytes: 0,
                    received_bytes: 0,
                },
                Err(_) => ProtocolResult::Fail {
                    reason: "timeout".into(),
                    sent_bytes: 0,
                    received_bytes: 0,
                },
            }
        }
        Ok(Err(e)) => ProtocolResult::Fail {
            reason: format!("accept: {e}"),
            sent_bytes: 0,
            received_bytes: 0,
        },
        Err(_) => ProtocolResult::Fail {
            reason: "timeout".into(),
            sent_bytes: 0,
            received_bytes: 0,
        },
    }
}

async fn udp_open_initiator(target: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let bind_addr = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = match UdpSocket::bind(bind_addr).await {
        Ok(s) => s,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("udp bind: {e}"),
            };
        }
    };

    let mut last_err = String::new();
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        debug!("udp sending 1 bytes to {}:{}", target.ip(), target.port());
        match tokio::time::timeout(timeout, socket.send_to(&[ONE_BYTE_PAYLOAD], target)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                last_err = format!("send: {e}");
                continue;
            }
            Err(_) => {
                last_err = "send timeout".into();
                continue;
            }
        }

        let mut buf = [0u8; 2];
        trace!(
            "udp waiting for response (timeout={}ms)",
            timeout.as_millis()
        );
        match tokio::time::timeout(timeout, socket.recv_from(&mut buf)).await {
            Ok(Ok((1, src))) if src == target => {
                if buf[0] == ONE_BYTE_PAYLOAD {
                    return ProtocolResult::Pass {
                        sent_bytes: 1,
                        received_bytes: 1,
                    };
                } else {
                    return ProtocolResult::Fail {
                        reason: format!(
                            "mismatch: sent 0x{:02x}, received 0x{:02x}",
                            ONE_BYTE_PAYLOAD, buf[0]
                        ),
                        sent_bytes: 1,
                        received_bytes: 1,
                    };
                }
            }
            Ok(Ok((n, src))) => {
                last_err = if src != target {
                    "recv from unexpected source".into()
                } else {
                    format!("recv len: {n} != 1")
                };
                continue;
            }
            Ok(Err(e)) => {
                last_err = format!("recv: {e}");
                continue;
            }
            Err(_) => {
                last_err = "timeout".into();
                continue;
            }
        }
    }
    ProtocolResult::Fail {
        reason: last_err,
        sent_bytes: 1,
        received_bytes: 0,
    }
}

async fn udp_open_target(addr: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let socket = match UdpSocket::bind(addr).await {
        Ok(s) => s,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("udp bind: {e}"),
            };
        }
    };

    let mut buf = [0u8; 2];
    let mut last_err = String::new();
    for _ in 0..5 {
        trace!("udp waiting for response (timeout={}ms)", 1000u64);
        match tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            socket.recv_from(&mut buf),
        )
        .await
        {
            Ok(Ok((n, src))) => {
                if n != 1 {
                    return ProtocolResult::Fail {
                        reason: format!("recv len: {n} != 1"),
                        sent_bytes: 0,
                        received_bytes: 0,
                    };
                }

                if buf[0] != ONE_BYTE_PAYLOAD {
                    return ProtocolResult::Fail {
                        reason: format!(
                            "mismatch: expected 0x{ONE_BYTE_PAYLOAD:02x}, received 0x{:02x}",
                            buf[0]
                        ),
                        sent_bytes: 0,
                        received_bytes: 1,
                    };
                }

                debug!("udp sending 1 bytes to {}:{}", src.ip(), src.port());
                match tokio::time::timeout(timeout, socket.send_to(&buf[..n], src)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        return ProtocolResult::Fail {
                            reason: format!("send: {e}"),
                            sent_bytes: 0,
                            received_bytes: 1,
                        };
                    }
                    Err(_) => {
                        return ProtocolResult::Fail {
                            reason: "send timeout".into(),
                            sent_bytes: 0,
                            received_bytes: 1,
                        };
                    }
                }

                return ProtocolResult::Pass {
                    sent_bytes: 1,
                    received_bytes: 1,
                };
            }
            Ok(Err(e)) => last_err = format!("recv: {e}"),
            Err(_) => last_err = "recv timeout".into(),
        }
    }
    ProtocolResult::Fail {
        reason: last_err,
        sent_bytes: 0,
        received_bytes: 0,
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

    async fn run(&self, ctx: TestContext) -> ProtocolResult {
        match ctx.transport {
            Transport::Tcp => match ctx.direction {
                Direction::ClientToServer => tcp_open_initiator(ctx.target_addr, ctx.timeout).await,
                Direction::ServerToClient => tcp_open_target(ctx.target_addr, ctx.timeout).await,
            },
            Transport::Udp => match ctx.direction {
                Direction::ClientToServer => udp_open_initiator(ctx.target_addr, ctx.timeout).await,
                Direction::ServerToClient => udp_open_target(ctx.target_addr, ctx.timeout).await,
            },
            Transport::Icmp => ProtocolResult::Error {
                reason: "ICMP not supported by open test".into(),
            },
        }
    }
}

pub struct KbTest;

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

async fn tcp_1kb_initiator(target: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let mut last_err = String::new();
    for attempt in 0..20 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        debug!(
            "connecting to {}:{} (timeout={}ms)",
            target.ip(),
            target.port(),
            timeout.as_millis()
        );
        match tokio::time::timeout(timeout, TcpStream::connect(target)).await {
            Ok(Ok(mut stream)) => {
                let payload = kb_payload();
                let payload_hash = compute_sha256(&payload);

                debug!(
                    "sending {} bytes to {}",
                    payload.len(),
                    stream.peer_addr().map_or("?".into(), |a| a.to_string())
                );
                match tokio::time::timeout(timeout, stream.write_all(&payload)).await {
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
                debug!(
                    "waiting for {} bytes from {}",
                    KB,
                    stream.peer_addr().map_or("?".into(), |a| a.to_string())
                );
                match tokio::time::timeout(timeout, stream.read_exact(&mut buf)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        return ProtocolResult::Fail {
                            reason: format!("read: {e}"),
                            sent_bytes: KB as u64,
                            received_bytes: 0,
                        };
                    }
                    Err(_) => {
                        return ProtocolResult::Fail {
                            reason: "read: timeout".into(),
                            sent_bytes: KB as u64,
                            received_bytes: 0,
                        };
                    }
                }

                let recv_hash = compute_sha256(&buf);
                if payload_hash == recv_hash {
                    return ProtocolResult::Pass {
                        sent_bytes: KB as u64,
                        received_bytes: KB as u64,
                    };
                } else {
                    return ProtocolResult::Fail {
                        reason: "mismatch".into(),
                        sent_bytes: KB as u64,
                        received_bytes: KB as u64,
                    };
                }
            }
            Ok(Err(e)) => {
                if e.kind() == std::io::ErrorKind::ConnectionRefused {
                    last_err = "refused".into();
                    continue;
                } else {
                    return ProtocolResult::Fail {
                        reason: format!("connect: {e}"),
                        sent_bytes: 0,
                        received_bytes: 0,
                    };
                }
            }
            Err(_) => {
                last_err = "timeout".into();
                continue;
            }
        }
    }
    ProtocolResult::Fail {
        reason: last_err,
        sent_bytes: 0,
        received_bytes: 0,
    }
}

async fn tcp_1kb_target(addr: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("bind: {e}"),
            };
        }
    };

    debug!("waiting for connection on {}", addr);
    let (mut stream, _) = match tokio::time::timeout(timeout, listener.accept()).await {
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

    let mut buf = vec![0u8; KB];
    debug!(
        "waiting for {} bytes from {}",
        KB,
        stream.peer_addr().map_or("?".into(), |a| a.to_string())
    );
    match tokio::time::timeout(timeout, stream.read_exact(&mut buf)).await {
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

    if buf != kb_payload() {
        return ProtocolResult::Fail {
            reason: "mismatch: received payload differs from expected 1kb data".into(),
            sent_bytes: 0,
            received_bytes: KB as u64,
        };
    }

    debug!(
        "sending {} bytes to {}",
        buf.len(),
        stream.peer_addr().map_or("?".into(), |a| a.to_string())
    );
    match tokio::time::timeout(timeout, stream.write_all(&buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("write: {e}"),
                sent_bytes: 0,
                received_bytes: KB as u64,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "write: timeout".into(),
                sent_bytes: 0,
                received_bytes: KB as u64,
            };
        }
    }

    ProtocolResult::Pass {
        sent_bytes: KB as u64,
        received_bytes: KB as u64,
    }
}

async fn udp_1kb_initiator(target: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let bind_addr = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = match UdpSocket::bind(bind_addr).await {
        Ok(s) => s,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("udp bind: {e}"),
            };
        }
    };

    let payload = kb_payload();
    let payload_hash = compute_sha256(&payload);

    let mut last_err = String::new();
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        debug!(
            "udp sending {} bytes to {}:{}",
            payload.len(),
            target.ip(),
            target.port()
        );
        match tokio::time::timeout(timeout, socket.send_to(&payload, target)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                last_err = format!("send: {e}");
                continue;
            }
            Err(_) => {
                last_err = "send timeout".into();
                continue;
            }
        }

        let mut buf = vec![0u8; KB + 1];
        trace!(
            "udp waiting for response (timeout={}ms)",
            timeout.as_millis()
        );
        match tokio::time::timeout(timeout, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, src))) if src == target => {
                if n != KB {
                    return ProtocolResult::Fail {
                        reason: format!("recv len: {n} != {KB}"),
                        sent_bytes: KB as u64,
                        received_bytes: n as u64,
                    };
                }
                let recv_hash = compute_sha256(&buf[..n]);
                if payload_hash == recv_hash {
                    return ProtocolResult::Pass {
                        sent_bytes: KB as u64,
                        received_bytes: KB as u64,
                    };
                } else {
                    return ProtocolResult::Fail {
                        reason: "mismatch".into(),
                        sent_bytes: KB as u64,
                        received_bytes: KB as u64,
                    };
                }
            }
            Ok(Ok((_n, _src))) => {
                last_err = "recv from unexpected source".into();
                continue;
            }
            Ok(Err(e)) => {
                last_err = format!("recv: {e}");
                continue;
            }
            Err(_) => {
                last_err = "timeout".into();
                continue;
            }
        }
    }
    ProtocolResult::Fail {
        reason: last_err,
        sent_bytes: KB as u64,
        received_bytes: 0,
    }
}

async fn udp_1kb_target(addr: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let socket = match UdpSocket::bind(addr).await {
        Ok(s) => s,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("udp bind: {e}"),
            };
        }
    };

    let mut buf = vec![0u8; KB + 1];
    let mut last_err = String::new();
    for _ in 0..5 {
        trace!("udp waiting for response (timeout={}ms)", 1000u64);
        match tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            socket.recv_from(&mut buf),
        )
        .await
        {
            Ok(Ok((n, src))) => {
                if n != KB {
                    return ProtocolResult::Fail {
                        reason: format!("recv len: {n} != {KB}"),
                        sent_bytes: 0,
                        received_bytes: n as u64,
                    };
                }

                if buf[..n] != kb_payload() {
                    return ProtocolResult::Fail {
                        reason: "mismatch: received payload differs from expected 1kb data".into(),
                        sent_bytes: 0,
                        received_bytes: n as u64,
                    };
                }

                debug!("udp sending {} bytes to {}:{}", n, src.ip(), src.port());
                match tokio::time::timeout(timeout, socket.send_to(&buf[..n], src)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        return ProtocolResult::Fail {
                            reason: format!("send: {e}"),
                            sent_bytes: 0,
                            received_bytes: n as u64,
                        };
                    }
                    Err(_) => {
                        return ProtocolResult::Fail {
                            reason: "send timeout".into(),
                            sent_bytes: 0,
                            received_bytes: n as u64,
                        };
                    }
                }

                return ProtocolResult::Pass {
                    sent_bytes: KB as u64,
                    received_bytes: KB as u64,
                };
            }
            Ok(Err(e)) => last_err = format!("recv: {e}"),
            Err(_) => last_err = "recv timeout".into(),
        }
    }
    ProtocolResult::Fail {
        reason: last_err,
        sent_bytes: 0,
        received_bytes: 0,
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

    async fn run(&self, ctx: TestContext) -> ProtocolResult {
        match ctx.transport {
            Transport::Tcp => match ctx.direction {
                Direction::ClientToServer => tcp_1kb_initiator(ctx.target_addr, ctx.timeout).await,
                Direction::ServerToClient => tcp_1kb_target(ctx.target_addr, ctx.timeout).await,
            },
            Transport::Udp => match ctx.direction {
                Direction::ClientToServer => udp_1kb_initiator(ctx.target_addr, ctx.timeout).await,
                Direction::ServerToClient => udp_1kb_target(ctx.target_addr, ctx.timeout).await,
            },
            Transport::Icmp => ProtocolResult::Error {
                reason: "ICMP not supported by 1kb test".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn tcp_open_target_rejects_payload_changed_in_transit() {
        let reservation = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let address = reservation.local_addr().expect("local address");
        drop(reservation);

        let target = tokio::spawn(tcp_open_target(address, std::time::Duration::from_secs(1)));
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

        let result = udp_open_initiator(target, std::time::Duration::from_millis(20)).await;
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

        let result = udp_1kb_initiator(target, std::time::Duration::from_millis(50)).await;
        responder.abort();
        assert!(matches!(
            result,
            ProtocolResult::Fail { ref reason, .. } if reason.contains("recv len")
        ));
    }
}
