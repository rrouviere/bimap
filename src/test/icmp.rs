use crate::orchestrator::ProtocolResult;
use crate::packet::icmp::{IcmpHeader, ALL_ICMP_TYPES, ICMP_TYPE_ECHO_REPLY};
use crate::packet::ip::Ipv4Header;
use crate::test::{Direction, Layer, TestContext, TestProtocol, Transport};
use async_trait::async_trait;
use std::net::Ipv4Addr;
use std::time::Duration;

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
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

async fn send_icmp_echo(
    dest: Ipv4Addr,
    id: u16,
    seq: u16,
    icmp_type: u8,
    timeout: Duration,
) -> ProtocolResult {
    if !is_root() {
        return ProtocolResult::Error {
            reason: "permission: ICMP requires root".into(),
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
    let icmp = IcmpHeader {
        icmp_type,
        code: 0,
        checksum: 0,
        identifier: id,
        sequence: seq,
    };
    let csum = icmp.compute_checksum(payload);
    let mut packet = icmp.encode();
    packet[2] = (csum >> 8) as u8;
    packet[3] = (csum & 0xFF) as u8;
    packet.extend_from_slice(payload);

    let sa = to_sockaddr_in(dest);
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

    unsafe {
        libc::close(fd);
    }

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

    let n = received as usize;
    let iph = match Ipv4Header::decode(&recv_buf[..n]) {
        Some(h) => h,
        None => {
            return ProtocolResult::Fail {
                reason: "bad IP header".into(),
                sent_bytes: sent as u64,
                received_bytes: n as u64,
            };
        }
    };

    let icmp_offset = (iph.ihl * 4) as usize;
    if icmp_offset + 8 > n {
        return ProtocolResult::Fail {
            reason: "short ICMP reply".into(),
            sent_bytes: sent as u64,
            received_bytes: n as u64,
        };
    }

    let reply = match IcmpHeader::decode(&recv_buf[icmp_offset..]) {
        Some(r) => r,
        None => {
            return ProtocolResult::Fail {
                reason: "bad ICMP header".into(),
                sent_bytes: sent as u64,
                received_bytes: n as u64,
            };
        }
    };

    if reply.icmp_type == ICMP_TYPE_ECHO_REPLY {
        ProtocolResult::Pass {
            sent_bytes: sent as u64,
            received_bytes: n as u64,
        }
    } else {
        ProtocolResult::Fail {
            reason: format!(
                "unexpected ICMP type: {}",
                IcmpHeader::type_name(reply.icmp_type)
            ),
            sent_bytes: sent as u64,
            received_bytes: n as u64,
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

        send_icmp_echo(dest, 0x42, 1, 8, ctx.timeout).await
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
        if !is_root() {
            return ProtocolResult::Error {
                reason: "permission: ICMP requires root".into(),
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
            let result =
                send_icmp_echo(dest, icmp_type as u16, idx as u16, icmp_type, ctx.timeout).await;
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
    fn is_root_returns_bool() {
        let _result = is_root();
        // Function must not panic
    }

    #[test]
    fn create_raw_socket_fails_as_nonroot() {
        if is_root() {
            return;
        }
        let result = create_raw_socket();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Permission denied") || err.contains("Operation not permitted"));
    }
}
