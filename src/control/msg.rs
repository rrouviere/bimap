use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello {
        version: u32,
        fingerprint: String,
    },
    Configure {
        tests: Vec<String>,
        port_ranges: Vec<PortRangeSpec>,
        bidir: bool,
    },
    Ack {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Test {
        id: u32,
        protocol: String,
        transport: String,
        port: u16,
        direction: String,
    },
    Report {
        id: u32,
        sent: Option<TransferReport>,
        received: Option<TransferReport>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Done,
    Bye {
        summary: TestSummary,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortRangeSpec {
    pub transport: String,
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferReport {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSummary {
    pub passed: u32,
    pub failed: u32,
    pub errors: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_roundtrip() {
        let msg = Message::Hello {
            version: 1,
            fingerprint: "sha256:deadbeef".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        match back {
            Message::Hello {
                version,
                fingerprint,
            } => {
                assert_eq!(version, 1);
                assert_eq!(fingerprint, "sha256:deadbeef");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn configure_roundtrip() {
        let msg = Message::Configure {
            tests: vec!["open".into(), "dns".into()],
            port_ranges: vec![PortRangeSpec {
                transport: "tcp".into(),
                start: 1,
                end: 1024,
            }],
            bidir: false,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        match back {
            Message::Configure {
                tests,
                port_ranges,
                bidir,
            } => {
                assert_eq!(tests.len(), 2);
                assert_eq!(port_ranges[0].transport, "tcp");
                assert!(!bidir);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn report_roundtrip() {
        let msg = Message::Report {
            id: 42,
            sent: Some(TransferReport {
                bytes: 1024,
                sha256: "abcdef".into(),
            }),
            received: Some(TransferReport {
                bytes: 1024,
                sha256: "abcdef".into(),
            }),
            error: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        match back {
            Message::Report {
                id, sent, received, ..
            } => {
                assert_eq!(id, 42);
                assert!(sent.unwrap().bytes == 1024);
                assert!(received.unwrap().bytes == 1024);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn report_with_error() {
        let msg = Message::Report {
            id: 1,
            sent: None,
            received: None,
            error: Some("bind: address in use".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        match back {
            Message::Report {
                id,
                sent,
                received,
                error,
            } => {
                assert_eq!(id, 1);
                assert!(sent.is_none());
                assert!(received.is_none());
                assert!(error.unwrap().contains("in use"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn malformed_json_is_error() {
        let result: Result<Message, _> = serde_json::from_str("not json");
        assert!(result.is_err());
    }

    #[test]
    fn unknown_fields_tolerated() {
        let json = r#"{"type":"hello","version":1,"fingerprint":"abc","extra_field":123}"#;
        let back: Message = serde_json::from_str(json).unwrap();
        assert!(matches!(back, Message::Hello { .. }));
    }

    #[test]
    fn bye_roundtrip() {
        let msg = Message::Bye {
            summary: TestSummary {
                passed: 5,
                failed: 2,
                errors: 1,
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"bye\""));
        let back: Message = serde_json::from_str(&json).unwrap();
        match back {
            Message::Bye { summary } => {
                assert_eq!(summary.passed, 5);
                assert_eq!(summary.failed, 2);
                assert_eq!(summary.errors, 1);
            }
            _ => panic!("wrong variant"),
        }
    }
}
