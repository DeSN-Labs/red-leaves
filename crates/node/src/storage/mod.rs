//! SQLite persistence for NAS authentication and social content metadata.
#[cfg(feature = "nas")]
use sqlx::{
    ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

#[cfg(feature = "nas")]
pub async fn open(url: &str) -> Result<SqlitePool, Box<dyn std::error::Error + Send + Sync>> {
    let options = url
        .parse::<SqliteConnectOptions>()?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5))
        .disable_statement_logging();
    #[cfg(unix)]
    let filename = options.get_filename().to_path_buf();
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(options)
        .await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for suffix in ["", "-wal", "-shm"] {
            let mut path = filename.as_os_str().to_os_string();
            path.push(suffix);
            match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    let mut tx = pool.begin().await?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *tx)
        .await?;
    match version {
        0 => {
            sqlx::raw_sql(include_str!("schema.sql"))
                .execute(&mut *tx)
                .await?;
        }
        6..=8 => {}
        _ => return Err("Unsupported database schema version. Use an empty database or a matching node version.".into()),
    }
    if version == 0 || version == 6 {
        sqlx::raw_sql(include_str!("likes.sql"))
            .execute(&mut *tx)
            .await?;
    }
    if version != 8 {
        sqlx::raw_sql(include_str!("access.sql"))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(pool)
}

#[cfg(all(test, feature = "nas"))]
mod tests;
