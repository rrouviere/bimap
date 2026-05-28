use crate::orchestrator::ProtocolResult;
use crate::packet::icmp::{ALL_ICMP_TYPES, ICMP_TYPE_ECHO_REPLY};
use crate::test::{Direction, Layer, TestContext, TestProtocol, Transport};
use async_trait::async_trait;
use std::net::Ipv4Addr;
use std::time::Duration;

fn has_icmp_capability() -> bool {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_ICMP) };
    if fd >= 0 {
        unsafe { libc::close(fd) };
        return true;
    }
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_RAW, libc::IPPROTO_ICMP) };
    if fd >= 0 {
        unsafe { libc::close(fd) };
        return true;
    }
    false
}

fn create_raw_socket() -> Result<i32, String> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_RAW, libc::IPPROTO_ICMP) };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!("socket: {err}"));
    }
    Ok(fd)
}

unsafe fn set_socket_timeout(fd: i32, timeout: Duration) -> Result<(), String> {
    let tv = libc::timeval {
        tv_sec: timeout.as_secs() as libc::time_t,
        tv_usec: timeout.subsec_micros() as libc::suseconds_t,
    };
    let ret = libc::setsockopt(
        fd,
        libc::SOL_SOCKET,
        libc::SO_RCVTIMEO,
        &tv as *const _ as *const libc::c_void,
        std::mem::size_of::<libc::timeval>() as libc::socklen_t,
    );
    if ret < 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!("setsockopt: {err}"));
    }
    Ok(())
}

fn to_sockaddr_in(addr: Ipv4Addr) -> libc::sockaddr_in {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_port = 0;
    sa.sin_addr = libc::in_addr {
        s_addr: u32::from_be_bytes(addr.octets()),
    };
    sa
}

fn build_icmp_echo(icmp_type: u8, id: u16, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(8 + payload.len());
    packet.push(icmp_type);
    packet.push(0);
    packet.extend_from_slice(&[0u8; 2]);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&seq.to_be_bytes());
    packet.extend_from_slice(payload);
    let csum = pnet_packet::util::checksum(&packet, packet.len());
    packet[2..4].copy_from_slice(&csum.to_be_bytes());
    packet
}

fn ip_header_len(buf: &[u8]) -> Option<usize> {
    let packet = pnet_packet::ipv4::Ipv4Packet::new(buf)?;
    Some((packet.get_header_length() as usize) * 4)
}

async fn send_icmp_echo(
    dest: Ipv4Addr,
    id: u16,
    seq: u16,
    icmp_type: u8,
    timeout: Duration,
    verbose: bool,
) -> ProtocolResult {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_ICMP) };
    if fd < 0 {
        return send_icmp_echo_raw(dest, id, seq, icmp_type, timeout, verbose).await;
    }

    if let Err(e) = unsafe { set_socket_timeout(fd, timeout) } {
        unsafe { libc::close(fd) };
        return ProtocolResult::Error { reason: e };
    }

    let payload = b"bimap";
    let sa = to_sockaddr_in(dest);

    if verbose {
        eprintln!("[v] icmp ping-socket sending echo request to {dest}");
    }

    let sent = unsafe {
        libc::sendto(
            fd,
            payload.as_ptr() as *const libc::c_void,
            payload.len(),
            0,
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if sent < 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return ProtocolResult::Error {
            reason: format!("sendto: {err}"),
        };
    }

    if verbose {
        eprintln!(
            "[v] icmp waiting for echo reply (timeout={}ms)",
            timeout.as_millis()
        );
    }

    let mut recv_buf = [0u8; 1500];
    let received = unsafe {
        libc::recvfrom(
            fd,
            recv_buf.as_mut_ptr() as *mut libc::c_void,
            recv_buf.len(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };

    unsafe { libc::close(fd) };

    if received < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock
            || err.kind() == std::io::ErrorKind::TimedOut
        {
            return ProtocolResult::Fail {
                reason: "timeout".into(),
                sent_bytes: sent as u64,
                received_bytes: 0,
            };
        }
        return ProtocolResult::Fail {
            reason: format!("no-reply: {err}"),
            sent_bytes: sent as u64,
            received_bytes: 0,
        };
    }

    ProtocolResult::Pass {
        sent_bytes: sent as u64,
        received_bytes: received as u64,
    }
}

async fn send_icmp_echo_raw(
    dest: Ipv4Addr,
    id: u16,
    seq: u16,
    icmp_type: u8,
    timeout: Duration,
    verbose: bool,
) -> ProtocolResult {
    if !has_icmp_capability() {
        return ProtocolResult::Error {
            reason: "permission: ICMP requires root or CAP_NET_RAW".into(),
        };
    }

    let fd = match create_raw_socket() {
        Ok(f) => f,
        Err(e) => {
            return ProtocolResult::Error { reason: e };
        }
    };

    if let Err(e) = unsafe { set_socket_timeout(fd, timeout) } {
        unsafe {
            libc::close(fd);
        }
        return ProtocolResult::Error { reason: e };
    }

    let payload = b"bimap";
    let packet = build_icmp_echo(icmp_type, id, seq, payload);

    let sa = to_sockaddr_in(dest);
    if verbose {
        eprintln!("[v] icmp raw sending type={} seq={}", icmp_type, seq);
    }
    let sent = unsafe {
        libc::sendto(
            fd,
            packet.as_ptr() as *const libc::c_void,
            packet.len(),
            0,
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if sent < 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return ProtocolResult::Error {
            reason: format!("sendto: {err}"),
        };
    }

    if verbose {
        eprintln!(
            "[v] icmp raw waiting for reply (timeout={}ms)",
            timeout.as_millis()
        );
    }
    let start = std::time::Instant::now();
    loop {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            unsafe { libc::close(fd) };
            return ProtocolResult::Fail {
                reason: "timeout".into(),
                sent_bytes: sent as u64,
                received_bytes: 0,
            };
        }
        let remaining = timeout - elapsed;

        if let Err(e) = unsafe { set_socket_timeout(fd, remaining) } {
            unsafe { libc::close(fd) };
            return ProtocolResult::Error { reason: e };
        }

        let mut recv_buf = [0u8; 1500];
        let received = unsafe {
            libc::recvfrom(
                fd,
                recv_buf.as_mut_ptr() as *mut libc::c_void,
                recv_buf.len(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };

        if received < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            if err.kind() == std::io::ErrorKind::WouldBlock
                || err.kind() == std::io::ErrorKind::TimedOut
            {
                return ProtocolResult::Fail {
                    reason: "timeout".into(),
                    sent_bytes: sent as u64,
                    received_bytes: 0,
                };
            }
            return ProtocolResult::Fail {
                reason: format!("no-reply: {err}"),
                sent_bytes: sent as u64,
                received_bytes: 0,
            };
        }

        let n = received as usize;
        let icmp_offset = match ip_header_len(&recv_buf[..n]) {
            Some(offset) => offset,
            None => continue,
        };

        if icmp_offset + 8 > n {
            continue;
        }

        let reply_type = recv_buf[icmp_offset];
        let reply_id = u16::from_be_bytes([recv_buf[icmp_offset + 4], recv_buf[icmp_offset + 5]]);

        if reply_type == ICMP_TYPE_ECHO_REPLY && reply_id == id {
            unsafe { libc::close(fd) };
            return ProtocolResult::Pass {
                sent_bytes: sent as u64,
                received_bytes: n as u64,
            };
        }
    }
}

pub struct IcmpPingTest;

#[async_trait]
impl TestProtocol for IcmpPingTest {
    fn name(&self) -> &'static str {
        "icmp-ping"
    }

    fn layer(&self) -> Layer {
        Layer::L3
    }

    fn transports(&self) -> &[Transport] {
        &[Transport::Icmp]
    }

    async fn run(&self, ctx: TestContext) -> ProtocolResult {
        if ctx.direction == Direction::ServerToClient {
            return ProtocolResult::Pass {
                sent_bytes: 0,
                received_bytes: 0,
            };
        }

        let dest = match ctx.target_addr.ip() {
            std::net::IpAddr::V4(ip) => ip,
            std::net::IpAddr::V6(_) => {
                return ProtocolResult::Error {
                    reason: "IPv6 not supported".into(),
                };
            }
        };

        send_icmp_echo(dest, 0x42, 1, 8, ctx.timeout, ctx.verbose).await
    }
}

pub struct IcmpFullTest;

#[async_trait]
impl TestProtocol for IcmpFullTest {
    fn name(&self) -> &'static str {
        "icmp-full"
    }

    fn layer(&self) -> Layer {
        Layer::L3
    }

    fn transports(&self) -> &[Transport] {
        &[Transport::Icmp]
    }

    async fn run(&self, ctx: TestContext) -> ProtocolResult {
        if !has_icmp_capability() {
            return ProtocolResult::Error {
                reason: "permission: ICMP requires root or CAP_NET_RAW".into(),
            };
        }

        if ctx.direction == Direction::ServerToClient {
            return ProtocolResult::Pass {
                sent_bytes: 0,
                received_bytes: 0,
            };
        }

        let dest = match ctx.target_addr.ip() {
            std::net::IpAddr::V4(ip) => ip,
            std::net::IpAddr::V6(_) => {
                return ProtocolResult::Error {
                    reason: "IPv6 not supported".into(),
                };
            }
        };

        let mut result_count = 0u64;
        for (idx, &(icmp_type, name)) in ALL_ICMP_TYPES.iter().enumerate() {
            let result = send_icmp_echo(
                dest,
                icmp_type as u16,
                idx as u16,
                icmp_type,
                ctx.timeout,
                ctx.verbose,
            )
            .await;
            match &result {
                ProtocolResult::Pass { .. } => {
                    eprintln!("icmp-full: type={} ({}) pass", icmp_type, name);
                }
                ProtocolResult::Fail { reason, .. } => {
                    eprintln!("icmp-full: type={} ({}) fail: {}", icmp_type, name, reason);
                }
                ProtocolResult::Error { reason } => {
                    eprintln!("icmp-full: type={} ({}) error: {}", icmp_type, name, reason);
                }
            }
            result_count += 1;
        }

        ProtocolResult::Pass {
            sent_bytes: result_count,
            received_bytes: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_icmp_capability_returns_bool() {
        let _result = has_icmp_capability();
    }

    #[test]
    fn create_raw_socket_fails_as_nonroot() {
        if has_icmp_capability() {
            return;
        }
        let result = create_raw_socket();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Permission denied") || err.contains("Operation not permitted"));
    }
}
