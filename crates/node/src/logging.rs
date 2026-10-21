//! Application logging. Sensitive transports and SQL statements never reach these sinks.
use std::{error::Error, io, path::PathBuf};
use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::{ErrorCounter, WorkerGuard};
use tracing_subscriber::{
    Layer, Registry, filter::Targets, fmt::writer::BoxMakeWriter, layer::SubscriberExt,
};

type LogError = Box<dyn Error + Send + Sync>;
type OutputLayer = Box<dyn Layer<Registry> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Text,
    Json,
}
pub struct Config {
    level: LevelFilter,
    format: Format,
    console: bool,
    file: bool,
    directory: PathBuf,
    max_files: usize,
}
impl Config {
    pub fn from_env() -> Result<Self, LogError> {
        Self::read(|name| std::env::var(name).ok())
    }
    fn read(get: impl Fn(&str) -> Option<String>) -> Result<Self, LogError> {
        fn flag(value: Option<String>, default: bool) -> Result<bool, LogError> {
            match value.as_deref() {
                None => Ok(default),
                Some("true") => Ok(true),
                Some("false") => Ok(false),
                _ => Err("Log output flags must be true or false".into()),
            }
        }
        let level = get("RED_LEAVES_LOG_LEVEL")
            .unwrap_or_else(|| "info".into())
            .parse()
            .map_err(|_| "RED_LEAVES_LOG_LEVEL must be off, error, warn, info, debug or trace")?;
        let format = match get("RED_LEAVES_LOG_FORMAT").as_deref() {
            None | Some("text") => Format::Text,
            Some("json") => Format::Json,
            _ => return Err("RED_LEAVES_LOG_FORMAT must be text or json".into()),
        };
        let max_files = get("RED_LEAVES_LOG_MAX_FILES")
            .unwrap_or_else(|| "7".into())
            .parse()
            .map_err(|_| "RED_LEAVES_LOG_MAX_FILES must be an integer from 1 to 365")?;
        if !(1..=365).contains(&max_files) {
            return Err("RED_LEAVES_LOG_MAX_FILES must be an integer from 1 to 365".into());
        }
        let directory =
            PathBuf::from(get("RED_LEAVES_LOG_DIR").unwrap_or_else(|| "runtime/logs".into()));
        let file = flag(get("RED_LEAVES_LOG_FILE"), true)?;
        if file && directory.as_os_str().is_empty() {
            return Err("RED_LEAVES_LOG_DIR must not be empty when file logging is enabled".into());
        }
        Ok(Self {
            level,
            format,
            console: flag(get("RED_LEAVES_LOG_CONSOLE"), true)?,
            file,
            directory,
            max_files,
        })
    }
    fn filter(&self) -> Targets {
        // Library debug output can include HTTP credentials or decoded protocol data.
        // Keep library diagnostics at WARN or lower and disable sensitive SQL/HTTP targets.
        Targets::new()
            .with_default(self.level.min(LevelFilter::WARN))
            .with_target("red_leaves", self.level)
            .with_target("sqlx", LevelFilter::OFF)
            .with_target("reqwest", LevelFilter::OFF)
            .with_target("hyper", LevelFilter::OFF)
            .with_target("hyper_util", LevelFilter::OFF)
            .with_target("h2", LevelFilter::OFF)
    }
}
pub struct Guard {
    _worker: Option<WorkerGuard>,
    dropped: Option<ErrorCounter>,
}
impl Guard {
    fn dropped_lines(&self) -> usize {
        self.dropped.as_ref().map_or(0, ErrorCounter::dropped_lines)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let dropped = self.dropped_lines();
        if dropped > 0 {
            tracing::warn!(
                dropped_lines = dropped,
                "File log queue overflowed; some log lines were dropped"
            );
        }
        // The worker field drops after this method and flushes pending file output.
    }
}
fn output(writer: BoxMakeWriter, format: Format) -> OutputLayer {
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(writer);
    match format {
        Format::Text => layer.boxed(),
        Format::Json => layer.json().boxed(),
    }
}
fn private_directory(path: &std::path::Path) -> Result<(), LogError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(
        |_| "Unable to create log directory; check RED_LEAVES_LOG_DIR and its permissions",
    )?;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "Unable to inspect log directory")?;
    if !metadata.is_dir() {
        return Err("RED_LEAVES_LOG_DIR must be a directory, not a symlink or file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                "Log directory must be private; set Unix directory permissions to 700".into(),
            );
        }
    }
    Ok(())
}
fn build(
    config: &Config,
    role: &str,
) -> Result<(Box<dyn tracing::Subscriber + Send + Sync>, Guard), LogError> {
    let mut layers: Vec<OutputLayer> = Vec::new();
    if config.console {
        layers.push(output(BoxMakeWriter::new(io::stderr), config.format));
    }
    let mut guard = Guard {
        _worker: None,
        dropped: None,
    };
    if config.file && config.level != LevelFilter::OFF {
        private_directory(&config.directory)?;
        let appender = tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(format!("red-leaves-{role}"))
            .filename_suffix("log")
            .max_log_files(config.max_files)
            .build(&config.directory)
            .map_err(
                |_| "Unable to open log file; check the log directory and available disk space",
            )?;
        let (writer, worker) = tracing_appender::non_blocking::NonBlockingBuilder::default()
            .buffered_lines_limit(2048)
            .lossy(true)
            .thread_name("red-leaves-log-writer")
            .finish(appender);
        guard.dropped = Some(writer.error_counter());
        guard._worker = Some(worker);
        layers.push(output(BoxMakeWriter::new(writer), config.format));
    }
    Ok((
        Box::new(
            tracing_subscriber::registry()
                .with(layers)
                .with(config.filter()),
        ),
        guard,
    ))
}
pub fn init(role: &str) -> Result<Guard, LogError> {
    let config = Config::from_env()?;
    let (subscriber, guard) = build(&config, role)?;
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_| "Logging subscriber is already initialized")?;
    // Panic payloads may contain request data. Record only the source location.
    std::panic::set_hook(Box::new(|panic| {
        if let Some(location) = panic.location() {
            tracing::error!(
                file = location.file(),
                line = location.line(),
                "A task panicked"
            );
        } else {
            tracing::error!("A task panicked");
        }
    }));
    tracing::info!(node_role=role,format=?config.format,file_logging=config.file,console_logging=config.console,"Logging initialized");
    Ok(guard)
}

/// A stage is safe to log; an arbitrary error's Display/Debug may contain credentials.
#[cfg(any(feature = "nas", feature = "relay"))]
pub struct Failure {
    pub stage: &'static str,
    source: LogError,
}
#[cfg(any(feature = "nas", feature = "relay"))]
impl Failure {
    pub fn at<E: Into<LogError>>(stage: &'static str) -> impl FnOnce(E) -> Self {
        move |source| Self {
            stage,
            source: source.into(),
        }
    }
    pub fn report(&self) {
        let kind = self
            .source
            .downcast_ref::<io::Error>()
            .map(|error| format!("{:?}", error.kind()))
            .unwrap_or_else(|| "operation_failed".into());
        tracing::error!(
            stage = self.stage,
            error_kind = kind,
            "Node operation failed"
        );
    }
}

#[cfg(test)]
mod tests;
