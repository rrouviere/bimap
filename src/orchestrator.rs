use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use crate::control::msg::{Message, PortRangeSpec, TestSummary, TransferReport, PROTOCOL_VERSION};
use crate::control::ControlChannel;
use crate::output::{
    finish_fail_line, format_port_ranges, is_interactive, print_err, print_fail, print_fail_live,
    print_pass, print_summary,
};
use crate::test::{Direction, TestContext, TestProtocol, TestRegistry, Transport};

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
) -> Result<TestSummary, String> {
    let client_addr = channel.peer_addr()?;
    let (timeout_ms, parallel, test_bind_ip) = match channel.recv().await? {
        Message::Configure {
            target,
            timeout_ms,
            client_version,
            parallel,
            ..
        } => {
            if client_version != PROTOCOL_VERSION {
                channel
                    .send(&Message::Ack {
                        ok: false,
                        message: Some(format!(
                            "client protocol version {client_version} does not match server version {PROTOCOL_VERSION}"
                        )),
                    })
                    .await?;
                return Err("client and server protocol versions do not match".into());
            }
            let bind_addr = target.as_deref().and_then(parse_configured_bind_addr);
            channel
                .send(&Message::Ack {
                    ok: true,
                    message: None,
                })
                .await?;
            (timeout_ms, parallel.max(1), bind_addr)
        }
        _ => return Err("expected Configure message".into()),
    };

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errors = 0u32;

    loop {
        // Read first message: must be Test or Done
        let first = channel.recv().await?;
        let (mut batch, mut done_after) = match first {
            Message::Done => break,
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
                let server_dir = match dir {
                    Direction::ClientToServer => Direction::ServerToClient,
                    Direction::ServerToClient => Direction::ClientToServer,
                };
                let mut target_addr = if server_dir == Direction::ClientToServer {
                    client_addr
                } else {
                    test_bind_ip.unwrap_or(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0))
                };
                target_addr.set_port(port);
                let batch = vec![ServerBatchEntry {
                    id,
                    proto,
                    transport,
                    dir: server_dir,
                    port,
                    target_addr,
                }];
                (batch, false)
            }
            _ => return Err("unexpected message, expected Test or Done".into()),
        };

        // The client marks each batch explicitly. Inferring the boundary from
        // a quiet period loses requests when TLS/TCP spaces out a large burst.
        loop {
            match channel.recv().await? {
                Message::Test {
                    id,
                    protocol,
                    transport,
                    port,
                    direction,
                } => {
                    if batch.len() >= parallel {
                        return Err("batch exceeds configured parallel limit".into());
                    }
                    let transport = Transport::from_str(&transport)
                        .ok_or_else(|| format!("unknown transport: {transport}"))?;
                    let proto = registry
                        .find(&protocol)
                        .ok_or_else(|| format!("unknown protocol: {protocol}"))?;
                    let dir = parse_direction(&direction)?;
                    let server_dir = match dir {
                        Direction::ClientToServer => Direction::ServerToClient,
                        Direction::ServerToClient => Direction::ClientToServer,
                    };
                    let mut target_addr = if server_dir == Direction::ClientToServer {
                        client_addr
                    } else {
                        test_bind_ip
                            .unwrap_or(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0))
                    };
                    target_addr.set_port(port);
                    batch.push(ServerBatchEntry {
                        id,
                        proto,
                        transport,
                        dir: server_dir,
                        port,
                        target_addr,
                    });
                }
                Message::BatchEnd => break,
                Message::Done => {
                    done_after = true;
                    break;
                }
                _ => return Err("unexpected message while collecting batch".into()),
            }
        }

        // Process batch in parallel
        let mut unordered: FuturesUnordered<_> = batch
            .into_iter()
            .map(|entry| {
                let ctx = TestContext {
                    direction: entry.dir,
                    transport: entry.transport,
                    port: entry.port,
                    target_addr: entry.target_addr,
                    timeout: Duration::from_millis(timeout_ms),
                };
                Box::pin(async move { (entry.id, run_protocol(entry.proto, ctx).await) })
            })
            .collect();

        while let Some((id, result)) = unordered.next().await {
            match &result {
                ProtocolResult::Pass { .. } => passed += 1,
                ProtocolResult::Fail { .. } => failed += 1,
                ProtocolResult::Error { .. } => errors += 1,
            }
            let report = protocol_result_to_report(id, &result);
            channel.send(&report).await?;
        }

        if done_after {
            break;
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

async fn run_protocol(proto: &dyn TestProtocol, context: TestContext) -> ProtocolResult {
    let deadline = context.timeout.saturating_add(Duration::from_secs(2));
    match tokio::time::timeout(deadline, proto.run(context)).await {
        Ok(result) => result,
        Err(_) => ProtocolResult::Fail {
            reason: "timeout (test took too long)".into(),
            sent_bytes: 0,
            received_bytes: 0,
        },
    }
}

pub struct ClientConfig {
    pub tests: Vec<String>,
    pub port_ranges: Vec<(String, u16, u16)>,
    pub bidir: bool,
    pub timeout_ms: u64,
    pub parallel: usize,
    pub server_addr: IpAddr,
    pub target_addr: SocketAddr,
    pub json: bool,
    pub json_export: bool,
    pub verbose: u8,
    pub quiet: bool,
}

struct BatchEntry<'a> {
    id: u32,
    test_name: &'a str,
    transport: Transport,
    transport_str: &'a str,
    port: u16,
    direction: Direction,
}

struct ServerBatchEntry<'a> {
    id: u32,
    proto: &'a dyn TestProtocol,
    transport: Transport,
    dir: Direction,
    port: u16,
    target_addr: SocketAddr,
}

#[allow(clippy::too_many_arguments)]
async fn execute_batch(
    channel: &mut ControlChannel,
    batch: &[BatchEntry<'_>],
    proto: &dyn TestProtocol,
    config: &ClientConfig,
    passed: &mut u32,
    failed: &mut u32,
    errors: &mut u32,
    results: &mut Vec<TestEntry>,
) -> Result<(), String> {
    for entry in batch {
        channel
            .send(&Message::Test {
                id: entry.id,
                protocol: entry.test_name.to_string(),
                transport: entry.transport_str.to_string(),
                port: entry.port,
                direction: entry.direction.as_str().to_string(),
            })
            .await?;
    }
    channel.send(&Message::BatchEnd).await?;

    let local_addr = channel.local_addr()?;
    let mut unordered: FuturesUnordered<_> = batch
        .iter()
        .map(|entry| {
            let mut target_addr = if entry.direction == Direction::ServerToClient {
                local_addr
            } else {
                config.target_addr
            };
            target_addr.set_port(entry.port);
            let ctx = TestContext {
                direction: entry.direction,
                transport: entry.transport,
                port: entry.port,
                target_addr,
                timeout: Duration::from_millis(config.timeout_ms),
            };
            Box::pin(async move { (entry.id, run_protocol(proto, ctx).await) })
        })
        .collect();

    let mut completed: HashMap<u32, ProtocolResult> = HashMap::new();
    while let Some((id, result)) = unordered.next().await {
        completed.insert(id, result);
    }

    let mut expected: HashSet<u32> = batch.iter().map(|entry| entry.id).collect();
    let mut server_errors: HashMap<u32, String> = HashMap::new();
    let receive_timeout = Duration::from_millis(config.timeout_ms.saturating_add(2100));
    while !expected.is_empty() {
        match tokio::time::timeout(receive_timeout, channel.recv()).await {
            Ok(Ok(Message::Report {
                id,
                error,
                sent,
                received,
            })) => {
                if !expected.remove(&id) {
                    return Err(format!("unexpected or duplicate report ID: {id}"));
                }
                if let Some(reason) = error {
                    let local_result = completed
                        .get_mut(&id)
                        .ok_or_else(|| format!("missing local result for test {id}"))?;
                    if let ProtocolResult::Pass {
                        sent_bytes,
                        received_bytes,
                    } = local_result
                    {
                        *local_result = if sent.is_none() && received.is_none() {
                            ProtocolResult::Error {
                                reason: format!("server: {reason}"),
                            }
                        } else {
                            ProtocolResult::Fail {
                                reason: format!("server: {reason}"),
                                sent_bytes: *sent_bytes,
                                received_bytes: *received_bytes,
                            }
                        };
                    }
                    server_errors.insert(id, reason);
                }
            }
            Ok(Ok(message)) => return Err(format!("expected Report, got {message:?}")),
            Ok(Err(error)) => return Err(format!("receive report: {error}")),
            Err(_) => return Err("timeout waiting for server reports".into()),
        }
    }

    let interactive = should_render_interactive(
        is_interactive(),
        config.quiet,
        config.json,
        config.json_export,
    );
    let mut fail_map: HashMap<(String, String, String, String), Vec<u16>> = HashMap::new();
    let mut pass_map: HashMap<(String, String, String), Vec<u16>> = HashMap::new();
    for entry in batch {
        let result = completed
            .remove(&entry.id)
            .ok_or_else(|| format!("missing local result for test {}", entry.id))?;
        let server_error = server_errors.remove(&entry.id);
        match &result {
            ProtocolResult::Pass { .. } => *passed += 1,
            ProtocolResult::Fail { .. } => *failed += 1,
            ProtocolResult::Error { .. } => *errors += 1,
        }
        if config.json && !config.json_export {
            print_result(
                entry.id,
                entry.test_name,
                entry.transport_str,
                entry.port,
                entry.direction,
                &result,
                true,
                server_error.as_deref(),
            );
        } else if !config.json_export {
            match &result {
                ProtocolResult::Pass {
                    sent_bytes,
                    received_bytes,
                } => {
                    if interactive {
                        pass_map
                            .entry((
                                entry.test_name.to_string(),
                                entry.transport_str.to_string(),
                                entry.direction.as_str().to_string(),
                            ))
                            .or_default()
                            .push(entry.port);
                    } else if !config.quiet {
                        print_pass(format_args!(
                            "{} {} {} {} (tx={} rx={})",
                            entry.test_name,
                            entry.transport_str,
                            entry.port,
                            entry.direction.as_str(),
                            sent_bytes,
                            received_bytes
                        ));
                    }
                }
                ProtocolResult::Fail {
                    reason,
                    sent_bytes,
                    received_bytes,
                } => {
                    let key = (
                        entry.test_name.to_string(),
                        entry.transport_str.to_string(),
                        entry.direction.as_str().to_string(),
                        reason.clone(),
                    );
                    fail_map.entry(key).or_default().push(entry.port);
                    if interactive {
                        let mut line = String::new();
                        for ((tn, ts, dir, r), ports) in &fail_map {
                            if !line.is_empty() {
                                line.push_str("  ");
                            }
                            let ranges = format_port_ranges(ports);
                            line.push_str(&format!("{tn} {ts} {ranges} {dir} {r}"));
                        }
                        print_fail_live(&line);
                    } else {
                        print_fail(format_args!(
                            "{} {} {} {} {} (tx={} rx={})",
                            entry.test_name,
                            entry.transport_str,
                            entry.port,
                            entry.direction.as_str(),
                            reason,
                            sent_bytes,
                            received_bytes
                        ));
                    }
                }
                ProtocolResult::Error { reason } => {
                    let key = (
                        entry.test_name.to_string(),
                        entry.transport_str.to_string(),
                        entry.direction.as_str().to_string(),
                        reason.clone(),
                    );
                    fail_map.entry(key).or_default().push(entry.port);
                    if interactive {
                        let mut line = String::new();
                        for ((tn, ts, dir, r), ports) in &fail_map {
                            if !line.is_empty() {
                                line.push_str("  ");
                            }
                            let ranges = format_port_ranges(ports);
                            line.push_str(&format!("{tn} {ts} {ranges} {dir} {r}"));
                        }
                        print_fail_live(&line);
                    } else {
                        print_err(format_args!(
                            "{} {} {} {} {}",
                            entry.test_name,
                            entry.transport_str,
                            entry.port,
                            entry.direction.as_str(),
                            reason
                        ));
                    }
                }
            }
        }
        results.push(TestEntry {
            protocol: entry.test_name.to_string(),
            transport: entry.transport_str.to_string(),
            port: entry.port,
            direction: entry.direction,
            result,
            server_error,
        });
    }

    if interactive {
        finish_fail_line();
        for line in consolidated_pass_lines(&pass_map) {
            print_pass(format_args!("{line}"));
        }
    }

    Ok(())
}

pub async fn run_client(
    mut channel: ControlChannel,
    registry: &TestRegistry,
    config: &ClientConfig,
) -> Result<TestSummary, String> {
    let mut port_ranges = config.port_ranges.clone();

    validate_port_ranges(&port_ranges)?;

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
            target: Some(config.target_addr.to_string()),
            timeout_ms: config.timeout_ms,
            client_version: PROTOCOL_VERSION,
            parallel: config.parallel,
        })
        .await?;

    match channel.recv().await? {
        Message::Ack { ok: true, .. } => {}
        Message::Ack {
            ok: false,
            message: Some(msg),
        } => return Err(format!("server rejected config: {msg}")),
        Message::Ack { ok: false, .. } => return Err("server rejected config".into()),
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

            let mut batch: Vec<BatchEntry> = Vec::new();
            let batch_size = config.parallel.max(1);

            let directions = if config.bidir {
                vec![Direction::ClientToServer, Direction::ServerToClient]
            } else {
                vec![Direction::ClientToServer]
            };

            // Opposite directions must not bind the same loopback port concurrently.
            for dir in directions {
                for port in *start..=*end {
                    batch.push(BatchEntry {
                        id,
                        test_name,
                        transport,
                        transport_str,
                        port,
                        direction: dir,
                    });
                    id += 1;

                    if batch.len() >= batch_size {
                        execute_batch(
                            &mut channel,
                            &batch,
                            proto,
                            config,
                            &mut passed,
                            &mut failed,
                            &mut errors,
                            &mut results,
                        )
                        .await?;
                        batch.clear();
                    }
                }
                if !batch.is_empty() {
                    execute_batch(
                        &mut channel,
                        &batch,
                        proto,
                        config,
                        &mut passed,
                        &mut failed,
                        &mut errors,
                        &mut results,
                    )
                    .await?;
                    batch.clear();
                }
            }
        }
    }

    let merged = merge_into_ranges(&results);

    if config.json_export {
        print_json_export(&merged, passed, failed, errors);
    } else if !config.json {
        print_user_summary(passed, failed, errors);
    }

    channel.send(&Message::Done).await?;

    let summary = TestSummary {
        passed,
        failed,
        errors,
    };

    match channel.recv().await {
        Ok(Message::Bye { .. }) => Ok(summary),
        Ok(other) => Err(format!("expected Bye, got {other:?}")),
        Err(e) => Err(format!("recv Bye: {e}")),
    }
}

fn parse_direction(s: &str) -> Result<Direction, String> {
    match s {
        "->" => Ok(Direction::ClientToServer),
        "<-" => Ok(Direction::ServerToClient),
        _ => Err(format!("unknown direction: {s}")),
    }
}

fn parse_configured_bind_addr(value: &str) -> Option<SocketAddr> {
    value.parse::<SocketAddr>().ok().or_else(|| {
        value
            .parse::<IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, 0))
    })
}

fn validate_port_ranges(port_ranges: &[(String, u16, u16)]) -> Result<(), String> {
    if port_ranges.iter().any(|(_, start, end)| start > end) {
        return Err("invalid port range: start must not exceed end".into());
    }
    Ok(())
}

fn should_render_interactive(terminal: bool, quiet: bool, json: bool, json_export: bool) -> bool {
    terminal && !quiet && !json && !json_export
}

/// Build the consolidated PASS port-range lines to print once the live
/// scan loop finishes.
///
/// In interactive mode per-port PASS lines are suppressed during the loop
/// (they would flicker against the `\r`-overwriting fail line on the same
/// row); passes accumulate silently in `pass_map` instead. After the loop
/// `finish_fail_line` finalises the fail row, then this helper turns the
/// collected pass groups into one line per group: `"<test> <transport>
/// <port-ranges> <direction>"`. Fails are NOT included here — they are
/// already visible on the live fail row above, and reprinting them was the
/// duplication bug. callers should `print_pass` each returned line.
fn consolidated_pass_lines(pass_map: &HashMap<(String, String, String), Vec<u16>>) -> Vec<String> {
    pass_map
        .iter()
        .map(|((tn, ts, dir), ports)| {
            format!("{} {} {} {}", tn, ts, format_port_ranges(ports), dir)
        })
        .collect()
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
        let mut output = serde_json::json!({
            "id": id,
            "protocol": protocol,
            "transport": transport,
            "port": port,
            "direction": direction.as_str(),
            "status": status,
            "reason": reason,
            "tx": tx,
            "rx": rx,
        });
        if let Some(error) = server_error {
            output["server_error"] = serde_json::json!(error);
        }
        println!("{output}");
    } else {
        match result {
            ProtocolResult::Pass {
                sent_bytes,
                received_bytes,
            } => {
                print_pass(format_args!(
                    "{protocol} {transport} {port} {} (tx={sent_bytes} rx={received_bytes})",
                    direction.as_str()
                ));
            }
            ProtocolResult::Fail {
                reason,
                sent_bytes,
                received_bytes,
            } => {
                print_fail(format_args!(
                    "{protocol} {transport} {port} {} {reason} (tx={sent_bytes} rx={received_bytes})",
                    direction.as_str()
                ));
            }
            ProtocolResult::Error { reason } => {
                print_err(format_args!(
                    "{protocol} {transport} {port} {} {reason}",
                    direction.as_str()
                ));
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
                    && last.server_error == entry.server_error
                    && match &last.ports {
                        PortRange::Range(_, e) => e.checked_add(1) == Some(entry.port),
                        PortRange::Single(p) => p.checked_add(1) == Some(entry.port),
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

fn print_user_summary(passed: u32, failed: u32, errors: u32) {
    print_summary(format_args!(
        "{passed} passed, {failed} failed, {errors} errors"
    ));
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
            "version": env!("CARGO_PKG_VERSION"),
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

    #[test]
    fn descending_programmatic_port_range_is_rejected() {
        let ranges = vec![("tcp".into(), 20, 10)];
        assert!(validate_port_ranges(&ranges)
            .expect_err("descending range")
            .contains("start must not exceed end"));
    }

    #[test]
    fn quiet_mode_disables_interactive_pass_output() {
        assert!(!should_render_interactive(true, true, false, false));
        assert!(should_render_interactive(true, false, false, false));
    }

    #[test]
    fn configured_ipv6_bind_address_keeps_its_scope_id() {
        let mut address = parse_configured_bind_addr("[fe80::1%2]:0").expect("scoped address");
        address.set_port(8080);
        assert!(
            matches!(address, SocketAddr::V6(address) if address.scope_id() == 2 && address.port() == 8080)
        );
    }

    #[test]
    fn consolidated_pass_lines_groups_contiguous_ports() {
        let mut pass_map: HashMap<(String, String, String), Vec<u16>> = HashMap::new();
        pass_map.insert(
            ("1kb".into(), "tcp".into(), "->".into()),
            vec![1, 2, 3, 5, 6],
        );
        let lines = consolidated_pass_lines(&pass_map);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "1kb tcp 1-3,5-6 ->");
    }

    #[test]
    fn consolidated_pass_lines_empty_when_no_passes() {
        let pass_map: HashMap<(String, String, String), Vec<u16>> = HashMap::new();
        assert!(consolidated_pass_lines(&pass_map).is_empty());
    }

    #[test]
    fn consolidated_pass_lines_emits_one_line_per_group() {
        let mut pass_map: HashMap<(String, String, String), Vec<u16>> = HashMap::new();
        pass_map.insert(("1kb".into(), "tcp".into(), "->".into()), vec![1, 2]);
        pass_map.insert(("1kb".into(), "tcp".into(), "<-".into()), vec![10, 11]);
        let mut lines = consolidated_pass_lines(&pass_map);
        lines.sort_unstable();
        assert_eq!(lines, vec!["1kb tcp 1-2 ->", "1kb tcp 10-11 <-"]);
    }
}
