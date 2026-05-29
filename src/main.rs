use bimap::cli::{parse, Command};
use bimap::control::msg::Message;
use bimap::output;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::process;
use tracing::{debug, error, info};

fn strip_ipv6_brackets(s: &str) -> &str {
    s.strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(s)
}

fn main() {
    let command = match parse() {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("bimap: {e}");
            process::exit(2);
        }
    };

    let verbose = match &command {
        Command::Server { verbose, .. } => *verbose,
        Command::Client { verbose, .. } => *verbose,
    };

    output::init_tracing(verbose);

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
                        error!("{e}");
                        return 2;
                    }
                };
                info!("fingerprint: {fingerprint}");

                let acceptor = match make_tls_acceptor(cert, key) {
                    Ok(a) => a,
                    Err(e) => {
                        error!("{e}");
                        return 2;
                    }
                };

                let listener = match bind_with_reuse(&bind).await {
                    Ok(l) => l,
                    Err((bind, e)) => {
                        let has_port_443 =
                            bind.rfind(':').is_some_and(|pos| &bind[pos + 1..] == "443");
                        let hint =
                            if has_port_443 && e.kind() == std::io::ErrorKind::PermissionDenied {
                                " (port 443 requires root/sudo; use --bind with port > 1024)"
                            } else if e.kind() == std::io::ErrorKind::AddrInUse {
                                " (address already in use)"
                            } else {
                                ""
                            };
                        error!("cannot bind {bind}: {e}{hint}");
                        return 3;
                    }
                };
                info!("listening on {bind}");

                loop {
                    let (tls_stream, peer) = match server_tls_accept(&acceptor, &listener).await {
                        Ok(r) => r,
                        Err(e) => {
                            error!("accept error: {e}");
                            continue;
                        }
                    };
                    info!("connection from {peer}");

                    let registry = build_registry();
                    let mut channel = channel_from_tls_stream(tls_stream, verbose);

                    if let Err(e) = channel
                        .send(&Message::Hello {
                            version: 1,
                            fingerprint: fingerprint.clone(),
                        })
                        .await
                    {
                        error!("send hello: {e}");
                        continue;
                    }

                    match orchestrator::run_server(channel, &registry).await {
                        Ok(summary) => {
                            info!(
                                "done: {} passed, {} failed, {} errors",
                                summary.passed, summary.failed, summary.errors
                            );
                        }
                        Err(e) => {
                            error!("server session error: {e}");
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
                parallel,
                verbose,
                quiet,
            } => {
                let (server, port) = if let Some(ref cs) = control_server {
                    let sock_addr: SocketAddr = match cs.parse() {
                        Ok(a) => a,
                        Err(_) => {
                            error!("--control-server must be ip:port (IPv6: [::1]:443)");
                            return 2;
                        }
                    };
                    (sock_addr.ip(), sock_addr.port())
                } else {
                    match server {
                        Some(ref s) => {
                            let s = strip_ipv6_brackets(s);
                            let ip: IpAddr = match s.parse() {
                                Ok(ip) => ip,
                                Err(_) => match (s, port).to_socket_addrs() {
                                    Ok(mut addrs) => match addrs.next() {
                                        Some(a) => a.ip(),
                                        None => {
                                            error!("could not resolve server '{s}'");
                                            return 2;
                                        }
                                    },
                                    Err(e) => {
                                        error!("could not resolve server '{s}': {e}");
                                        return 2;
                                    }
                                },
                            };
                            (ip, port)
                        }
                        None => {
                            error!("--server or --control-server is required");
                            return 2;
                        }
                    }
                };
                let target_str = target.unwrap_or_else(|| server.to_string());
                let target_ip: IpAddr = {
                    let s = strip_ipv6_brackets(&target_str);
                    match s.parse() {
                        Ok(ip) => ip,
                        Err(_) => match (s, 0).to_socket_addrs() {
                            Ok(mut addrs) => match addrs.next() {
                                Some(a) => a.ip(),
                                None => {
                                    error!("could not resolve '{target_str}': no addresses");
                                    return 2;
                                }
                            },
                            Err(e) => {
                                error!("could not resolve '{target_str}': {e}");
                                return 2;
                            }
                        },
                    }
                };
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
                        error!(
                            "--port-range required for L4/L7 tests (e.g. --port-range tcp/1-1024)"
                        );
                        error!(
                            "       ICMP tests (icmp-ping, icmp-full) work without --port-range"
                        );
                        return 2;
                    }
                }

                let connector = match make_tls_connector() {
                    Ok(c) => c,
                    Err(e) => {
                        error!("{e}");
                        return 2;
                    }
                };

                let control_target = SocketAddr::new(server, port);
                let tls_stream = match client_tls_connect(&connector, control_target).await {
                    Ok(s) => s,
                    Err(e) => {
                        error!("cannot connect to {control_target}: {e}");
                        return 3;
                    }
                };

                let mut channel = channel_from_client_tls(tls_stream, verbose);

                let hello = match channel.recv().await {
                    Ok(Message::Hello {
                        version: _,
                        fingerprint: fp,
                    }) => fp,
                    Ok(_) => {
                        error!("unexpected message from server");
                        return 3;
                    }
                    Err(e) => {
                        error!("recv hello: {e}");
                        return 3;
                    }
                };

                debug!("server fingerprint: {hello}");

                if let Some(ref expected) = fingerprint {
                    if hello != *expected
                        && format!("SHA256:{hello}") != *expected
                        && hello != expected.replace("SHA256:", "")
                    {
                        error!("fingerprint mismatch!");
                        return 3;
                    }
                    info!("fingerprint verified");
                }

                let port_ranges: Vec<(String, u16, u16)> = port_range
                    .iter()
                    .filter_map(|spec| {
                        let parts: Vec<&str> = spec.splitn(2, '/').collect();
                        if parts.len() != 2 {
                            error!("invalid port-range: {spec}");
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
                    parallel,
                    server_addr: server,
                    target_str,
                    target_ip,
                    json,
                    json_export,
                    verbose,
                    quiet,
                };

                let registry = build_registry();
                match orchestrator::run_client(channel, &registry, &config).await {
                    Ok(summary) => {
                        if summary.failed > 0 || summary.errors > 0 {
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        error!("client error: {e}");
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
