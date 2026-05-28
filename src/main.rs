use bimap::cli::{parse, Command};
use bimap::control::msg::Message;
use std::net::IpAddr;
use std::net::ToSocketAddrs;
use std::process;

fn main() {
    let command = match parse() {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("bimap: {e}");
            process::exit(2);
        }
    };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let exit_code = rt.block_on(async {
        match command {
            Command::Server { bind, verbose } => {
                use bimap::control::channel_from_tls_stream;
                use bimap::control::tls::{
                    generate_ephemeral_cert, make_tls_acceptor, server_tls_accept,
                };
                use bimap::orchestrator;
                use bimap::test::build_registry;

                let (cert, key, fingerprint) = match generate_ephemeral_cert() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("bimap: {e}");
                        return 2;
                    }
                };
                eprintln!("[bimap-server] fingerprint: {fingerprint}");

                let acceptor = match make_tls_acceptor(cert, key) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("bimap: {e}");
                        return 2;
                    }
                };

                let listener = match bind_with_reuse(&bind).await {
                    Ok(l) => l,
                    Err((bind, e)) => {
                        let has_port_443 = bind.rfind(':').is_some_and(|pos| &bind[pos + 1..] == "443");
                        let hint = if has_port_443 && e.kind() == std::io::ErrorKind::PermissionDenied {
                            " (port 443 requires root/sudo; use --bind with port > 1024)"
                        } else if e.kind() == std::io::ErrorKind::AddrInUse {
                            " (address already in use)"
                        } else {
                            ""
                        };
                        eprintln!("bimap: cannot bind {bind}: {e}{hint}");
                        return 3;
                    }
                };
                eprintln!("[bimap-server] listening on {bind}");

                loop {
                    let (tls_stream, peer) = match server_tls_accept(&acceptor, &listener).await {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("bimap: accept error: {e}");
                            continue;
                        }
                    };
                    eprintln!("[bimap-server] connection from {peer}");

                    let registry = build_registry();
                    let mut channel = channel_from_tls_stream(tls_stream, verbose);

                    if let Err(e) = channel
                        .send(&Message::Hello {
                            version: 1,
                            fingerprint: fingerprint.clone(),
                        })
                        .await
                    {
                        eprintln!("bimap: send hello: {e}");
                        continue;
                    }

                    match orchestrator::run_server(channel, &registry, 5000).await {
                        Ok(summary) => {
                            eprintln!(
                                "[bimap-server] done: {} passed, {} failed, {} errors",
                                summary.passed, summary.failed, summary.errors
                            );
                        }
                        Err(e) => {
                            eprintln!("bimap: server session error: {e}");
                        }
                    }
                }
            }

            Command::Client {
                server,
                port,
                control_server,
                target,
                test,
                port_range,
                bidir,
                timeout,
                fingerprint,
                json,
                json_export,
                quiet: _,
                verbose,
            } => {
                let (server, port) = if let Some(ref cs) = control_server {
                    let (ip_str, port_str) = match cs.split_once(':') {
                        Some(p) => p,
                        None => {
                            eprintln!("bimap: --control-server must be ip:port");
                            return 2;
                        }
                    };
                    let ip: IpAddr = match ip_str.parse() {
                        Ok(ip) => ip,
                        Err(_) => {
                            eprintln!("bimap: bad IP in --control-server");
                            return 2;
                        }
                    };
                    let port: u16 = match port_str.parse() {
                        Ok(p) => p,
                        Err(_) => {
                            eprintln!("bimap: bad port in --control-server");
                            return 2;
                        }
                    };
                    (ip, port)
                } else {
                    (server, port)
                };
                let target = target.unwrap_or(server);
                use bimap::control::channel_from_client_tls;
                use bimap::control::tls::{client_tls_connect, make_tls_connector};
                use bimap::orchestrator;
                use bimap::test::build_registry;

                if test.is_empty() {
                    let registry = build_registry();
                    let names = registry.names();
                    println!("available tests:");
                    for name in names {
                        let Some(proto) = registry.find(name) else {
                            continue;
                        };
                        let transports: Vec<&str> =
                            proto.transports().iter().map(|t| t.as_str()).collect();
                        println!(
                            "  {:<12} layer={:?} transports={}",
                            name,
                            proto.layer(),
                            transports.join(",")
                        );
                    }
                    return 0;
                }
                if port_range.is_empty() {
                    let registry = build_registry();
                    let has_l4 = test.iter().any(|name| {
                        registry
                            .find(name)
                            .is_some_and(|p| p.layer() != bimap::test::Layer::L3)
                    });
                    if has_l4 {
                        eprintln!("bimap: --port-range required for L4/L7 tests (e.g. --port-range tcp/1-1024)");
                        eprintln!("       ICMP tests (icmp-ping, icmp-full) work without --port-range");
                        return 2;
                    }
                }

                let connector = match make_tls_connector() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("bimap: {e}");
                        return 2;
                    }
                };

                let control_target = format!("{server}:{port}");
                let tls_stream = match client_tls_connect(&connector, &control_target).await {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("bimap: cannot connect to {control_target}: {e}");
                        return 3;
                    }
                };

                let mut channel = channel_from_client_tls(tls_stream, verbose as u8);

                let hello = match channel.recv().await {
                    Ok(Message::Hello {
                        version: _,
                        fingerprint: fp,
                    }) => fp,
                    Ok(_) => {
                        eprintln!("bimap: unexpected message from server");
                        return 3;
                    }
                    Err(e) => {
                        eprintln!("bimap: recv hello: {e}");
                        return 3;
                    }
                };

                eprintln!("[bimap-client] server fingerprint: {hello}");

                if let Some(ref expected) = fingerprint {
                    if hello != *expected
                        && format!("SHA256:{hello}") != *expected
                        && hello != expected.replace("SHA256:", "")
                    {
                        eprintln!("[bimap-client] fingerprint mismatch!");
                        return 3;
                    }
                    eprintln!("[bimap-client] fingerprint verified");
                }

                let port_ranges: Vec<(String, u16, u16)> = port_range
                    .iter()
                    .filter_map(|spec| {
                        let parts: Vec<&str> = spec.splitn(2, '/').collect();
                        if parts.len() != 2 {
                            eprintln!("bimap: invalid port-range: {spec}");
                            return None;
                        }
                        let transport = parts[0].to_string();
                        if parts[1] == "any" || parts[1] == "icmp" {
                            Some((transport, 0, 0))
                        } else {
                            let range_parts: Vec<&str> = parts[1].splitn(2, '-').collect();
                            let start = range_parts[0].parse().ok()?;
                            let end = range_parts
                                .get(1)
                                .and_then(|e| e.parse().ok())
                                .unwrap_or(start);
                            Some((transport, start, end))
                        }
                    })
                    .collect();

                let config = bimap::orchestrator::ClientConfig {
                    tests: test,
                    port_ranges,
                    bidir,
                    timeout_ms: timeout,
                    server_addr: server,
                    target_addr: target,
                    json,
                    json_export,
                    verbose,
                };

                let registry = build_registry();
                match orchestrator::run_client(channel, &registry, &config).await {
                    Ok(summary) => {
                        eprintln!(
                            "[bimap-client] done: {} passed, {} failed, {} errors",
                            summary.passed, summary.failed, summary.errors
                        );
                        if summary.failed > 0 || summary.errors > 0 {
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("bimap: client error: {e}");
                        3
                    }
                }
            }
        }
    });

    process::exit(exit_code);
}

async fn bind_with_reuse(addr: &str) -> Result<tokio::net::TcpListener, (String, std::io::Error)> {
    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| (addr.to_string(), e))?
        .next()
        .ok_or_else(|| {
            (
                addr.to_string(),
                std::io::Error::other("no address resolved"),
            )
        })?;

    let domain = if sock_addr.is_ipv4() {
        tokio::net::TcpSocket::new_v4()
    } else {
        tokio::net::TcpSocket::new_v6()
    }
    .map_err(|e| (addr.to_string(), e))?;

    domain
        .set_reuseaddr(true)
        .map_err(|e| (addr.to_string(), e))?;
    domain.bind(sock_addr).map_err(|e| (addr.to_string(), e))?;
    domain.listen(1024).map_err(|e| (addr.to_string(), e))
}
