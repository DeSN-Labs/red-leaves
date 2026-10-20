//! SQLite persistence for NAS authentication.
#[cfg(feature = "nas")]
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

#[cfg(feature = "nas")]
pub async fn open(url: &str) -> Result<SqlitePool, Box<dyn std::error::Error + Send + Sync>> {
    let options = url
        .parse::<SqliteConnectOptions>()?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(options)
        .await?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await?;
    if version > 1 {
        return Err("Unsupported database schema version".into());
    }
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(include_str!("../../migrations/0001_auth.sql"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(pool)
}
