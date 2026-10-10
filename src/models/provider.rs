use serde::Deserialize;
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, FromRow)]
pub struct Provider {
    pub id: i64,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub enabled: bool,
    pub timeout_secs: i64,
}

impl Provider {
    /// The key with all but the last 4 characters hidden, for display.
    pub fn masked_key(&self) -> String {
        match self.api_key.len() {
            0 => "—".into(),
            n if n <= 8 => "••••".into(),
            n => format!("••••{}", &self.api_key[n - 4..]),
        }
    }
}

/// Provider list row with its model count.
#[derive(Debug, FromRow)]
pub struct ProviderRow {
    pub id: i64,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    pub model_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct ProviderForm {
    pub name: String,
    pub base_url: String,
    /// Empty on edit means "keep the current key".
    #[serde(default)]
    pub api_key: String,
    pub timeout_secs: Option<i64>,
    /// Checkbox: present when ticked.
    pub enabled: Option<String>,
}

impl ProviderForm {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() {
            return Err("Name is required.");
        }
        if !(self.base_url.starts_with("http://") || self.base_url.starts_with("https://")) {
            return Err("Base URL must start with http:// or https://.");
        }
        Ok(())
    }

    fn base_url(&self) -> String {
        // Accept a pasted ".../chat/completions" URL and keep the API root.
        let url = self.base_url.trim().trim_end_matches('/');
        url.strip_suffix("/chat/completions")
            .unwrap_or(url)
            .to_string()
    }

    fn timeout(&self) -> i64 {
        self.timeout_secs.unwrap_or(120).clamp(5, 1800)
    }
}

pub async fn list(db: &SqlitePool) -> sqlx::Result<Vec<ProviderRow>> {
    sqlx::query_as(
        "SELECT p.id, p.name, p.base_url, p.enabled, COUNT(m.id) AS model_count
         FROM providers p LEFT JOIN models m ON m.provider_id = p.id
         GROUP BY p.id ORDER BY p.name",
    )
    .fetch_all(db)
    .await
}

pub async fn get(db: &SqlitePool, id: i64) -> sqlx::Result<Provider> {
    sqlx::query_as(
        "SELECT id, name, base_url, api_key, enabled, timeout_secs FROM providers WHERE id = ?",
    )
    .bind(id)
    .fetch_one(db)
    .await
}

pub async fn create(db: &SqlitePool, form: &ProviderForm) -> sqlx::Result<i64> {
    let id = sqlx::query(
        "INSERT INTO providers (name, base_url, api_key, timeout_secs, enabled) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(form.name.trim())
    .bind(form.base_url())
    .bind(form.api_key.trim())
    .bind(form.timeout())
    .bind(form.enabled.is_some())
    .execute(db)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub async fn update(db: &SqlitePool, id: i64, form: &ProviderForm) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE providers SET name = ?, base_url = ?, timeout_secs = ?, enabled = ?,
                api_key = CASE WHEN ? = '' THEN api_key ELSE ? END
         WHERE id = ?",
    )
    .bind(form.name.trim())
    .bind(form.base_url())
    .bind(form.timeout())
    .bind(form.enabled.is_some())
    .bind(form.api_key.trim())
    .bind(form.api_key.trim())
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM providers WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// True when a UNIQUE constraint rejected the write (duplicate name).
pub fn is_duplicate(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}
