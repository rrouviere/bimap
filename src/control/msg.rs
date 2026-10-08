use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello {
        version: u32,
        fingerprint: String,
    },
    Configure {
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        timeout_ms: u64,
        client_version: u32,
        parallel: usize,
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
    BatchEnd,
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
pub struct TransferReport {
    pub bytes: u64,
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
    fn control_json_serialization_contains_only_used_fields() {
        for expected in [
            serde_json::json!({"type":"configure","target":"127.0.0.1:0","timeout_ms":500,"client_version":PROTOCOL_VERSION,"parallel":1}),
            serde_json::json!({"type":"report","id":1,"sent":{"bytes":1},"received":{"bytes":1}}),
        ] {
            let message: Message = serde_json::from_value(expected.clone()).expect("message");
            assert_eq!(serde_json::to_value(message).expect("serialize"), expected);
        }
    }

    #[test]
    fn control_json_configuration_requires_explicit_settings() {
        let complete = serde_json::json!({"type":"configure","target":"127.0.0.1:0","timeout_ms":500,"client_version":PROTOCOL_VERSION,"parallel":1});
        for field in ["timeout_ms", "client_version", "parallel"] {
            let mut incomplete = complete.clone();
            incomplete.as_object_mut().expect("object").remove(field);
            assert!(
                serde_json::from_value::<Message>(incomplete).is_err(),
                "missing {field} must be rejected"
            );
        }
    }

    #[test]
    fn hello_roundtrip() {
        let msg = Message::Hello {
            version: PROTOCOL_VERSION,
            fingerprint: "sha256:deadbeef".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        match back {
            Message::Hello {
                version,
                fingerprint,
            } => {
                assert_eq!(version, PROTOCOL_VERSION);
                assert_eq!(fingerprint, "sha256:deadbeef");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn batch_end_roundtrip() {
        let json = serde_json::to_string(&Message::BatchEnd).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Message::BatchEnd));
    }

    #[test]
    fn configure_roundtrip() {
        let message = Message::Configure {
            target: Some("127.0.0.1:0".into()),
            timeout_ms: 500,
            client_version: PROTOCOL_VERSION,
            parallel: 100,
        };
        let json = serde_json::to_string(&message).expect("serialize");
        match serde_json::from_str::<Message>(&json).expect("deserialize") {
            Message::Configure {
                target,
                timeout_ms,
                client_version,
                parallel,
            } => {
                assert_eq!(target.as_deref(), Some("127.0.0.1:0"));
                assert_eq!(timeout_ms, 500);
                assert_eq!(client_version, PROTOCOL_VERSION);
                assert_eq!(parallel, 100);
            }
            _ => panic!("expected Configure"),
        }
    }

    #[test]
    fn report_roundtrip() {
        let msg = Message::Report {
            id: 42,
            sent: Some(TransferReport { bytes: 1024 }),
            received: Some(TransferReport { bytes: 1024 }),
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
