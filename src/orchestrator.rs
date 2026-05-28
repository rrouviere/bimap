use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use crate::control::msg::{Message, PortRangeSpec, TestSummary, TransferReport};
use crate::control::ControlChannel;
use crate::test::{Direction, TestContext, TestRegistry, Transport};

#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolResult {
    Pass {
        sent_bytes: u64,
        received_bytes: u64,
    },
    Fail {
        reason: String,
        sent_bytes: u64,
        received_bytes: u64,
    },
    Error {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct TestEntry {
    pub protocol: String,
    pub transport: String,
    pub port: u16,
    pub direction: Direction,
    pub result: ProtocolResult,
    pub server_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum PortRange {
    Single(u16),
    Range(u16, u16),
}

impl std::fmt::Display for PortRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortRange::Single(p) => write!(f, "{p}"),
            PortRange::Range(s, e) => write!(f, "{s}-{e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MergedEntry {
    pub protocol: String,
    pub transport: String,
    pub ports: PortRange,
    pub direction: Direction,
    pub status: String,
    pub reason: String,
    pub count: u64,
    pub server_error: Option<String>,
}

pub async fn run_server(
    mut channel: ControlChannel,
    registry: &TestRegistry,
    timeout_ms: u64,
) -> Result<TestSummary, String> {
    match channel.recv().await? {
        Message::Configure { .. } => {
            channel
                .send(&Message::Ack {
                    ok: true,
                    message: None,
                })
                .await?;
        }
        _ => return Err("expected Configure message".into()),
    }

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errors = 0u32;

    loop {
        match channel.recv().await? {
            Message::Test {
                id,
                protocol,
                transport,
                port,
                direction,
            } => {
                let proto = registry
                    .find(&protocol)
                    .ok_or_else(|| format!("unknown protocol: {protocol}"))?;

                let transport = Transport::from_str(&transport)
                    .ok_or_else(|| format!("unknown transport: {transport}"))?;

                let dir = parse_direction(&direction)?;
                // Server-side direction is inverted
                let server_dir = match dir {
                    Direction::ClientToServer => Direction::ServerToClient,
                    Direction::ServerToClient => Direction::ClientToServer,
                };

                let target_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);

                let ctx = TestContext {
                    direction: server_dir,
                    transport,
                    port,
                    target_addr,
                    timeout: Duration::from_millis(timeout_ms),
                    verbose: false,
                };

                let result = proto.run(ctx).await;

                match &result {
                    ProtocolResult::Pass { .. } => passed += 1,
                    ProtocolResult::Fail { .. } => failed += 1,
                    ProtocolResult::Error { .. } => errors += 1,
                }

                let report = protocol_result_to_report(id, &result);
                channel.send(&report).await?;
            }
            Message::Done => break,
            _ => return Err("unexpected message, expected Test or Done".into()),
        }
    }

    let summary = TestSummary {
        passed,
        failed,
        errors,
    };
    channel
        .send(&Message::Bye {
            summary: summary.clone(),
        })
        .await?;
    Ok(summary)
}

pub struct ClientConfig {
    pub tests: Vec<String>,
    pub port_ranges: Vec<(String, u16, u16)>,
    pub bidir: bool,
    pub timeout_ms: u64,
    pub server_addr: IpAddr,
    pub json: bool,
    pub json_export: bool,
    pub verbose: bool,
}

pub async fn run_client(
    mut channel: ControlChannel,
    registry: &TestRegistry,
    config: &ClientConfig,
) -> Result<TestSummary, String> {
    let mut port_ranges = config.port_ranges.clone();

    for test_name in &config.tests {
        let Some(proto) = registry.find(test_name) else {
            return Err(format!("unknown protocol: {test_name}"));
        };
        let mut needs_icmp = false;
        let mut has_any = false;
        for transport in proto.transports() {
            let has_matching = port_ranges
                .iter()
                .any(|(t, _, _)| Transport::from_str(t).is_some_and(|pt| pt == *transport));
            if has_matching {
                has_any = true;
            } else if *transport == Transport::Icmp {
                needs_icmp = true;
            }
        }
        if needs_icmp && !has_any {
            port_ranges.push(("icmp".into(), 0, 0));
        } else if !has_any {
            let transports: Vec<&str> = proto.transports().iter().map(|t| t.as_str()).collect();
            return Err(format!(
                "{test_name} needs a matching --port-range (supports {})",
                transports.join(", ")
            ));
        }
    }

    let port_range_specs: Vec<PortRangeSpec> = port_ranges
        .iter()
        .map(|(transport, start, end)| PortRangeSpec {
            transport: transport.clone(),
            start: *start,
            end: *end,
        })
        .collect();

    channel
        .send(&Message::Configure {
            tests: config.tests.to_vec(),
            port_ranges: port_range_specs,
            bidir: config.bidir,
        })
        .await?;

    match channel.recv().await? {
        Message::Ack { ok: true, .. } => {}
        Message::Ack {
            ok: false,
            message: Some(msg),
        } => return Err(format!("server rejected config: {msg}")),
        Message::Ack {
            ok: false,
            message: None,
        } => return Err("server rejected config".into()),
        _ => return Err("expected Ack".into()),
    }

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errors = 0u32;
    let mut id = 0u32;
    let mut results: Vec<TestEntry> = Vec::new();

    for test_name in &config.tests {
        let proto = registry
            .find(test_name)
            .ok_or_else(|| format!("unknown protocol: {test_name}"))?;

        for (transport_str, start, end) in &port_ranges {
            let Some(transport) = Transport::from_str(transport_str) else {
                continue;
            };

            if !proto.transports().contains(&transport) {
                continue;
            }

            for port in *start..=*end {
                let directions = if config.bidir {
                    vec![Direction::ClientToServer, Direction::ServerToClient]
                } else {
                    vec![Direction::ClientToServer]
                };

                for dir in directions {
                    let target_addr = SocketAddr::new(config.server_addr, port);

                    if config.verbose {
                        eprintln!("[v] orchestrator sending test {} to server", id);
                    }

                    channel
                        .send(&Message::Test {
                            id,
                            protocol: test_name.clone(),
                            transport: transport_str.clone(),
                            port,
                            direction: dir.as_str().to_string(),
                        })
                        .await?;

                    let ctx = TestContext {
                        direction: dir,
                        transport,
                        port,
                        target_addr,
                        timeout: Duration::from_millis(config.timeout_ms),
                        verbose: config.verbose,
                    };

                    if config.verbose {
                        eprintln!(
                            "[v] test {} {}/{} port={} {}",
                            test_name,
                            transport_str,
                            port,
                            port,
                            dir.as_str()
                        );
                    }

                    let result = proto.run(ctx).await;

                    match &result {
                        ProtocolResult::Pass { .. } => passed += 1,
                        ProtocolResult::Fail { .. } => failed += 1,
                        ProtocolResult::Error { .. } => errors += 1,
                    }

                    if config.verbose {
                        eprintln!("[v] orchestrator waiting for report {}", id);
                    }

                    let server_error = match channel.recv().await? {
                        Message::Report { error, .. } => error,
                        _ => return Err("expected Report".into()),
                    };

                    if let Some(ref err_msg) = server_error {
                        if !matches!(result, ProtocolResult::Pass { .. }) {
                            eprintln!("  server: {err_msg}");
                        }
                    }

                    if config.json && !config.json_export {
                        print_result(
                            id,
                            test_name,
                            transport_str,
                            port,
                            dir,
                            &result,
                            true,
                            server_error.as_deref(),
                        );
                    }

                    results.push(TestEntry {
                        protocol: test_name.clone(),
                        transport: transport_str.clone(),
                        port,
                        direction: dir,
                        result,
                        server_error,
                    });

                    id += 1;
                }
            }
        }
    }

    let merged = merge_into_ranges(&results);

    if config.json_export {
        print_json_export(&merged, passed, failed, errors);
    } else if !config.json {
        print_merged_results(&merged);
        print_summary(passed, failed, errors);
    }

    channel.send(&Message::Done).await?;

    let summary = TestSummary {
        passed,
        failed,
        errors,
    };

    match channel.recv().await? {
        Message::Bye { .. } => Ok(summary),
        _ => Err("expected Bye".into()),
    }
}

fn parse_direction(s: &str) -> Result<Direction, String> {
    match s {
        "->" => Ok(Direction::ClientToServer),
        "<-" => Ok(Direction::ServerToClient),
        _ => Err(format!("unknown direction: {s}")),
    }
}

fn protocol_result_to_report(id: u32, result: &ProtocolResult) -> Message {
    match result {
        ProtocolResult::Pass {
            sent_bytes,
            received_bytes,
        } => Message::Report {
            id,
            sent: Some(TransferReport {
                bytes: *sent_bytes,
                sha256: String::new(),
            }),
            received: Some(TransferReport {
                bytes: *received_bytes,
                sha256: String::new(),
            }),
            error: None,
        },
        ProtocolResult::Fail {
            reason,
            sent_bytes,
            received_bytes,
        } => Message::Report {
            id,
            sent: Some(TransferReport {
                bytes: *sent_bytes,
                sha256: String::new(),
            }),
            received: Some(TransferReport {
                bytes: *received_bytes,
                sha256: String::new(),
            }),
            error: Some(reason.clone()),
        },
        ProtocolResult::Error { reason } => Message::Report {
            id,
            sent: None,
            received: None,
            error: Some(reason.clone()),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn print_result(
    id: u32,
    protocol: &str,
    transport: &str,
    port: u16,
    direction: Direction,
    result: &ProtocolResult,
    json: bool,
    server_error: Option<&str>,
) {
    if json {
        let (status, reason, tx, rx) = match result {
            ProtocolResult::Pass {
                sent_bytes,
                received_bytes,
            } => ("pass", String::new(), *sent_bytes, *received_bytes),
            ProtocolResult::Fail {
                reason,
                sent_bytes,
                received_bytes,
            } => ("fail", reason.clone(), *sent_bytes, *received_bytes),
            ProtocolResult::Error { reason } => ("error", reason.clone(), 0u64, 0u64),
        };
        let err_field = server_error
            .map(|e| format!(r#","server_error":"{}""#, e))
            .unwrap_or_default();
        println!(
            r#"{{"id":{id},"protocol":"{protocol}","transport":"{transport}","port":{port},"direction":"{dir}","status":"{status}","reason":"{reason}","tx":{tx},"rx":{rx}{err_field}}}"#,
            dir = direction.as_str(),
        );
    } else {
        match result {
            ProtocolResult::Pass {
                sent_bytes,
                received_bytes,
            } => {
                println!(
                    "PASS {protocol} {transport} {port} {} (tx={sent_bytes} rx={received_bytes})",
                    direction.as_str()
                );
            }
            ProtocolResult::Fail {
                reason,
                sent_bytes,
                received_bytes,
            } => {
                println!(
                    "FAIL {protocol} {transport} {port} {} {reason} (tx={sent_bytes} rx={received_bytes})",
                    direction.as_str()
                );
            }
            ProtocolResult::Error { reason } => {
                println!(
                    "ERR  {protocol} {transport} {port} {} {reason}",
                    direction.as_str()
                );
            }
        }
    }
}

fn merge_into_ranges(results: &[TestEntry]) -> Vec<MergedEntry> {
    if results.is_empty() {
        return vec![];
    }

    let mut sorted: Vec<&TestEntry> = results.iter().collect();
    sorted.sort_by(|a, b| {
        a.protocol
            .cmp(&b.protocol)
            .then(a.transport.cmp(&b.transport))
            .then(a.direction.as_str().cmp(b.direction.as_str()))
            .then(a.port.cmp(&b.port))
    });

    let mut merged: Vec<MergedEntry> = vec![];

    for entry in sorted {
        let (status, reason) = match &entry.result {
            ProtocolResult::Pass { .. } => ("pass", String::new()),
            ProtocolResult::Fail { reason, .. } => ("fail", reason.clone()),
            ProtocolResult::Error { reason } => ("error", reason.clone()),
        };

        let should_merge = merged
            .last()
            .map(|last| {
                last.protocol == entry.protocol
                    && last.transport == entry.transport
                    && last.direction == entry.direction
                    && last.status == status
                    && last.reason == reason
                    && match &last.ports {
                        PortRange::Range(_, e) => *e + 1 == entry.port,
                        PortRange::Single(p) => *p + 1 == entry.port,
                    }
            })
            .unwrap_or(false);

        if should_merge {
            if let Some(last) = merged.last_mut() {
                last.ports = match &last.ports {
                    PortRange::Single(p) => PortRange::Range(*p, entry.port),
                    PortRange::Range(s, _) => PortRange::Range(*s, entry.port),
                };
                last.count += 1;
            }
        } else {
            merged.push(MergedEntry {
                protocol: entry.protocol.clone(),
                transport: entry.transport.clone(),
                ports: PortRange::Single(entry.port),
                direction: entry.direction,
                status: status.to_string(),
                reason,
                count: 1,
                server_error: entry.server_error.clone(),
            });
        }
    }

    merged
}

fn print_merged_results(merged: &[MergedEntry]) {
    for m in merged {
        match m.status.as_str() {
            "pass" => println!(
                "PASS {} {} {} {} ({} tests)",
                m.protocol,
                m.transport,
                m.ports,
                m.direction.as_str(),
                m.count
            ),
            "fail" => println!(
                "FAIL {} {} {} {} {} ({} tests)",
                m.protocol,
                m.transport,
                m.ports,
                m.direction.as_str(),
                m.reason,
                m.count
            ),
            "error" => println!(
                "ERR  {} {} {} {} {}",
                m.protocol,
                m.transport,
                m.ports,
                m.direction.as_str(),
                m.reason
            ),
            _ => {}
        }
    }
}

fn print_summary(passed: u32, failed: u32, errors: u32) {
    println!("--- {passed} passed, {failed} failed, {errors} errors ---");
}

fn print_json_export(merged: &[MergedEntry], passed: u32, failed: u32, errors: u32) {
    let results_arr: Vec<serde_json::Value> = merged
        .iter()
        .map(|m| {
            let mut obj = serde_json::json!({
                "protocol": m.protocol,
                "transport": m.transport,
                "direction": m.direction.as_str(),
                "status": m.status,
            });
            if m.status == "fail" || m.status == "error" {
                obj["reason"] = serde_json::json!(m.reason);
            }
            match &m.ports {
                PortRange::Single(p) => {
                    obj["port"] = serde_json::json!(p);
                }
                PortRange::Range(s, e) => {
                    obj["port_start"] = serde_json::json!(s);
                    obj["port_end"] = serde_json::json!(e);
                    obj["count"] = serde_json::json!(m.count);
                }
            }
            if let Some(ref err) = m.server_error {
                obj["server_error"] = serde_json::json!(err);
            }
            obj
        })
        .collect();

    let output = serde_json::json!({
        "bimap": {
            "version": "0.2.4",
            "mode": "client"
        },
        "summary": {
            "passed": passed,
            "failed": failed,
            "errors": errors
        },
        "results": results_arr,
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("json serialization")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_result_sizes() {
        let r = ProtocolResult::Pass {
            sent_bytes: 1,
            received_bytes: 1,
        };
        assert!(matches!(r, ProtocolResult::Pass { .. }));
    }

    #[test]
    fn parse_direction_valid() {
        assert_eq!(parse_direction("->").unwrap(), Direction::ClientToServer);
        assert_eq!(parse_direction("<-").unwrap(), Direction::ServerToClient);
    }

    #[test]
    fn parse_direction_invalid() {
        assert!(parse_direction("invalid").is_err());
    }
}
