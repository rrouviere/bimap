use crate::packet::ip::internet_checksum;

pub const ICMP_HEADER_LEN: usize = 8;
pub const ICMP_TYPE_ECHO_REPLY: u8 = 0;
pub const ICMP_TYPE_ECHO_REQUEST: u8 = 8;
pub const ICMP_TYPE_DEST_UNREACHABLE: u8 = 3;
pub const ICMP_TYPE_SOURCE_QUENCH: u8 = 4;
pub const ICMP_TYPE_REDIRECT: u8 = 5;
pub const ICMP_TYPE_TIME_EXCEEDED: u8 = 11;
pub const ICMP_TYPE_PARAM_PROBLEM: u8 = 12;
pub const ICMP_TYPE_TIMESTAMP: u8 = 13;
pub const ICMP_TYPE_TIMESTAMP_REPLY: u8 = 14;
pub const ICMP_TYPE_INFO_REQUEST: u8 = 15;
pub const ICMP_TYPE_INFO_REPLY: u8 = 16;
pub const ICMP_TYPE_ADDRESS_MASK: u8 = 17;
pub const ICMP_TYPE_ADDRESS_MASK_REPLY: u8 = 18;

#[derive(Debug, Clone)]
pub struct IcmpHeader {
    pub icmp_type: u8,
    pub code: u8,
    pub checksum: u16,
    pub identifier: u16,
    pub sequence: u16,
}

pub const ALL_ICMP_TYPES: &[(u8, &str)] = &[
    (0, "echo-reply"),
    (3, "dest-unreachable"),
    (4, "source-quench"),
    (5, "redirect"),
    (8, "echo-request"),
    (9, "router-advertisement"),
    (10, "router-solicitation"),
    (11, "time-exceeded"),
    (12, "param-problem"),
    (13, "timestamp"),
    (14, "timestamp-reply"),
    (15, "info-request"),
    (16, "info-reply"),
    (17, "address-mask"),
    (18, "address-mask-reply"),
    (30, "traceroute"),
    (31, "datagram-conversion"),
    (33, "ipv6-where-are-you"),
    (34, "ipv6-i-am-here"),
    (35, "mobile-registration"),
    (37, "domain-name-request"),
    (38, "domain-name-reply"),
    (40, "photuris"),
    (42, "extended-echo"),
];

impl IcmpHeader {
    pub fn from_echo_request(id: u16, seq: u16) -> Self {
        let mut header = IcmpHeader {
            icmp_type: ICMP_TYPE_ECHO_REQUEST,
            code: 0,
            checksum: 0,
            identifier: id,
            sequence: seq,
        };
        header.checksum = header.compute_checksum(&[]);
        header
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0u8; ICMP_HEADER_LEN];
        buf[0] = self.icmp_type;
        buf[1] = self.code;
        buf[2] = (self.checksum >> 8) as u8;
        buf[3] = self.checksum as u8;
        buf[4] = (self.identifier >> 8) as u8;
        buf[5] = self.identifier as u8;
        buf[6] = (self.sequence >> 8) as u8;
        buf[7] = self.sequence as u8;
        buf
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < ICMP_HEADER_LEN {
            return None;
        }
        Some(IcmpHeader {
            icmp_type: buf[0],
            code: buf[1],
            checksum: u16::from_be_bytes([buf[2], buf[3]]),
            identifier: u16::from_be_bytes([buf[4], buf[5]]),
            sequence: u16::from_be_bytes([buf[6], buf[7]]),
        })
    }

    pub fn compute_checksum(&self, payload: &[u8]) -> u16 {
        let mut header = self.clone();
        header.checksum = 0;
        let mut encoded = header.encode();
        encoded.extend_from_slice(payload);
        internet_checksum(&encoded)
    }

    pub fn type_name(icmp_type: u8) -> &'static str {
        for &(t, name) in ALL_ICMP_TYPES {
            if t == icmp_type {
                return name;
            }
        }
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icmp_echo_roundtrip() {
        let header = IcmpHeader::from_echo_request(1234, 1);
        let encoded = header.encode();
        assert_eq!(encoded.len(), ICMP_HEADER_LEN);
        let decoded = IcmpHeader::decode(&encoded).unwrap();
        assert_eq!(decoded.icmp_type, ICMP_TYPE_ECHO_REQUEST);
        assert_eq!(decoded.identifier, 1234);
        assert_eq!(decoded.sequence, 1);
    }

    #[test]
    fn checksum_is_nonzero() {
        let header = IcmpHeader::from_echo_request(0, 0);
        assert_ne!(header.checksum, 0);
    }

    #[test]
    fn decode_short_buffer() {
        assert!(IcmpHeader::decode(&[0; 4]).is_none());
    }

    #[test]
    fn all_types_have_names() {
        for &(t, name) in ALL_ICMP_TYPES {
            assert!(!name.is_empty(), "type {t} has no name");
        }
    }

    #[test]
    fn type_name_known_types() {
        assert_eq!(IcmpHeader::type_name(8), "echo-request");
        assert_eq!(IcmpHeader::type_name(0), "echo-reply");
        assert_eq!(IcmpHeader::type_name(13), "timestamp");
    }

    #[test]
    fn type_name_unknown() {
        assert_eq!(IcmpHeader::type_name(99), "unknown");
    }
}
