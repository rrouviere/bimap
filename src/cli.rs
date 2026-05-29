use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "bimap", version, about = "Bidirectional network mapper")]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start bimap in server mode (responds to client test requests)
    #[command(name = "server")]
    Server {
        /// Bind address (default: [::]:443 — dual-stack on Linux, IPv6-only on macOS/Windows)
        #[arg(long, default_value = "[::]:443")]
        bind: String,

        /// Verbose debug output. -vvv dumps raw messages
        #[arg(short = 'v', long, action = clap::ArgAction::Count, default_value_t = 0)]
        verbose: u8,
    },

    /// Start bimap in client mode (connects to server, runs tests)
    #[command(
        name = "client",
        after_help = "AVAILABLE TESTS:\n  open        L4  tcp,udp  — TCP/1-byte or UDP echo roundtrip\n  1kb         L4  tcp,udp  — 1024-byte data roundtrip + SHA-256 check\n  icmp-ping   L3  icmp     — ICMP echo request/reply (root required)\n  icmp-full   L3  icmp     — full ICMP type scan (root required)\n  tls         L7  tcp      — TLS handshake + 1024-byte SHA-256 check\n  dns         L7  tcp,udp  — DNS A query for example.com\n\nRun without --test to list available tests."
    )]
    Client {
        /// Control server address (ip:port, IPv6: [::1]:443). Replaces --server + --port
        #[arg(long, value_name = "IP:PORT")]
        control_server: Option<String>,

        /// Target address for tests (IP or hostname, default: control server IP)
        #[arg(long, value_name = "ADDRESS")]
        target: Option<String>,

        /// Server address (IP or hostname, required unless --control-server is given)
        #[arg(long, value_name = "ADDRESS", required = false)]
        server: Option<String>,

        /// Control port (default: 443)
        #[arg(long, default_value_t = 443)]
        port: u16,

        /// Test to run (repeatable). Omit to list available tests
        #[arg(long, value_name = "NAME", num_args = 0..)]
        test: Vec<String>,

        /// Port range to probe (repeatable)
        /// Format: <transport>/<start>-<end>, e.g. tcp/1-1024, udp/8000-8999, icmp/any
        #[arg(long, value_name = "SPEC")]
        port_range: Vec<String>,

        /// Run all tests in reverse direction
        #[arg(long)]
        bidir: bool,

        /// Per-test timeout in milliseconds (default: 500)
        #[arg(long, default_value_t = 500)]
        timeout: u64,

        /// Auto-accept by default. If set, must match server SHA-256 fingerprint or we refuse
        #[arg(long, value_name = "HASH")]
        fingerprint: Option<String>,

        /// Output one JSON object per result line
        #[arg(long)]
        json: bool,

        /// Export all results as a single JSON object at the end
        #[arg(long)]
        json_export: bool,

        /// Number of tests to run in parallel (default: 100, 1 = sequential)
        #[arg(long, default_value_t = 100)]
        parallel: usize,

        /// Verbose debug output. -v = debug, -vv = trace
        #[arg(short = 'v', long, action = clap::ArgAction::Count, default_value_t = 0)]
        verbose: u8,

        /// Only show failures and final summary (suppress PASS lines)
        #[arg(short = 'q', long)]
        quiet: bool,
    },
}

pub fn parse() -> Result<Command, String> {
    let args = Args::parse();
    match args.command {
        Some(cmd) => Ok(cmd),
        None => Err("no command specified, use --help for usage".into()),
    }
}
