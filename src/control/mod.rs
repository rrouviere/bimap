pub mod msg;
pub mod tls;

use msg::Message;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

pub struct ControlChannel {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    verbose: u8,
}

impl ControlChannel {
    pub async fn send(&mut self, msg: &Message) -> Result<(), String> {
        let mut json = serde_json::to_vec(msg).map_err(|e| format!("serialize: {e}"))?;
        if self.verbose >= 3 {
            eprintln!("[bimap-server] >>> {}", String::from_utf8_lossy(&json));
        }
        json.push(b'\n');
        self.writer
            .write_all(&json)
            .await
            .map_err(|e| format!("write: {e}"))
    }

    pub async fn recv(&mut self) -> Result<Message, String> {
        let mut line = String::new();
        let mut reader = BufReader::new(&mut self.reader);
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("read: {e}"))?;
        if line.is_empty() {
            return Err("connection closed".into());
        }
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if self.verbose >= 3 {
            eprintln!("[bimap-server] <<< {trimmed}");
        }
        serde_json::from_str(trimmed).map_err(|e| format!("deserialize: {e}"))
    }
}

fn new_channel(
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    verbose: u8,
) -> ControlChannel {
    ControlChannel {
        reader,
        writer,
        verbose,
    }
}

pub fn channel_from_tls_stream(
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    verbose: u8,
) -> ControlChannel {
    let (rx, tx) = tokio::io::split(stream);
    new_channel(Box::new(rx), Box::new(tx), verbose)
}

pub fn channel_from_client_tls(
    stream: tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
) -> ControlChannel {
    let (rx, tx) = tokio::io::split(stream);
    new_channel(Box::new(rx), Box::new(tx), 0)
}
