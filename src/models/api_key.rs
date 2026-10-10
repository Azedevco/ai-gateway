use rand::RngExt;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};

const PREFIX: &str = "agw_";

#[derive(Debug, FromRow)]
pub struct ApiKeyRow {
    pub id: i64,
    pub name: String,
    pub key_prefix: String,
    pub enabled: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

fn hash(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// Creates a key and returns the plaintext. It is shown once and never stored.
pub async fn create(db: &SqlitePool, name: &str) -> sqlx::Result<String> {
    let bytes: [u8; 24] = rand::rng().random();
    let key = format!("{PREFIX}{}", hex::encode(bytes));
    sqlx::query("INSERT INTO api_keys (name, key_hash, key_prefix) VALUES (?, ?, ?)")
        .bind(name.trim())
        .bind(hash(&key))
        .bind(&key[..PREFIX.len() + 6])
        .execute(db)
        .await?;
    Ok(key)
}

/// Returns the key id if `key` is a known, enabled key.
pub async fn verify(db: &SqlitePool, key: &str) -> sqlx::Result<Option<i64>> {
    let row: Option<(i64,)> = sqlx::query_as(
        "UPDATE api_keys SET last_used_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE key_hash = ? AND enabled RETURNING id",
    )
    .bind(hash(key))
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn list(db: &SqlitePool) -> sqlx::Result<Vec<ApiKeyRow>> {
    sqlx::query_as(
        "SELECT id, name, key_prefix, enabled, created_at, last_used_at FROM api_keys ORDER BY created_at DESC",
    )
    .fetch_all(db)
    .await
}

pub async fn toggle(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE api_keys SET enabled = NOT enabled WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM api_keys WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_hex() {
        assert_eq!(hash("agw_x"), hash("agw_x"));
        assert_eq!(hash("agw_x").len(), 64);
        assert_ne!(hash("agw_x"), hash("agw_y"));
    }
}
