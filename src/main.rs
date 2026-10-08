use bimap::cli::{parse, parse_port_ranges, Command};
use bimap::control::msg::{Message, PROTOCOL_VERSION};
use bimap::output;
use std::net::{SocketAddr, ToSocketAddrs};
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
                            version: PROTOCOL_VERSION,
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
                let control_target = if let Some(ref cs) = control_server {
                    match cs.parse::<SocketAddr>() {
                        Ok(address) => address,
                        Err(_) => {
                            error!("--control-server must be ip:port (IPv6: [::1]:443)");
                            return 2;
                        }
                    }
                } else {
                    match server {
                        Some(ref hostname) => {
                            let hostname = strip_ipv6_brackets(hostname);
                            match (hostname, port).to_socket_addrs() {
                                Ok(mut addresses) => match addresses.next() {
                                    Some(address) => address,
                                    None => {
                                        error!("could not resolve server '{hostname}'");
                                        return 2;
                                    }
                                },
                                Err(error) => {
                                    error!("could not resolve server '{hostname}': {error}");
                                    return 2;
                                }
                            }
                        }
                        None => {
                            error!("--server or --control-server is required");
                            return 2;
                        }
                    }
                };
                let explicit_target = target.is_some();
                let target_str = target.unwrap_or_else(|| control_target.ip().to_string());
                let target_addr: SocketAddr = if !explicit_target {
                    let mut address = control_target;
                    address.set_port(0);
                    address
                } else {
                    let hostname = strip_ipv6_brackets(&target_str);
                    match (hostname, 0).to_socket_addrs() {
                        Ok(mut addresses) => match addresses.next() {
                            Some(address) => address,
                            None => {
                                error!("could not resolve '{target_str}': no addresses");
                                return 2;
                            }
                        },
                        Err(error) => {
                            error!("could not resolve '{target_str}': {error}");
                            return 2;
                        }
                    }
                };
                use bimap::control::channel_from_client_tls;
                use bimap::control::tls::{client_tls_connect, make_pinned_tls_connector};
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

                let port_ranges = match parse_port_ranges(&port_range) {
                    Ok(ranges) => ranges,
                    Err(error) => {
                        error!("{error}");
                        return 2;
                    }
                };

                let connector = match make_pinned_tls_connector(fingerprint.as_deref()) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("{e}");
                        return 2;
                    }
                };

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
                        version,
                        fingerprint: fp,
                    }) => {
                        if version != PROTOCOL_VERSION {
                            error!(
                                "server protocol version {version} does not match client version {PROTOCOL_VERSION}"
                            );
                            return 3;
                        }
                        fp
                    }
                    Ok(_) => {
                        error!("unexpected message from server");
                        return 3;
                    }
                    Err(e) => {
                        error!("recv hello: {e}");
                        return 3;
                    }
                };

                debug!("server advertised fingerprint: {hello}");

                if fingerprint.is_some() {
                    info!("fingerprint verified");
                }

                let config = bimap::orchestrator::ClientConfig {
                    tests: test,
                    port_ranges,
                    bidir,
                    timeout_ms: timeout,
                    parallel,
                    server_addr: control_target.ip(),
                    target_addr,
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
