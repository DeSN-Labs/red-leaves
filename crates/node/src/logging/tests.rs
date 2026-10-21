use super::*;
use std::{collections::HashMap, fs};

fn config(values: &[(&str, &str)]) -> Result<Config, LogError> {
    let values: HashMap<_, _> = values.iter().copied().collect();
    Config::read(|name| values.get(name).map(|value| value.to_string()))
}
fn directory() -> PathBuf {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("red-leaves-logs-{}-{id}", std::process::id()))
}
#[test]
fn configuration_has_safe_defaults_and_rejects_invalid_values() {
    let defaults = config(&[]).unwrap();
    assert_eq!(defaults.level, LevelFilter::INFO);
    assert!(defaults.console && defaults.file);
    assert_eq!(defaults.max_files, 7);
    for values in [
        [("RED_LEAVES_LOG_LEVEL", "unexpected")],
        [("RED_LEAVES_LOG_FORMAT", "xml")],
        [("RED_LEAVES_LOG_CONSOLE", "yes")],
        [("RED_LEAVES_LOG_MAX_FILES", "0")],
        [("RED_LEAVES_LOG_MAX_FILES", "366")],
        [("RED_LEAVES_LOG_DIR", "")],
    ] {
        assert!(config(&values).is_err());
    }
    assert!(config(&[("RED_LEAVES_LOG_FILE", "false"), ("RED_LEAVES_LOG_DIR", "")]).is_ok());
}
#[test]
fn json_files_flush_filter_levels_and_suppress_sensitive_library_logs() {
    let path = directory();
    let settings = config(&[
        ("RED_LEAVES_LOG_FORMAT", "json"),
        ("RED_LEAVES_LOG_LEVEL", "debug"),
        ("RED_LEAVES_LOG_CONSOLE", "false"),
        ("RED_LEAVES_LOG_DIR", path.to_str().unwrap()),
    ])
    .unwrap();
    let (subscriber, guard) = build(&settings, "nas").unwrap();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(event_kind = "startup", "Node started");
        tracing::debug!("Debug event");
        tracing::trace!("Suppressed trace event");
        tracing::warn!(target:"sqlx::query","PRIVATE SQL parameters");
        tracing::warn!(target:"reqwest::connect","SECRET RPC credentials");
        tracing::debug!(target:"libp2p_request_response","PRIVATE protocol message");
        tracing::warn!(target:"libp2p_swarm","Peer unavailable");
    });
    assert_eq!(guard.dropped_lines(), 0);
    drop(guard);
    let files = fs::read_dir(&path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1);
    let text = fs::read_to_string(&files[0]).unwrap();
    let lines = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|value| value["timestamp"].is_string()));
    assert!(
        text.contains("Node started")
            && text.contains("Debug event")
            && text.contains("Peer unavailable")
    );
    for secret in ["PRIVATE", "SECRET", "Suppressed trace"] {
        assert!(!text.contains(secret));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn retention_only_removes_matching_logs_and_disabled_files_create_no_directory() {
    let path = directory();
    private_directory(&path).unwrap();
    for name in [
        "red-leaves-nas.2000-01-01.log",
        "red-leaves-nas.2000-01-02.log",
        "red-leaves-relay.2000-01-01.log",
        "notes.txt",
    ] {
        fs::write(path.join(name), "Old log").unwrap();
    }
    let settings = config(&[
        ("RED_LEAVES_LOG_CONSOLE", "false"),
        ("RED_LEAVES_LOG_DIR", path.to_str().unwrap()),
        ("RED_LEAVES_LOG_MAX_FILES", "2"),
    ])
    .unwrap();
    let (subscriber, guard) = build(&settings, "nas").unwrap();
    tracing::subscriber::with_default(subscriber, || tracing::info!("Current log"));
    drop(guard);
    assert!(!path.join("red-leaves-nas.2000-01-01.log").exists());
    assert!(path.join("notes.txt").exists());
    assert!(path.join("red-leaves-relay.2000-01-01.log").exists());
    assert!(
        fs::read_dir(&path)
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("red-leaves-nas."))
            .count()
            <= 2
    );
    fs::remove_dir_all(&path).unwrap();
    let settings = config(&[
        ("RED_LEAVES_LOG_LEVEL", "off"),
        ("RED_LEAVES_LOG_DIR", path.to_str().unwrap()),
    ])
    .unwrap();
    let (_, guard) = build(&settings, "nas").unwrap();
    drop(guard);
    assert!(!path.exists());
}
#[cfg(unix)]
#[test]
fn shared_or_symlink_log_directories_are_rejected_without_chmod() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let path = directory();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(private_directory(&path).is_err());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o755
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let link = path.with_extension("link");
    symlink(&path, &link).unwrap();
    assert!(private_directory(&link).is_err());
    fs::remove_file(link).unwrap();
    fs::remove_dir(path).unwrap();
}
