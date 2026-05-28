use crate::packet::ip::internet_checksum;

pub const UDP_HEADER_LEN: usize = 8;

#[derive(Debug, Clone)]
pub struct UdpHeader {
    pub source_port: u16,
    pub destination_port: u16,
    pub length: u16,
    pub checksum: u16,
}

impl UdpHeader {
    pub fn new(source_port: u16, destination_port: u16, payload_len: u16) -> Self {
        UdpHeader {
            source_port,
            destination_port,
            length: UDP_HEADER_LEN as u16 + payload_len,
            checksum: 0,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0u8; UDP_HEADER_LEN];
        buf[0] = (self.source_port >> 8) as u8;
        buf[1] = self.source_port as u8;
        buf[2] = (self.destination_port >> 8) as u8;
        buf[3] = self.destination_port as u8;
        buf[4] = (self.length >> 8) as u8;
        buf[5] = self.length as u8;
        buf[6] = (self.checksum >> 8) as u8;
        buf[7] = self.checksum as u8;
        buf
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < UDP_HEADER_LEN {
            return None;
        }
        Some(UdpHeader {
            source_port: u16::from_be_bytes([buf[0], buf[1]]),
            destination_port: u16::from_be_bytes([buf[2], buf[3]]),
            length: u16::from_be_bytes([buf[4], buf[5]]),
            checksum: u16::from_be_bytes([buf[6], buf[7]]),
        })
    }

    pub fn compute_checksum(
        &self,
        pseudo_header: &crate::packet::tcp::Ipv4PseudoHeader,
        payload: &[u8],
    ) -> u16 {
        let mut data = Vec::new();
        data.extend_from_slice(&pseudo_header.encode());
        data.extend_from_slice(&self.encode());
        data.extend_from_slice(payload);
        internet_checksum(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::tcp::Ipv4PseudoHeader;

    #[test]
    fn udp_header_roundtrip() {
        let header = UdpHeader::new(12345, 53, 8);
        let encoded = header.encode();
        assert_eq!(encoded.len(), UDP_HEADER_LEN);
        let decoded = UdpHeader::decode(&encoded).unwrap();
        assert_eq!(decoded.source_port, 12345);
        assert_eq!(decoded.destination_port, 53);
        assert_eq!(decoded.length, 16); // 8 header + 8 payload
    }

    #[test]
    fn udp_checksum_nonzero_with_payload() {
        let header = UdpHeader::new(12345, 53, 3);
        let pseudo = Ipv4PseudoHeader {
            source: [10, 0, 0, 1],
            destination: [10, 0, 0, 2],
            protocol: 17,
            tcp_length: 11,
        };
        let csum = header.compute_checksum(&pseudo, b"abc");
        assert_ne!(csum, 0);
    }

    #[test]
    fn decode_short_buffer() {
        assert!(UdpHeader::decode(&[0; 4]).is_none());
    }
}
