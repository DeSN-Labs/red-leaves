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
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await?;
    if version > 4 {
        return Err("Unsupported database schema version".into());
    }
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(include_str!("../../migrations/0001_auth.sql"))
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(include_str!("../../migrations/0002_posts.sql"))
        .execute(&mut *tx)
        .await?;
    if version < 3 {
        sqlx::raw_sql(include_str!("../../migrations/0003_media.sql"))
            .execute(&mut *tx)
            .await?;
    }
    if version < 4 {
        sqlx::raw_sql(include_str!("../../migrations/0004_delivery.sql"))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("PRAGMA user_version=4")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(pool)
}
