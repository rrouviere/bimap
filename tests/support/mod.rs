use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub fn available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("reserve port")
        .local_addr()
        .expect("reserved address")
        .port()
}

pub struct Process(pub Child);

impl Process {
    pub fn output(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.0.try_wait().expect("process status") {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "process did not finish within 10s"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        self.0
            .stdout
            .take()
            .expect("stdout")
            .read_to_end(&mut stdout)
            .expect("read stdout");
        self.0
            .stderr
            .take()
            .expect("stderr")
            .read_to_end(&mut stderr)
            .expect("read stderr");
        Output {
            status,
            stdout,
            stderr,
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        match self.0.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(error) => eprintln!("process status: {error}"),
        }
        if let Err(error) = self.0.kill() {
            eprintln!("stop process: {error}");
        }
        if let Err(error) = self.0.wait() {
            eprintln!("reap process: {error}");
        }
    }
}

pub struct Server {
    pub process: Process,
    host: String,
    port: u16,
    pub fingerprint: String,
    messages: Receiver<String>,
}

impl Server {
    pub fn start() -> Self {
        Self::start_on("127.0.0.1")
    }

    pub fn start_on(host: &str) -> Self {
        let port = available_port();
        let address = format!("{host}:{port}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_bimap"))
            .args(["server", "--bind", &address, "-v"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start server");
        let stderr = child.stderr.take().expect("server stderr");
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let line = line.expect("server log line");
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            process: Process(child),
            host: host.to_string(),
            port,
            fingerprint: String::new(),
            messages,
        };
        server.wait_for("listening on");
        assert!(
            server.fingerprint.starts_with("SHA256:"),
            "missing server fingerprint"
        );
        server
    }

    pub fn wait_for(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let line = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("server did not reach expected state");
            if let Some((_, fingerprint)) = line.split_once("fingerprint: ") {
                self.fingerprint = fingerprint.trim().to_string();
            }
            if line.contains(expected) {
                return;
            }
        }
    }

    pub fn client(&self, arguments: &[&str]) -> Process {
        Process(
            Command::new(env!("CARGO_BIN_EXE_bimap"))
                .args([
                    "client",
                    "--server",
                    &self.host,
                    "--port",
                    &self.port.to_string(),
                ])
                .args(arguments)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start client"),
        )
    }
}
