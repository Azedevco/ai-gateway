use chrono::{Duration, DurationRound, Utc};
use serde::Deserialize;
use sqlx::{FromRow, SqlitePool};

/// Dashboard time window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Range {
    #[default]
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl Range {
    pub const ALL: [Range; 3] = [Range::Day, Range::Week, Range::Month];

    pub fn key(self) -> &'static str {
        match self {
            Range::Day => "24h",
            Range::Week => "7d",
            Range::Month => "30d",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Range::Day => "Last 24 hours",
            Range::Week => "Last 7 days",
            Range::Month => "Last 30 days",
        }
    }

    /// SQLite modifier for `strftime(..., 'now', ?)`.
    fn modifier(self) -> &'static str {
        match self {
            Range::Day => "-24 hours",
            Range::Week => "-7 days",
            Range::Month => "-30 days",
        }
    }

    fn hourly(self) -> bool {
        self == Range::Day
    }
}

#[derive(Debug, Default, FromRow)]
pub struct Totals {
    pub requests: i64,
    pub ok: i64,
    pub failed: i64,
    /// Requests answered after at least one failed attempt.
    pub fallbacks: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub avg_latency_ms: f64,
}

/// Per-model upstream results (from attempts, so failed fallback tries count too).
#[derive(Debug, FromRow)]
pub struct ModelStat {
    pub model_name: String,
    pub provider_name: String,
    pub attempts: i64,
    pub ok: i64,
    pub failed: i64,
    /// Requests this model answered.
    pub answered: i64,
    pub total_tokens: i64,
    pub avg_latency_ms: f64,
}

#[derive(Debug, FromRow)]
pub struct KeyStat {
    pub name: String,
    pub requests: i64,
    pub failed: i64,
    pub total_tokens: i64,
}

#[derive(Debug, FromRow)]
pub struct RequestedStat {
    pub requested_model: String,
    pub requests: i64,
    pub failed: i64,
}

/// One time bucket of the activity chart.
#[derive(Debug, Clone)]
pub struct Bucket {
    /// Bucket start, UTC, `YYYY-MM-DDTHH` (hourly) or `YYYY-MM-DD` (daily).
    pub key: String,
    pub requests: i64,
    pub failed: i64,
    pub tokens: i64,
}

pub async fn totals(db: &SqlitePool, range: Range) -> sqlx::Result<Totals> {
    sqlx::query_as(
        "SELECT COUNT(*) AS requests,
                COALESCE(SUM(status = 'ok'), 0) AS ok,
                COALESCE(SUM(status = 'error'), 0) AS failed,
                COALESCE(SUM(status = 'ok' AND attempts > 1), 0) AS fallbacks,
                COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens,
                COALESCE(SUM(completion_tokens), 0) AS completion_tokens,
                COALESCE(SUM(total_tokens), 0) AS total_tokens,
                COALESCE(AVG(latency_ms), 0.0) AS avg_latency_ms
         FROM requests WHERE created_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)",
    )
    .bind(range.modifier())
    .fetch_one(db)
    .await
}

pub async fn by_model(db: &SqlitePool, range: Range) -> sqlx::Result<Vec<ModelStat>> {
    sqlx::query_as(
        "SELECT a.model_name, a.provider_name,
                COUNT(*) AS attempts,
                SUM(a.ok) AS ok,
                SUM(NOT a.ok) AS failed,
                COUNT(DISTINCT CASE WHEN a.ok THEN a.request_id END) AS answered,
                COALESCE((SELECT SUM(r.total_tokens) FROM requests r
                           WHERE r.model_name = a.model_name AND r.provider_name = a.provider_name
                             AND r.status = 'ok'
                             AND r.created_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)), 0) AS total_tokens,
                COALESCE(AVG(CASE WHEN a.ok THEN a.latency_ms END), 0.0) AS avg_latency_ms
         FROM attempts a
         WHERE a.created_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)
         GROUP BY a.model_name, a.provider_name
         ORDER BY answered DESC, attempts DESC",
    )
    .bind(range.modifier())
    .fetch_all(db)
    .await
}

pub async fn by_key(db: &SqlitePool, range: Range) -> sqlx::Result<Vec<KeyStat>> {
    sqlx::query_as(
        "SELECT COALESCE(k.name, '(deleted key)') AS name, COUNT(*) AS requests,
                SUM(r.status = 'error') AS failed, SUM(r.total_tokens) AS total_tokens
         FROM requests r LEFT JOIN api_keys k ON k.id = r.api_key_id
         WHERE r.created_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)
         GROUP BY r.api_key_id ORDER BY requests DESC",
    )
    .bind(range.modifier())
    .fetch_all(db)
    .await
}

pub async fn by_requested(db: &SqlitePool, range: Range) -> sqlx::Result<Vec<RequestedStat>> {
    sqlx::query_as(
        "SELECT requested_model, COUNT(*) AS requests, SUM(status = 'error') AS failed
         FROM requests WHERE created_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)
         GROUP BY requested_model ORDER BY requests DESC",
    )
    .bind(range.modifier())
    .fetch_all(db)
    .await
}

/// Activity per hour (24h) or per day (7d/30d), with empty buckets filled in.
pub async fn timeline(db: &SqlitePool, range: Range) -> sqlx::Result<Vec<Bucket>> {
    let width = if range.hourly() { 13 } else { 10 };
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT substr(created_at, 1, ?) AS bucket, COUNT(*), SUM(status = 'error'), SUM(total_tokens)
         FROM requests WHERE created_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)
         GROUP BY bucket",
    )
    .bind(width)
    .bind(range.modifier())
    .fetch_all(db)
    .await?;

    let (step, count, fmt) = match range {
        Range::Day => (Duration::hours(1), 24, "%Y-%m-%dT%H"),
        Range::Week => (Duration::days(1), 7, "%Y-%m-%d"),
        Range::Month => (Duration::days(1), 30, "%Y-%m-%d"),
    };
    let end = Utc::now()
        .duration_trunc(step)
        .unwrap_or_else(|_| Utc::now());
    Ok((0..count)
        .rev()
        .map(|i| {
            let key = (end - step * i).format(fmt).to_string();
            let (requests, failed, tokens) = rows
                .iter()
                .find(|r| r.0 == key)
                .map(|r| (r.1, r.2, r.3))
                .unwrap_or_default();
            Bucket {
                key,
                requests,
                failed,
                tokens,
            }
        })
        .collect())
}
