use crate::orchestrator::ProtocolResult;
use crate::packet::dns;
use crate::test::{Direction, Layer, TestContext, TestProtocol, Transport};
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

pub struct DnsTest;

static DNS_QUERY_ID: AtomicU16 = AtomicU16::new(0);

fn next_query_id() -> u16 {
    DNS_QUERY_ID.fetch_add(1, Ordering::SeqCst)
}

async fn dns_udp_initiator(
    target: SocketAddr,
    timeout: std::time::Duration,
    verbose: bool,
) -> ProtocolResult {
    if verbose {
        eprintln!("[v] dns binding udp port 0");
    }
    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("bind: {e}"),
            };
        }
    };

    let query_bytes = match dns::build_dns_query("bimap.test", next_query_id()) {
        Ok(b) => b,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("query: {e}"),
            };
        }
    };
    let query_len = query_bytes.len() as u64;

    let mut last_err = String::new();
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        if verbose {
            eprintln!("[v] dns sending query to {}:{}", target.ip(), target.port());
        }
        if let Err(e) = tokio::time::timeout(timeout, socket.send_to(&query_bytes, target)).await {
            last_err = format!("send timeout: {e}");
            continue;
        }
        if let Err(e) = socket.send_to(&query_bytes, target).await {
            last_err = format!("send: {e}");
            continue;
        }

        let mut buf = [0u8; 1500];
        if verbose {
            eprintln!(
                "[v] dns waiting for response (timeout={}ms)",
                timeout.as_millis()
            );
        }
        match tokio::time::timeout(timeout, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, _addr))) => match dns::parse_dns_message(&buf[..n]) {
                Ok(response) => {
                    if response.message_type() == hickory_proto::op::MessageType::Response {
                        return ProtocolResult::Pass {
                            sent_bytes: query_len,
                            received_bytes: n as u64,
                        };
                    } else {
                        last_err = "dns-malformed: not a response".into();
                        continue;
                    }
                }
                Err(e) => {
                    last_err = format!("dns-malformed: {e}");
                    continue;
                }
            },
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
        sent_bytes: query_len,
        received_bytes: 0,
    }
}

async fn dns_tcp_initiator(
    target: SocketAddr,
    timeout: std::time::Duration,
    verbose: bool,
) -> ProtocolResult {
    let mut stream = loop {
        if verbose {
            eprintln!(
                "[v] dns tcp connecting to {}:{}",
                target.ip(),
                target.port()
            );
        }
        match tokio::time::timeout(timeout, TcpStream::connect(target)).await {
            Ok(Ok(s)) => break s,
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Ok(Err(e)) => {
                return ProtocolResult::Fail {
                    reason: format!("connect: {e}"),
                    sent_bytes: 0,
                    received_bytes: 0,
                };
            }
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    };

    let query_bytes = match dns::build_dns_query("bimap.test", next_query_id()) {
        Ok(b) => b,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("query: {e}"),
            };
        }
    };
    let len_bytes = (query_bytes.len() as u16).to_be_bytes();
    let mut framed = Vec::with_capacity(2 + query_bytes.len());
    framed.extend_from_slice(&len_bytes);
    framed.extend_from_slice(&query_bytes);

    let framed_len = framed.len() as u64;

    if verbose {
        eprintln!("[v] dns tcp sending query ({} bytes)", framed_len);
    }
    match tokio::time::timeout(timeout, stream.write_all(&framed)).await {
        Ok(Ok(())) => match tokio::time::timeout(timeout, stream.flush()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return ProtocolResult::Fail {
                    reason: format!("flush: {e}"),
                    sent_bytes: framed_len,
                    received_bytes: 0,
                };
            }
            Err(_) => {
                return ProtocolResult::Fail {
                    reason: "timeout".into(),
                    sent_bytes: framed_len,
                    received_bytes: 0,
                };
            }
        },
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("write: {e}"),
                sent_bytes: framed_len,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "timeout".into(),
                sent_bytes: framed_len,
                received_bytes: 0,
            };
        }
    }

    let mut len_buf = [0u8; 2];
    if verbose {
        eprintln!("[v] dns tcp waiting for length prefix");
    }
    match tokio::time::timeout(timeout, stream.read_exact(&mut len_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("read len: {e}"),
                sent_bytes: framed_len,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "timeout".into(),
                sent_bytes: framed_len,
                received_bytes: 0,
            };
        }
    }

    let response_len = u16::from_be_bytes(len_buf) as usize;
    if response_len == 0 || response_len > 65535 {
        return ProtocolResult::Fail {
            reason: "dns-malformed: bad length".into(),
            sent_bytes: framed_len,
            received_bytes: 0,
        };
    }

    let mut response_buf = vec![0u8; response_len];
    if verbose {
        eprintln!("[v] dns tcp waiting for {} bytes response", response_len);
    }
    match tokio::time::timeout(timeout, stream.read_exact(&mut response_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("read body: {e}"),
                sent_bytes: framed_len,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "timeout".into(),
                sent_bytes: framed_len,
                received_bytes: 0,
            };
        }
    }

    match dns::parse_dns_message(&response_buf) {
        Ok(response) if response.message_type() == hickory_proto::op::MessageType::Response => {
            ProtocolResult::Pass {
                sent_bytes: framed_len,
                received_bytes: (2 + response_len) as u64,
            }
        }
        _ => ProtocolResult::Fail {
            reason: "dns-malformed".into(),
            sent_bytes: framed_len,
            received_bytes: (2 + response_len) as u64,
        },
    }
}

async fn dns_udp_target(port: u16, _timeout: std::time::Duration, verbose: bool) -> ProtocolResult {
    let bind_addr = format!("0.0.0.0:{port}");
    if verbose {
        eprintln!("[v] dns binding udp port {}", port);
    }
    let socket = match UdpSocket::bind(&bind_addr).await {
        Ok(s) => s,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("bind: {e}"),
            };
        }
    };

    let mut buf = [0u8; 1500];
    let mut last_err = String::new();
    for _ in 0..5 {
        if verbose {
            eprintln!("[v] dns waiting for response (timeout={}ms)", 1000u64);
        }
        match tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            socket.recv_from(&mut buf),
        )
        .await
        {
            Ok(Ok((n, addr))) => {
                let query = match dns::parse_dns_message(&buf[..n]) {
                    Ok(q) => q,
                    Err(e) => {
                        return ProtocolResult::Fail {
                            reason: format!("dns-malformed: {e}"),
                            sent_bytes: 0,
                            received_bytes: n as u64,
                        };
                    }
                };

                let response_bytes = match dns::build_dns_response(&query) {
                    Ok(b) => b,
                    Err(e) => {
                        return ProtocolResult::Error {
                            reason: format!("response: {e}"),
                        };
                    }
                };

                let sent_len = response_bytes.len() as u64;

                if let Err(e) = socket.send_to(&response_bytes, addr).await {
                    return ProtocolResult::Fail {
                        reason: format!("send response: {e}"),
                        sent_bytes: sent_len,
                        received_bytes: n as u64,
                    };
                }

                return ProtocolResult::Pass {
                    sent_bytes: sent_len,
                    received_bytes: n as u64,
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

async fn dns_tcp_target(port: u16, timeout: std::time::Duration, verbose: bool) -> ProtocolResult {
    let bind_addr = format!("0.0.0.0:{port}");
    if verbose {
        eprintln!("[v] dns binding udp port {}", port);
    }
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("bind: {e}"),
            };
        }
    };

    if verbose {
        eprintln!("[v] dns tcp waiting for connection on port {}", port);
    }
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

    let mut len_buf = [0u8; 2];
    if verbose {
        eprintln!("[v] dns tcp waiting for length prefix");
    }
    match tokio::time::timeout(timeout, stream.read_exact(&mut len_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("read len: {e}"),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "read len: timeout".into(),
                sent_bytes: 0,
                received_bytes: 0,
            };
        }
    }

    let query_len = u16::from_be_bytes(len_buf) as usize;
    if query_len == 0 || query_len > 65535 {
        return ProtocolResult::Fail {
            reason: format!("invalid query length: {query_len}"),
            sent_bytes: 0,
            received_bytes: 0,
        };
    }
    let mut query_buf = vec![0u8; query_len];
    if verbose {
        eprintln!("[v] dns tcp waiting for {} bytes query", query_len);
    }
    match tokio::time::timeout(timeout, stream.read_exact(&mut query_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProtocolResult::Fail {
                reason: format!("read body: {e}"),
                sent_bytes: 0,
                received_bytes: (2 + query_len) as u64,
            };
        }
        Err(_) => {
            return ProtocolResult::Fail {
                reason: "read body: timeout".into(),
                sent_bytes: 0,
                received_bytes: (2 + query_len) as u64,
            };
        }
    }

    let query = match dns::parse_dns_message(&query_buf) {
        Ok(q) => q,
        Err(e) => {
            return ProtocolResult::Fail {
                reason: format!("dns-malformed: {e}"),
                sent_bytes: 0,
                received_bytes: query_len as u64,
            };
        }
    };

    let response_bytes = match dns::build_dns_response(&query) {
        Ok(b) => b,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("response: {e}"),
            };
        }
    };

    let len_bytes = (response_bytes.len() as u16).to_be_bytes();
    let mut framed = Vec::with_capacity(2 + response_bytes.len());
    framed.extend_from_slice(&len_bytes);
    framed.extend_from_slice(&response_bytes);
    let framed_len = framed.len() as u64;

    if verbose {
        eprintln!("[v] dns tcp sending query ({} bytes)", framed_len);
    }
    if let Err(e) = stream.write_all(&framed).await {
        return ProtocolResult::Fail {
            reason: format!("write: {e}"),
            sent_bytes: framed_len,
            received_bytes: (2 + query_len) as u64,
        };
    }

    ProtocolResult::Pass {
        sent_bytes: framed_len,
        received_bytes: (2 + query_len) as u64,
    }
}

#[async_trait]
impl TestProtocol for DnsTest {
    fn name(&self) -> &'static str {
        "dns"
    }

    fn layer(&self) -> Layer {
        Layer::L7
    }

    fn transports(&self) -> &[Transport] {
        &[Transport::Tcp, Transport::Udp]
    }

    async fn run(&self, ctx: TestContext) -> ProtocolResult {
        match ctx.transport {
            Transport::Tcp => match ctx.direction {
                Direction::ClientToServer => {
                    dns_tcp_initiator(ctx.target_addr, ctx.timeout, ctx.verbose).await
                }
                Direction::ServerToClient => {
                    dns_tcp_target(ctx.port, ctx.timeout, ctx.verbose).await
                }
            },
            Transport::Udp => match ctx.direction {
                Direction::ClientToServer => {
                    dns_udp_initiator(ctx.target_addr, ctx.timeout, ctx.verbose).await
                }
                Direction::ServerToClient => {
                    dns_udp_target(ctx.port, ctx.timeout, ctx.verbose).await
                }
            },
            Transport::Icmp => ProtocolResult::Error {
                reason: "ICMP not supported by DNS test".into(),
            },
        }
    }
}
