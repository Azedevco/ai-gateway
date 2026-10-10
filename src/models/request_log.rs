use sqlx::{FromRow, SqlitePool};

/// One upstream call made while serving a request.
#[derive(Debug, Clone)]
pub struct Attempt {
    pub model_id: i64,
    pub model_name: String,
    pub provider_name: String,
    pub ok: bool,
    pub http_status: Option<u16>,
    pub latency_ms: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

impl Usage {
    /// Reads an OpenAI `usage` object; missing fields count as zero.
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        let u = v.get("usage")?.as_object()?;
        let n = |k: &str| u.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
        let (prompt, completion) = (n("prompt_tokens"), n("completion_tokens"));
        let total = match n("total_tokens") {
            0 => prompt + completion,
            t => t,
        };
        Some(Self {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
        })
    }
}

/// Everything recorded about a finished request.
#[derive(Debug, Clone)]
pub struct RequestRecord {
    pub api_key_id: Option<i64>,
    pub requested_model: String,
    pub stream: bool,
    pub http_status: u16,
    pub latency_ms: i64,
    pub usage: Usage,
    pub attempts: Vec<Attempt>,
    /// Error shown to the client when no upstream succeeded.
    pub error: Option<String>,
}

impl RequestRecord {
    fn answered_by(&self) -> Option<&Attempt> {
        self.attempts.iter().rev().find(|a| a.ok)
    }
}

pub async fn insert(db: &SqlitePool, r: &RequestRecord) -> sqlx::Result<()> {
    let answered = r.answered_by();
    let last = answered.or(r.attempts.last());
    let mut tx = db.begin().await?;
    let request_id = sqlx::query(
        "INSERT INTO requests (api_key_id, requested_model, model_id, model_name, provider_name, status,
                               http_status, attempts, stream, latency_ms, prompt_tokens, completion_tokens,
                               total_tokens, error)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(r.api_key_id)
    .bind(&r.requested_model)
    .bind(answered.map(|a| a.model_id))
    .bind(last.map(|a| &a.model_name))
    .bind(last.map(|a| &a.provider_name))
    .bind(if answered.is_some() { "ok" } else { "error" })
    .bind(r.http_status)
    .bind(r.attempts.len() as i64)
    .bind(r.stream)
    .bind(r.latency_ms)
    .bind(r.usage.prompt_tokens)
    .bind(r.usage.completion_tokens)
    .bind(r.usage.total_tokens)
    .bind(&r.error)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();

    for a in &r.attempts {
        sqlx::query(
            "INSERT INTO attempts (request_id, model_id, model_name, provider_name, ok, http_status, latency_ms, error)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(request_id)
        .bind(a.model_id)
        .bind(&a.model_name)
        .bind(&a.provider_name)
        .bind(a.ok)
        .bind(a.http_status)
        .bind(a.latency_ms)
        .bind(&a.error)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Writes the record in the background; logging must never fail or slow a client response.
pub fn record(db: SqlitePool, r: RequestRecord) {
    tokio::spawn(async move {
        if let Err(e) = insert(&db, &r).await {
            tracing::error!(error = %e, "failed to record request");
        }
    });
}

#[derive(Debug, FromRow)]
pub struct RequestRow {
    pub id: i64,
    pub created_at: String,
    pub requested_model: String,
    pub model_name: Option<String>,
    pub provider_name: Option<String>,
    pub key_name: Option<String>,
    pub status: String,
    pub http_status: i64,
    pub attempts: i64,
    pub stream: bool,
    pub latency_ms: i64,
    pub total_tokens: i64,
    pub error: Option<String>,
}

#[derive(Debug, FromRow)]
pub struct AttemptRow {
    pub model_name: String,
    pub provider_name: String,
    pub ok: bool,
    pub http_status: Option<i64>,
    pub latency_ms: i64,
    pub error: Option<String>,
}

pub struct Filter<'a> {
    pub status: Option<&'a str>,
    pub model: Option<&'a str>,
}

pub async fn list(
    db: &SqlitePool,
    f: &Filter<'_>,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<RequestRow>> {
    sqlx::query_as(
        "SELECT r.id, r.created_at, r.requested_model, r.model_name, r.provider_name, k.name AS key_name,
                r.status, r.http_status, r.attempts, r.stream, r.latency_ms, r.total_tokens, r.error
         FROM requests r LEFT JOIN api_keys k ON k.id = r.api_key_id
         WHERE (?1 IS NULL OR r.status = ?1)
           AND (?2 IS NULL OR r.requested_model = ?2 OR r.model_name = ?2)
         ORDER BY r.id DESC LIMIT ?3 OFFSET ?4",
    )
    .bind(f.status)
    .bind(f.model)
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await
}

pub async fn attempts_for(
    db: &SqlitePool,
    request_ids: &[i64],
) -> sqlx::Result<Vec<(i64, AttemptRow)>> {
    if request_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; request_ids.len()].join(",");
    let sql = format!(
        "SELECT request_id, model_name, provider_name, ok, http_status, latency_ms, error
         FROM attempts WHERE request_id IN ({placeholders}) ORDER BY id"
    );
    // Only `?` placeholders are interpolated; values are bound below.
    let mut q = sqlx::query_as::<_, (i64, String, String, bool, Option<i64>, i64, Option<String>)>(
        sqlx::AssertSqlSafe(sql),
    );
    for id in request_ids {
        q = q.bind(id);
    }
    Ok(q.fetch_all(db)
        .await?
        .into_iter()
        .map(
            |(rid, model_name, provider_name, ok, http_status, latency_ms, error)| {
                (
                    rid,
                    AttemptRow {
                        model_name,
                        provider_name,
                        ok,
                        http_status,
                        latency_ms,
                        error,
                    },
                )
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_fills_missing_total() {
        let u = Usage::from_json(&json!({"usage": {"prompt_tokens": 10, "completion_tokens": 5}}))
            .unwrap();
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens, u.total_tokens),
            (10, 5, 15)
        );
        assert!(Usage::from_json(&json!({"usage": null})).is_none());
    }
}
