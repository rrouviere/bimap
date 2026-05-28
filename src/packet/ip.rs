use std::net::Ipv4Addr;

pub const IPV4_HEADER_LEN: usize = 20;
pub const IP_PROTO_ICMP: u8 = 1;
pub const IP_PROTO_TCP: u8 = 6;
pub const IP_PROTO_UDP: u8 = 17;

#[derive(Debug, Clone)]
pub struct Ipv4Header {
    pub version: u8,
    pub ihl: u8,
    pub dscp: u8,
    pub ecn: u8,
    pub total_length: u16,
    pub identification: u16,
    pub flags: u8,
    pub fragment_offset: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub source: Ipv4Addr,
    pub destination: Ipv4Addr,
}

impl Ipv4Header {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0u8; IPV4_HEADER_LEN];
        buf[0] = (self.version << 4) | (self.ihl & 0x0F);
        buf[1] = (self.dscp << 2) | (self.ecn & 0x03);
        buf[2] = (self.total_length >> 8) as u8;
        buf[3] = self.total_length as u8;
        buf[4] = (self.identification >> 8) as u8;
        buf[5] = self.identification as u8;
        buf[6] = (self.flags << 5) | ((self.fragment_offset >> 8) as u8 & 0x1F);
        buf[7] = self.fragment_offset as u8;
        buf[8] = self.ttl;
        buf[9] = self.protocol;
        buf[12..16].copy_from_slice(&self.source.octets());
        buf[16..20].copy_from_slice(&self.destination.octets());
        let csum = internet_checksum(&buf[..IPV4_HEADER_LEN]);
        buf[10] = (csum >> 8) as u8;
        buf[11] = csum as u8;
        buf
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < IPV4_HEADER_LEN {
            return None;
        }
        Some(Ipv4Header {
            version: buf[0] >> 4,
            ihl: buf[0] & 0x0F,
            dscp: buf[1] >> 2,
            ecn: buf[1] & 0x03,
            total_length: u16::from_be_bytes([buf[2], buf[3]]),
            identification: u16::from_be_bytes([buf[4], buf[5]]),
            flags: buf[6] >> 5,
            fragment_offset: u16::from_be_bytes([buf[6] & 0x1F, buf[7]]),
            ttl: buf[8],
            protocol: buf[9],
            checksum: u16::from_be_bytes([buf[10], buf[11]]),
            source: Ipv4Addr::from(<[u8; 4]>::try_from(&buf[12..16]).ok()?),
            destination: Ipv4Addr::from(<[u8; 4]>::try_from(&buf[16..20]).ok()?),
        })
    }
}

pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_header_roundtrip() {
        let header = Ipv4Header {
            version: 4,
            ihl: 5,
            dscp: 0,
            ecn: 0,
            total_length: 40,
            identification: 12345,
            flags: 2,
            fragment_offset: 0,
            ttl: 64,
            protocol: IP_PROTO_TCP,
            checksum: 0,
            source: Ipv4Addr::new(10, 0, 0, 1),
            destination: Ipv4Addr::new(10, 0, 0, 2),
        };
        let encoded = header.encode();
        assert_eq!(encoded.len(), IPV4_HEADER_LEN);
        let decoded = Ipv4Header::decode(&encoded).unwrap();
        assert_eq!(decoded.source, header.source);
        assert_eq!(decoded.destination, header.destination);
        assert_eq!(decoded.protocol, IP_PROTO_TCP);
        assert_eq!(decoded.ttl, 64);
    }

    #[test]
    fn checksum_zero_payload() {
        assert_eq!(internet_checksum(&[0, 0, 0, 0]), 0xFFFF);
    }

    #[test]
    fn decode_short_buffer() {
        assert!(Ipv4Header::decode(&[0; 10]).is_none());
    }

    #[test]
    fn checksum_known_vector() {
        let data = [
            0x45, 0x00, 0x00, 0x3c, 0x1c, 0x46, 0x40, 0x00, 0x40, 0x06, 0x00, 0x00, 0xac, 0x10,
            0x0a, 0x63, 0xac, 0x10, 0x0a, 0x0c,
        ];
        let csum = internet_checksum(&data);
        assert_eq!(csum, 0xb1e6);
    }
}
