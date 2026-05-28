use hickory_proto::op::{Message, Query};
use hickory_proto::rr::rdata::A;
use hickory_proto::rr::{Name, RData, Record, RecordType};
use hickory_proto::serialize::binary::{BinDecodable, BinEncodable, BinEncoder};

pub fn build_dns_query(domain: &str, id: u16) -> Result<Vec<u8>, String> {
    let name = Name::from_utf8(domain).map_err(|e| format!("invalid name: {e}"))?;
    let mut msg = Message::new();
    msg.set_id(id);
    msg.set_recursion_desired(true);
    msg.add_query(Query::query(name, RecordType::A));
    let mut buf = Vec::new();
    let mut encoder = BinEncoder::new(&mut buf);
    msg.emit(&mut encoder)
        .map_err(|e| format!("encode query: {e}"))?;
    Ok(buf)
}

pub fn parse_dns_message(data: &[u8]) -> Result<Message, String> {
    Message::from_bytes(data).map_err(|e| format!("decode: {e}"))
}

pub fn build_dns_response(query: &Message) -> Result<Vec<u8>, String> {
    let mut response = Message::new();
    response.set_id(query.id());
    response.set_message_type(hickory_proto::op::MessageType::Response);
    response.set_recursion_desired(true);
    response.set_response_code(hickory_proto::op::ResponseCode::NoError);

    for q in query.queries() {
        response.add_query(q.clone());
    }

    let record = Record::from_rdata(
        query
            .queries()
            .first()
            .map(|q| q.name().clone())
            .unwrap_or_default(),
        300,
        RData::A(A::from(std::net::Ipv4Addr::new(10, 0, 0, 1))),
    );
    response.add_answer(record);

    let mut buf = Vec::new();
    let mut encoder = BinEncoder::new(&mut buf);
    response
        .emit(&mut encoder)
        .map_err(|e| format!("encode response: {e}"))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_parse_query() {
        let buf = build_dns_query("bimap.test", 12345).unwrap();
        let msg = parse_dns_message(&buf).unwrap();
        assert_eq!(msg.id(), 12345);
        assert_eq!(msg.queries().len(), 1);
        assert_eq!(msg.queries()[0].name().to_string(), "bimap.test.");
    }

    #[test]
    fn build_response_roundtrip() {
        let qbuf = build_dns_query("bimap.test", 42).unwrap();
        let query = parse_dns_message(&qbuf).unwrap();
        let rbuf = build_dns_response(&query).unwrap();
        let response = parse_dns_message(&rbuf).unwrap();
        assert!(response.message_type() == hickory_proto::op::MessageType::Response);
        assert_eq!(response.answers().len(), 1);
        assert_eq!(response.id(), 42);
    }

    #[test]
    fn malformed_data_is_error() {
        assert!(parse_dns_message(b"").is_err());
        assert!(parse_dns_message(b"garbage").is_err());
    }
}
