// Minimal stderr logger — dependency-free backend for the `log` facade.
// Without one installed, every log::warn!/error! in the engine (including
// prefab/chapter parse errors) is silently dropped.
//
// Level comes from the SMIRK_LOG env var (error|warn|info|debug|trace),
// defaulting to info.

struct StderrLogger;

static LOGGER: StderrLogger = StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool { true }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{:5}] {}: {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

/// Install the logger. Safe to call more than once — only the first wins.
pub fn init() {
    let level = match std::env::var("SMIRK_LOG").as_deref() {
        Ok("error") => log::LevelFilter::Error,
        Ok("warn")  => log::LevelFilter::Warn,
        Ok("debug") => log::LevelFilter::Debug,
        Ok("trace") => log::LevelFilter::Trace,
        _           => log::LevelFilter::Info,
    };
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}
