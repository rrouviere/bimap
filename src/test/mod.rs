use crate::orchestrator::ProtocolResult;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::time::Duration;

pub mod dns;
pub mod icmp;
pub mod port;
pub mod tls_test;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum Layer {
    L3,
    L4,
    L7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum Transport {
    Tcp,
    Udp,
    Icmp,
}

impl Transport {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Tcp => "tcp",
            Transport::Udp => "udp",
            Transport::Icmp => "icmp",
        }
    }

    #[allow(dead_code, clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "tcp" => Some(Transport::Tcp),
            "udp" => Some(Transport::Udp),
            "icmp" => Some(Transport::Icmp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    ClientToServer,
    ServerToClient,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::ClientToServer => "->",
            Direction::ServerToClient => "<-",
        }
    }
}

pub struct TestContext {
    pub direction: Direction,
    pub transport: Transport,
    pub port: u16,
    pub target_addr: SocketAddr,
    pub timeout: Duration,
    pub verbose: bool,
}

#[async_trait]
#[allow(dead_code)]
pub trait TestProtocol: Send + Sync {
    fn name(&self) -> &'static str;
    #[allow(dead_code)]
    fn layer(&self) -> Layer;
    fn transports(&self) -> &[Transport];

    async fn run(&self, ctx: TestContext) -> ProtocolResult;
}

pub struct TestRegistry {
    protocols: Vec<Box<dyn TestProtocol>>,
}

impl TestRegistry {
    pub fn new() -> Self {
        Self {
            protocols: Vec::new(),
        }
    }

    pub fn register(&mut self, protocol: Box<dyn TestProtocol>) {
        self.protocols.push(protocol);
    }

    pub fn find(&self, name: &str) -> Option<&dyn TestProtocol> {
        self.protocols
            .iter()
            .find(|p| p.name() == name)
            .map(|p| p.as_ref())
    }

    #[allow(dead_code)]
    #[allow(dead_code)]
    pub fn names(&self) -> Vec<&'static str> {
        self.protocols.iter().map(|p| p.name()).collect()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.protocols.is_empty()
    }
}

impl Default for TestRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn build_registry() -> TestRegistry {
    let mut registry = TestRegistry::new();
    registry.register(Box::new(port::OpenTest));
    registry.register(Box::new(port::KbTest));
    registry.register(Box::new(icmp::IcmpPingTest));
    registry.register(Box::new(icmp::IcmpFullTest));
    registry.register(Box::new(tls_test::TlsTest));
    registry.register(Box::new(dns::DnsTest));
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProtocol;

    #[async_trait]
    impl TestProtocol for MockProtocol {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn layer(&self) -> Layer {
            Layer::L4
        }
        fn transports(&self) -> &[Transport] {
            &[Transport::Tcp]
        }
        async fn run(&self, _ctx: TestContext) -> ProtocolResult {
            ProtocolResult::Pass {
                sent_bytes: 1,
                received_bytes: 1,
            }
        }
    }

    #[test]
    fn registry_find() {
        let mut registry = TestRegistry::new();
        registry.register(Box::new(MockProtocol));
        assert_eq!(registry.names(), vec!["mock"]);
        assert!(registry.find("mock").is_some());
        assert!(registry.find("nonexistent").is_none());
    }

    #[test]
    fn transport_from_str() {
        assert_eq!(Transport::from_str("tcp"), Some(Transport::Tcp));
        assert_eq!(Transport::from_str("udp"), Some(Transport::Udp));
        assert_eq!(Transport::from_str("icmp"), Some(Transport::Icmp));
        assert_eq!(Transport::from_str("sctp"), None);
    }

    #[test]
    fn direction_as_str() {
        assert_eq!(Direction::ClientToServer.as_str(), "->");
        assert_eq!(Direction::ServerToClient.as_str(), "<-");
    }
}
