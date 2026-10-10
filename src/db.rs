use std::str::FromStr;
use std::time::Duration;

use anyhow::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub async fn connect(url: &str) -> Result<SqlitePool> {
    if let Some(path) = url.strip_prefix("sqlite://")
        && let Some(dir) = std::path::Path::new(path.split('?').next().unwrap_or(path)).parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }

    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Deletes request logs older than `days` once an hour.
pub fn spawn_retention(pool: SqlitePool, days: i64) {
    if days <= 0 {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let cutoff = format!("-{days} days");
            let result = sqlx::query(
                "DELETE FROM requests WHERE created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)",
            )
            .bind(&cutoff)
            .execute(&pool)
            .await;
            if let Err(e) = result {
                tracing::warn!(error = %e, "request log retention failed");
            }
        }
    });
}
