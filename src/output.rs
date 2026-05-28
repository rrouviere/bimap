use owo_colors::OwoColorize;
use std::fmt;
use std::io::Write;
use tracing_subscriber::fmt::writer::MakeWriter;

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
