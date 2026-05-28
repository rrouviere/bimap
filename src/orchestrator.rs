use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tracing::{debug, trace};

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
    let (timeout_ms, parallel, test_bind_ip) = match channel.recv().await? {
        Message::Configure {
            target,
            timeout_ms,
            client_version,
            parallel,
            ..
        } => {
            if client_version < PROTOCOL_VERSION {
                channel
                    .send(&Message::Ack {
                        ok: false,
                        message: Some(format!(
                            "client version {client_version} too old, need {PROTOCOL_VERSION}"
                        )),
                    })
                    .await?;
                return Err("client version too old, update client".into());
            }
            let bind_ip: Option<IpAddr> = target.as_ref().and_then(|t| t.parse().ok());
            channel
                .send(&Message::Ack {
                    ok: true,
                    message: None,
                })
                .await?;
            (timeout_ms, parallel, bind_ip)
        }
        _ => return Err("expected Configure message".into()),
    };

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errors = 0u32;

    loop {
        // Read first message: must be Test or Done
        let first = channel.recv().await?;
        let (proto, mut batch, mut done_after) = match first {
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
                let bind_ip = test_bind_ip.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
                let target_addr = SocketAddr::new(bind_ip, port);
                let batch = vec![ServerBatchEntry {
                    id,
                    transport,
                    dir: server_dir,
                    port,
                    target_addr,
                }];
                (proto, batch, false)
            }
            _ => return Err("unexpected message, expected Test or Done".into()),
        };

        // Collect more Test messages — client sent them in one burst
        loop {
            if batch.len() >= parallel {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(10), channel.recv()).await {
                Ok(Ok(Message::Test {
                    id,
                    protocol: _,
                    transport,
                    port,
                    direction,
                })) => {
                    let transport = Transport::from_str(&transport)
                        .ok_or_else(|| format!("unknown transport: {transport}"))?;
                    let dir = parse_direction(&direction)?;
                    let server_dir = match dir {
                        Direction::ClientToServer => Direction::ServerToClient,
                        Direction::ServerToClient => Direction::ClientToServer,
                    };
                    let bind_ip = test_bind_ip.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
                    let target_addr = SocketAddr::new(bind_ip, port);
                    batch.push(ServerBatchEntry {
                        id,
                        transport,
                        dir: server_dir,
                        port,
                        target_addr,
                    });
                }
                Ok(Ok(Message::Done)) => {
                    done_after = true;
                    break;
                }
                Ok(Ok(_)) => return Err("unexpected message".into()),
                Ok(Err(e)) => return Err(e),
                Err(_) => break, // 10ms timeout, batch likely complete
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
                Box::pin(async move { (entry.id, proto.run(ctx).await) })
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

pub struct ClientConfig {
    pub tests: Vec<String>,
    pub port_ranges: Vec<(String, u16, u16)>,
    pub bidir: bool,
    pub timeout_ms: u64,
    pub parallel: usize,
    pub server_addr: IpAddr,
    pub target_addr: IpAddr,
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

struct ServerBatchEntry {
    id: u32,
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

    let mut unordered: FuturesUnordered<_> = batch
        .iter()
        .map(|entry| {
            let target_addr = SocketAddr::new(config.target_addr, entry.port);
            let ctx = TestContext {
                direction: entry.direction,
                transport: entry.transport,
                port: entry.port,
                target_addr,
                timeout: Duration::from_millis(config.timeout_ms),
            };
            Box::pin(async move { (entry.id, proto.run(ctx).await) })
        })
        .collect();

    use std::collections::HashMap as PortMap;

    let first_id = batch.first().map(|e| e.id).unwrap_or(0);
    let mut pending: HashMap<u32, ProtocolResult> = HashMap::new();
    let mut next_print = first_id;
    let interactive = is_interactive() && !config.json && !config.json_export;
    let mut fail_map: PortMap<(String, String, String, String), Vec<u16>> = PortMap::new();

    while let Some((id, result)) = unordered.next().await {
        match &result {
            ProtocolResult::Pass { .. } => *passed += 1,
            ProtocolResult::Fail { .. } => *failed += 1,
            ProtocolResult::Error { .. } => *errors += 1,
        }

        pending.insert(id, result);

        // Drain all sequentially available results
        while let Some(result) = pending.remove(&next_print) {
            if let Some(entry) = batch.iter().find(|e| e.id == next_print) {
                if config.json && !config.json_export {
                    print_result(
                        entry.id,
                        entry.test_name,
                        entry.transport_str,
                        entry.port,
                        entry.direction,
                        &result,
                        true,
                        None,
                    );
                } else if !config.json_export {
                    match &result {
                        ProtocolResult::Pass {
                            sent_bytes,
                            received_bytes,
                        } => {
                            // Interactive mode: condense — skip per-line PASS
                            if !interactive && !config.quiet {
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
                            if interactive {
                                let key = (
                                    entry.test_name.to_string(),
                                    entry.transport_str.to_string(),
                                    entry.direction.as_str().to_string(),
                                    reason.clone(),
                                );
                                fail_map.entry(key).or_default().push(entry.port);
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
                            if interactive {
                                let key = (
                                    entry.test_name.to_string(),
                                    entry.transport_str.to_string(),
                                    entry.direction.as_str().to_string(),
                                    reason.clone(),
                                );
                                fail_map.entry(key).or_default().push(entry.port);
                                let mut line = String::new();
                                for ((tn, ts, dir, r), ports) in &fail_map {
                                    if !line.is_empty() {
                                        line.push_str("  ");
                                    }
                                    let ranges = format_port_ranges(ports);
                                    line.push_str(&format!("{tn} {ts} {ranges} {dir} {r}"));
                                }
                                print_fail_live(&line);
                            } else if !config.quiet {
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
            }
            next_print += 1;
        }
    }

    if interactive {
        finish_fail_line();
        // Print consolidated fail lines
        for ((tn, ts, dir, r), ports) in &fail_map {
            let ranges = format_port_ranges(ports);
            print_fail(format_args!("{tn} {ts} {ranges} {dir} {r}"));
        }
    }

    // Read server Reports, match by ID (not position) to avoid desync
    let mut expected: HashSet<u32> = batch.iter().map(|e| e.id).collect();
    // Use a slightly longer timeout than the per-test timeout to avoid
    // racing with the last server report (both fire at ~same time).
    let recv_timeout = Duration::from_millis(config.timeout_ms + 100);
    while !expected.is_empty() {
        let recv_result = tokio::time::timeout(recv_timeout, channel.recv()).await;

        match recv_result {
            Ok(Ok(Message::Report { error, id, .. })) => {
                if expected.remove(&id) {
                    if let Some(result) = pending.remove(&id) {
                        if let Some(entry) = batch.iter().find(|e| e.id == id) {
                            if let Some(ref err_msg) = error {
                                if !matches!(result, ProtocolResult::Pass { .. }) {
                                    debug!("server: {err_msg}");
                                }
                            }
                            results.push(TestEntry {
                                protocol: entry.test_name.to_string(),
                                transport: entry.transport_str.to_string(),
                                port: entry.port,
                                direction: entry.direction,
                                result,
                                server_error: error,
                            });
                        }
                    }
                }
            }
            Ok(Ok(_)) => {
                debug!("server: unexpected message");
            }
            Ok(Err(e)) => {
                debug!("server: {e}");
            }
            Err(_) => {
                // Server didn't respond in time — use local results
                for id in expected.drain() {
                    if let Some(result) = pending.remove(&id) {
                        if let Some(entry) = batch.iter().find(|e| e.id == id) {
                            results.push(TestEntry {
                                protocol: entry.test_name.to_string(),
                                transport: entry.transport_str.to_string(),
                                port: entry.port,
                                direction: entry.direction,
                                result,
                                server_error: None,
                            });
                        }
                    }
                }
            }
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

            if config.parallel <= 1 {
                for port in *start..=*end {
                    let directions = if config.bidir {
                        vec![Direction::ClientToServer, Direction::ServerToClient]
                    } else {
                        vec![Direction::ClientToServer]
                    };

                    for dir in directions {
                        let target_addr = SocketAddr::new(config.target_addr, port);

                        debug!("orchestrator sending test {} to server", id);

                        channel
                            .send(&Message::Test {
                                id,
                                protocol: test_name.clone(),
                                transport: transport_str.clone(),
                                port,
                                direction: dir.as_str().to_string(),
                            })
                            .await?;

                        let quick =
                            tokio::time::timeout(Duration::from_millis(500), channel.recv()).await;

                        let (result, server_error) = match quick {
                            Ok(Ok(Message::Report {
                                error: Some(err),
                                sent: None,
                                received: None,
                                ..
                            })) => {
                                if config.verbose >= 1 {
                                    debug!(
                                        "skip test {} {}/{} {} (server: {err})",
                                        test_name,
                                        transport_str,
                                        port,
                                        dir.as_str()
                                    );
                                }
                                errors += 1;
                                (
                                    ProtocolResult::Error {
                                        reason: format!("server: {err}"),
                                    },
                                    Some(err),
                                )
                            }
                            Ok(Ok(Message::Report {
                                error: Some(err),
                                sent,
                                received,
                                ..
                            })) => {
                                let sb = sent.as_ref().map(|t| t.bytes).unwrap_or(0);
                                let rb = received.as_ref().map(|t| t.bytes).unwrap_or(0);
                                failed += 1;
                                (
                                    ProtocolResult::Fail {
                                        reason: err.clone(),
                                        sent_bytes: sb,
                                        received_bytes: rb,
                                    },
                                    Some(err),
                                )
                            }
                            Ok(Ok(Message::Report { sent, received, .. })) => {
                                let sb = sent.as_ref().map(|t| t.bytes).unwrap_or(0);
                                let rb = received.as_ref().map(|t| t.bytes).unwrap_or(0);
                                passed += 1;
                                (
                                    ProtocolResult::Pass {
                                        sent_bytes: sb,
                                        received_bytes: rb,
                                    },
                                    None,
                                )
                            }
                            _ => {
                                let ctx = TestContext {
                                    direction: dir,
                                    transport,
                                    port,
                                    target_addr,
                                    timeout: Duration::from_millis(config.timeout_ms),
                                };

                                trace!(
                                    "test {} {}/{} port={} {}",
                                    test_name,
                                    transport_str,
                                    port,
                                    port,
                                    dir.as_str()
                                );

                                let r = match tokio::time::timeout(
                                    Duration::from_millis(config.timeout_ms + 2000),
                                    proto.run(ctx),
                                )
                                .await
                                {
                                    Ok(r) => r,
                                    Err(_) => ProtocolResult::Fail {
                                        reason: "timeout (test took too long)".into(),
                                        sent_bytes: 0,
                                        received_bytes: 0,
                                    },
                                };

                                match &r {
                                    ProtocolResult::Pass { .. } => passed += 1,
                                    ProtocolResult::Fail { .. } => failed += 1,
                                    ProtocolResult::Error { .. } => errors += 1,
                                }

                                debug!("orchestrator waiting for report {}", id);

                                let se = match tokio::time::timeout(
                                    Duration::from_millis(config.timeout_ms),
                                    channel.recv(),
                                )
                                .await
                                {
                                    Ok(Ok(Message::Report { error, .. })) => error,
                                    Ok(Ok(_)) => {
                                        debug!("server: unexpected message (skipping)");
                                        None
                                    }
                                    Ok(Err(e)) => {
                                        debug!("server: {e}");
                                        None
                                    }
                                    Err(_) => {
                                        debug!(
                                            "server: timeout (no report within {}ms)",
                                            config.timeout_ms
                                        );
                                        None
                                    }
                                };

                                (r, se)
                            }
                        };

                        if let Some(ref err_msg) = server_error {
                            if !matches!(result, ProtocolResult::Pass { .. }) {
                                debug!("server: {err_msg}");
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
                        } else if !config.json_export {
                            match &result {
                                ProtocolResult::Pass {
                                    sent_bytes,
                                    received_bytes,
                                } => {
                                    print_pass(format_args!(
                                        "{} {} {} {} (tx={} rx={})",
                                        test_name,
                                        transport_str,
                                        port,
                                        dir.as_str(),
                                        sent_bytes,
                                        received_bytes
                                    ));
                                }
                                ProtocolResult::Fail {
                                    reason,
                                    sent_bytes,
                                    received_bytes,
                                } => {
                                    print_fail(format_args!(
                                        "{} {} {} {} {} (tx={} rx={})",
                                        test_name,
                                        transport_str,
                                        port,
                                        dir.as_str(),
                                        reason,
                                        sent_bytes,
                                        received_bytes
                                    ));
                                }
                                ProtocolResult::Error { reason } => {
                                    print_err(format_args!(
                                        "{} {} {} {} {}",
                                        test_name,
                                        transport_str,
                                        port,
                                        dir.as_str(),
                                        reason
                                    ));
                                }
                            }
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
            } else {
                let mut batch: Vec<BatchEntry> = Vec::new();
                let batch_size = config.parallel.max(1);

                for port in *start..=*end {
                    let directions = if config.bidir {
                        vec![Direction::ClientToServer, Direction::ServerToClient]
                    } else {
                        vec![Direction::ClientToServer]
                    };

                    for dir in directions {
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
}
