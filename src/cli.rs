use clap::{Parser, Subcommand};
use std::net::IpAddr;

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
        /// Bind address (default: 0.0.0.0:443)
        #[arg(long, default_value = "0.0.0.0:443")]
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
        /// Server address (required)
        #[arg(long, value_name = "IP")]
        server: IpAddr,

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

        /// Per-test timeout in milliseconds (default: 5000)
        #[arg(long, default_value_t = 5000)]
        timeout: u64,

        /// Trust this SHA-256 certificate fingerprint, skip interactive prompt
        #[arg(long, value_name = "HASH")]
        fingerprint: Option<String>,

        /// Output one JSON object per result line
        #[arg(long)]
        json: bool,

        /// Export all results as a single JSON object at the end
        #[arg(long)]
        json_export: bool,

        /// Suppress passing tests from output
        #[arg(short = 'q', long)]
        quiet: bool,

        /// Verbose debug output including timing and raw hex
        #[arg(short = 'v', long)]
        verbose: bool,
    },
}

pub fn parse() -> Result<Command, String> {
    let args = Args::parse();
    match args.command {
        Some(cmd) => Ok(cmd),
        None => Err("no command specified, use --help for usage".into()),
    }
}
