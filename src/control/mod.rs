pub mod msg;
pub mod tls;

use msg::Message;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tracing::{debug, trace};

pub struct ControlChannel {
    reader: BufReader<Box<dyn AsyncRead + Unpin + Send>>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    verbose: u8,
    frame: Vec<u8>,
    framing_failed: bool,
    peer_address: Result<std::net::SocketAddr, String>,
    local_address: Result<std::net::SocketAddr, String>,
}

impl ControlChannel {
    pub fn peer_addr(&self) -> Result<std::net::SocketAddr, String> {
        self.peer_address.clone()
    }

    pub fn local_addr(&self) -> Result<std::net::SocketAddr, String> {
        self.local_address.clone()
    }

    pub async fn send(&mut self, msg: &Message) -> Result<(), String> {
        let mut json = serde_json::to_vec(msg).map_err(|e| format!("serialize: {e}"))?;
        if self.verbose >= 1 {
            debug!("ctrl send: {} bytes", json.len());
        }
        if self.verbose >= 3 {
            trace!(">>> {}", String::from_utf8_lossy(&json));
        }
        json.push(b'\n');
        self.writer
            .write_all(&json)
            .await
            .map_err(|e| format!("write: {e}"))?;
        self.writer.flush().await.map_err(|e| format!("flush: {e}"))
    }

    pub async fn recv(&mut self) -> Result<Message, String> {
        const MAX_FRAME_BYTES: usize = 1024 * 1024;
        if self.framing_failed {
            return Err("control framing failed".into());
        }
        loop {
            let buffered = self
                .reader
                .fill_buf()
                .await
                .map_err(|error| format!("read: {error}"))?;
            if buffered.is_empty() {
                return Err(if self.frame.is_empty() {
                    "connection closed".into()
                } else {
                    "connection closed during control frame".into()
                });
            }
            let newline = buffered.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(buffered.len(), |index| index + 1);
            if self.frame.len() + count > MAX_FRAME_BYTES {
                self.frame.clear();
                self.framing_failed = true;
                return Err("control frame too large (maximum 1048576 bytes)".into());
            }
            self.frame.extend_from_slice(&buffered[..count]);
            self.reader.consume(count);
            if newline.is_some() {
                let frame = std::mem::take(&mut self.frame);
                if self.verbose >= 3 {
                    trace!("<<< {}", String::from_utf8_lossy(&frame));
                }
                return serde_json::from_slice(&frame)
                    .map_err(|error| format!("deserialize: {error}"));
            }
        }
    }
}

fn new_channel(
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    verbose: u8,
    peer_address: Result<std::net::SocketAddr, String>,
    local_address: Result<std::net::SocketAddr, String>,
) -> ControlChannel {
    ControlChannel {
        reader: BufReader::new(reader),
        writer,
        verbose,
        frame: Vec::new(),
        framing_failed: false,
        peer_address,
        local_address,
    }
}

pub fn channel_from_tls_stream(
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    verbose: u8,
) -> ControlChannel {
    let peer_address = stream
        .get_ref()
        .0
        .peer_addr()
        .map_err(|error| error.to_string());
    let local_address = stream
        .get_ref()
        .0
        .local_addr()
        .map_err(|error| error.to_string());
    let (rx, tx) = tokio::io::split(stream);
    new_channel(
        Box::new(rx),
        Box::new(tx),
        verbose,
        peer_address,
        local_address,
    )
}

pub fn channel_from_client_tls(
    stream: tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
    verbose: u8,
) -> ControlChannel {
    let peer_address = stream
        .get_ref()
        .0
        .peer_addr()
        .map_err(|error| error.to_string());
    let local_address = stream
        .get_ref()
        .0
        .local_addr()
        .map_err(|error| error.to_string());
    let (rx, tx) = tokio::io::split(stream);
    new_channel(
        Box::new(rx),
        Box::new(tx),
        verbose,
        peer_address,
        local_address,
    )
}
