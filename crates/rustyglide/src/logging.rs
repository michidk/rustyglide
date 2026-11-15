//! Opt-in logging for hosts without a logger, such as games loading the
//! drop-in libraries: `RUSTYGLIDE_LOG` names a file (or `stderr`) that gets
//! rustyglide's and wgpu's messages, `RUSTYGLIDE_LOG_LEVEL` sets the level
//! (`error`, `warn`, `info` by default, `debug`, `trace`).

use std::io::Write;
use std::sync::Mutex;

struct FileLog {
    out: Mutex<Box<dyn Write + Send>>,
    level: log::LevelFilter,
}

impl log::Log for FileLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
            let _ = writeln!(out, "{:<5} {}: {}", record.level(), record.target(), record.args());
            let _ = out.flush();
        }
    }

    fn flush(&self) {}
}

/// Installs the logger `RUSTYGLIDE_LOG` asks for. Does nothing when it is
/// unset or another logger is installed already.
pub fn init_from_env() {
    let Some(target) = std::env::var_os("RUSTYGLIDE_LOG").filter(|t| !t.is_empty()) else { return };
    let out: Box<dyn Write + Send> = if target == "stderr" {
        Box::new(std::io::stderr())
    } else {
        match std::fs::OpenOptions::new().create(true).append(true).open(&target) {
            Ok(file) => Box::new(file),
            Err(_) => return,
        }
    };
    let level =
        std::env::var("RUSTYGLIDE_LOG_LEVEL").ok().and_then(|l| l.parse().ok()).unwrap_or(log::LevelFilter::Info);
    if log::set_boxed_logger(Box::new(FileLog { out: Mutex::new(out), level })).is_ok() {
        log::set_max_level(level);
        // A panic inside a C export aborts the game; record why first.
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
            previous(info);
        }));
        log::info!("rustyglide {} logging at {level}", env!("CARGO_PKG_VERSION"));
    }
}
