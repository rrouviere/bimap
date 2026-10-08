use super::exchange::connect;
use crate::orchestrator::ProtocolResult;
use crate::packet::dns;
use crate::test::{Direction, Layer, TestContext, TestProtocol, Transport};
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tracing::{debug, trace};

pub struct DnsTest;

static DNS_QUERY_ID: AtomicU16 = AtomicU16::new(0);

fn next_query_id() -> u16 {
    DNS_QUERY_ID.fetch_add(1, Ordering::SeqCst)
}

fn validate_dns_response(query_bytes: &[u8], response_bytes: &[u8]) -> Result<(), String> {
    let query = dns::parse_dns_message(query_bytes)?;
    let response = dns::parse_dns_message(response_bytes)?;
    if response.message_type() != hickory_proto::op::MessageType::Response {
        return Err("dns-malformed: not a response".into());
    }
    if response.id() != query.id() {
        return Err("dns-mismatch: transaction ID".into());
    }
    if response.queries() != query.queries() || query.queries().is_empty() {
        return Err("dns-mismatch: question section".into());
    }
    Ok(())
}

async fn dns_udp_initiator(target: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let bind_addr = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    debug!("dns binding udp on {}", bind_addr);
    let socket = match UdpSocket::bind(bind_addr).await {
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

    let receive_timeout = (timeout / 5).min(std::time::Duration::from_millis(200));
    let mut last_err = String::new();
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        debug!("dns sending query to {}:{}", target.ip(), target.port());
        match tokio::time::timeout(timeout, socket.send_to(&query_bytes, target)).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                last_err = format!("send: {error}");
                continue;
            }
            Err(error) => {
                last_err = format!("send timeout: {error}");
                continue;
            }
        }

        let mut buf = [0u8; 1500];
        trace!(
            "dns waiting for response (timeout={}ms)",
            receive_timeout.as_millis()
        );
        match tokio::time::timeout(receive_timeout, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, _addr))) => match validate_dns_response(&query_bytes, &buf[..n]) {
                Ok(()) => {
                    return ProtocolResult::Pass {
                        sent_bytes: query_len,
                        received_bytes: n as u64,
                    };
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

async fn dns_tcp_initiator(target: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    let mut stream = match connect(target, timeout).await {
        Ok(stream) => stream,
        Err(result) => return result,
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

    debug!("dns tcp sending query ({} bytes)", framed_len);
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
    debug!("dns tcp waiting for length prefix");
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
    if response_len == 0 {
        return ProtocolResult::Fail {
            reason: "dns-malformed: bad length".into(),
            sent_bytes: framed_len,
            received_bytes: 0,
        };
    }

    let mut response_buf = vec![0u8; response_len];
    trace!("dns tcp waiting for {} bytes response", response_len);
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

    match validate_dns_response(&query_bytes, &response_buf) {
        Ok(()) => ProtocolResult::Pass {
            sent_bytes: framed_len,
            received_bytes: (2 + response_len) as u64,
        },
        Err(error) => ProtocolResult::Fail {
            reason: error,
            sent_bytes: framed_len,
            received_bytes: (2 + response_len) as u64,
        },
    }
}

async fn dns_udp_target(addr: SocketAddr, _timeout: std::time::Duration) -> ProtocolResult {
    debug!("dns binding udp on {}", addr);
    let socket = match UdpSocket::bind(addr).await {
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
        trace!("dns waiting for response (timeout={}ms)", 1000u64);
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

async fn dns_tcp_target(addr: SocketAddr, timeout: std::time::Duration) -> ProtocolResult {
    debug!("dns binding tcp on {}", addr);
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            return ProtocolResult::Error {
                reason: format!("bind: {e}"),
            };
        }
    };

    debug!("dns tcp waiting for connection on {}", addr);
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
    debug!("dns tcp waiting for length prefix");
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
    if query_len == 0 {
        return ProtocolResult::Fail {
            reason: format!("invalid query length: {query_len}"),
            sent_bytes: 0,
            received_bytes: 0,
        };
    }
    let mut query_buf = vec![0u8; query_len];
    trace!("dns tcp waiting for {} bytes query", query_len);
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

    debug!("dns tcp sending query ({} bytes)", framed_len);
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
        let operation = async {
            match ctx.transport {
                Transport::Tcp => match ctx.direction {
                    Direction::ClientToServer => {
                        dns_tcp_initiator(ctx.target_addr, ctx.timeout).await
                    }
                    Direction::ServerToClient => dns_tcp_target(ctx.target_addr, ctx.timeout).await,
                },
                Transport::Udp => match ctx.direction {
                    Direction::ClientToServer => {
                        dns_udp_initiator(ctx.target_addr, ctx.timeout).await
                    }
                    Direction::ServerToClient => dns_udp_target(ctx.target_addr, ctx.timeout).await,
                },
                Transport::Icmp => ProtocolResult::Error {
                    reason: "ICMP not supported by DNS test".into(),
                },
            }
        };
        match tokio::time::timeout(ctx.timeout, operation).await {
            Ok(result) => result,
            Err(_) => ProtocolResult::Fail {
                reason: "timeout".into(),
                sent_bytes: 0,
                received_bytes: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::serialize::binary::BinEncodable;

    #[tokio::test]
    async fn dns_tcp_refused_connection_finishes_within_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        drop(listener);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            DnsTest.run(TestContext {
                direction: Direction::ClientToServer,
                transport: Transport::Tcp,
                target_addr: address,
                timeout: std::time::Duration::from_millis(50),
            }),
        )
        .await
        .expect("DNS deadline must bound refused connections");
        assert!(matches!(result, ProtocolResult::Fail { .. }));
    }
    #[tokio::test]
    async fn dns_udp_delayed_listener_retries_and_passes() {
        let address: SocketAddr = "127.0.0.1:18640".parse().expect("address");
        let responder = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            DnsTest
                .run(TestContext {
                    direction: Direction::ServerToClient,
                    transport: Transport::Udp,
                    target_addr: address,
                    timeout: std::time::Duration::from_millis(500),
                })
                .await
        });
        let result = DnsTest
            .run(TestContext {
                direction: Direction::ClientToServer,
                transport: Transport::Udp,
                target_addr: address,
                timeout: std::time::Duration::from_millis(500),
            })
            .await;
        let target_result = responder.await.expect("responder");
        assert!(
            matches!(result, ProtocolResult::Pass { .. }),
            "initiator must retry lost first query: {result:?}"
        );
        assert!(
            matches!(target_result, ProtocolResult::Pass { .. }),
            "target must receive retried query: {target_result:?}"
        );
    }

    #[test]
    fn dns_response_rejects_wrong_transaction_id_and_missing_question() {
        let query = dns::build_dns_query("bimap.test", 0x1234).expect("query");
        let parsed_query = dns::parse_dns_message(&query).expect("parse query");
        let mut wrong_id = dns::build_dns_response(&parsed_query).expect("response");
        wrong_id[0] ^= 1;
        assert!(validate_dns_response(&query, &wrong_id)
            .expect_err("wrong transaction ID")
            .contains("transaction ID"));

        let mut no_questions =
            dns::parse_dns_message(&dns::build_dns_response(&parsed_query).expect("response"))
                .expect("parse response");
        no_questions.take_queries();
        let mut bytes = Vec::new();
        let mut encoder = hickory_proto::serialize::binary::BinEncoder::new(&mut bytes);
        no_questions.emit(&mut encoder).expect("encode response");
        assert!(validate_dns_response(&query, &bytes)
            .expect_err("missing question")
            .contains("question section"));
    }
}
