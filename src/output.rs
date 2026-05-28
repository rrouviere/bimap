use owo_colors::OwoColorize;
use std::fmt;
use std::io::Write;
use tracing_subscriber::fmt::writer::MakeWriter;

pub fn is_interactive() -> bool {
    #[cfg(unix)]
    unsafe {
        libc::isatty(libc::STDOUT_FILENO) == 1
    }
    #[cfg(not(unix))]
    true
}

/// Print a live-updating fail line that overwrites itself with `\r`.
/// Call `finish_fail_line` when done to finalize.
pub fn print_fail_live(msg: impl fmt::Display) {
    let s = if wants_color() {
        format!("{} {}", "FAIL".red().bold(), msg)
    } else {
        format!("FAIL {}", msg)
    };
    print!("\r{s}");
    std::io::stdout().flush().ok();
}

/// Finalize a live fail line (print newline).
pub fn finish_fail_line() {
    println!();
}

/// Format a sorted port list as compact ranges: "0-21,23-79,81-1023"
pub fn format_port_ranges(ports: &[u16]) -> String {
    if ports.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    let mut i = 0;
    while i < ports.len() {
        let start = ports[i];
        let mut end = start;
        while i + 1 < ports.len() && ports[i + 1] == end + 1 {
            end = ports[i + 1];
            i += 1;
        }
        if start == end {
            parts.push(start.to_string());
        } else {
            parts.push(format!("{start}-{end}"));
        }
        i += 1;
    }
    parts.join(",")
}

pub fn init_tracing(verbose: u8) {
    use tracing_subscriber::fmt;
    use tracing_subscriber::EnvFilter;

    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::builder()
        .with_default_directive(level.parse().unwrap())
        .from_env_lossy()
        .add_directive(format!("bimap={level}").parse().unwrap());
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(FlushWriter)
        .init();
}

struct FlushWriter;

impl<'a> MakeWriter<'a> for FlushWriter {
    type Writer = FlushStderr;

    fn make_writer(&'a self) -> Self::Writer {
        FlushStderr(std::io::stderr())
    }
}

struct FlushStderr(std::io::Stderr);

impl Write for FlushStderr {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.0.write(buf)?;
        self.0.flush()?;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

fn wants_color() -> bool {
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    #[cfg(unix)]
    unsafe {
        libc::isatty(libc::STDOUT_FILENO) == 1
    }
    #[cfg(not(unix))]
    true
}

pub fn print_pass(msg: impl fmt::Display) {
    if wants_color() {
        println!("{} {}", "PASS".green().bold(), msg);
    } else {
        println!("PASS {}", msg);
    }
}

pub fn print_fail(msg: impl fmt::Display) {
    if wants_color() {
        println!("{} {}", "FAIL".red().bold(), msg);
    } else {
        println!("FAIL {}", msg);
    }
}

pub fn print_err(msg: impl fmt::Display) {
    if wants_color() {
        println!("{} {}", "ERR ".yellow().bold(), msg);
    } else {
        println!("ERR  {}", msg);
    }
}

pub fn print_summary(msg: impl fmt::Display) {
    if wants_color() {
        println!("{}", msg.cyan());
    } else {
        println!("{}", msg);
    }
}
