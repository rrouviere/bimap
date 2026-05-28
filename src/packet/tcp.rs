use crate::packet::ip::internet_checksum;

pub const TCP_HEADER_LEN: usize = 20;

#[derive(Debug, Clone)]
pub struct TcpHeader {
    pub source_port: u16,
    pub destination_port: u16,
    pub sequence_number: u32,
    pub acknowledgment_number: u32,
    pub data_offset: u8,
    pub flags: u8,
    pub window_size: u16,
    pub checksum: u16,
    pub urgent_pointer: u16,
}

pub const TCP_FLAG_FIN: u8 = 0x01;
pub const TCP_FLAG_SYN: u8 = 0x02;
pub const TCP_FLAG_RST: u8 = 0x04;
pub const TCP_FLAG_PSH: u8 = 0x08;
pub const TCP_FLAG_ACK: u8 = 0x10;
pub const TCP_FLAG_URG: u8 = 0x20;

impl TcpHeader {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0u8; TCP_HEADER_LEN];
        buf[0] = (self.source_port >> 8) as u8;
        buf[1] = self.source_port as u8;
        buf[2] = (self.destination_port >> 8) as u8;
        buf[3] = self.destination_port as u8;
        buf[4..8].copy_from_slice(&self.sequence_number.to_be_bytes());
        buf[8..12].copy_from_slice(&self.acknowledgment_number.to_be_bytes());
        buf[12] = (self.data_offset << 4) & 0xF0;
        buf[13] = self.flags;
        buf[14] = (self.window_size >> 8) as u8;
        buf[15] = self.window_size as u8;
        buf[18] = (self.urgent_pointer >> 8) as u8;
        buf[19] = self.urgent_pointer as u8;
        buf
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < TCP_HEADER_LEN {
            return None;
        }
        Some(TcpHeader {
            source_port: u16::from_be_bytes([buf[0], buf[1]]),
            destination_port: u16::from_be_bytes([buf[2], buf[3]]),
            sequence_number: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
            acknowledgment_number: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
            data_offset: buf[12] >> 4,
            flags: buf[13],
            window_size: u16::from_be_bytes([buf[14], buf[15]]),
            checksum: u16::from_be_bytes([buf[16], buf[17]]),
            urgent_pointer: u16::from_be_bytes([buf[18], buf[19]]),
        })
    }

    pub fn compute_checksum(&self, pseudo_header: &Ipv4PseudoHeader, payload: &[u8]) -> u16 {
        let mut data = Vec::new();
        data.extend_from_slice(&pseudo_header.encode());
        let tcp_header = self.encode();
        data.extend_from_slice(&tcp_header);
        data.extend_from_slice(payload);
        let _tcp_length = (TCP_HEADER_LEN + payload.len()) as u16;
        internet_checksum(&data)
    }
}

#[derive(Debug, Clone)]
pub struct Ipv4PseudoHeader {
    pub source: [u8; 4],
    pub destination: [u8; 4],
    pub protocol: u8,
    pub tcp_length: u16,
}

impl Ipv4PseudoHeader {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(12);
        buf.extend_from_slice(&self.source);
        buf.extend_from_slice(&self.destination);
        buf.push(0);
        buf.push(self.protocol);
        buf.push((self.tcp_length >> 8) as u8);
        buf.push(self.tcp_length as u8);
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_header_roundtrip() {
        let header = TcpHeader {
            source_port: 12345,
            destination_port: 80,
            sequence_number: 1000,
            acknowledgment_number: 0,
            data_offset: 5,
            flags: TCP_FLAG_SYN,
            window_size: 65535,
            checksum: 0,
            urgent_pointer: 0,
        };
        let encoded = header.encode();
        assert_eq!(encoded.len(), TCP_HEADER_LEN);
        let decoded = TcpHeader::decode(&encoded).unwrap();
        assert_eq!(decoded.source_port, 12345);
        assert_eq!(decoded.destination_port, 80);
        assert_eq!(decoded.flags, TCP_FLAG_SYN);
    }

    #[test]
    fn tcp_checksum_nonzero() {
        let header = TcpHeader {
            source_port: 12345,
            destination_port: 80,
            sequence_number: 1000,
            acknowledgment_number: 0,
            data_offset: 5,
            flags: TCP_FLAG_SYN,
            window_size: 65535,
            checksum: 0,
            urgent_pointer: 0,
        };
        let pseudo = Ipv4PseudoHeader {
            source: [10, 0, 0, 1],
            destination: [10, 0, 0, 2],
            protocol: 6,
            tcp_length: 20,
        };
        let csum = header.compute_checksum(&pseudo, &[]);
        assert_ne!(csum, 0);
    }
}
