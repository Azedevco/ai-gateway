use sqlx::{FromRow, SqlitePool};

use super::kind::Kind;

/// A model together with its provider, as shown in lists and combo pickers.
#[derive(Debug, Clone, FromRow)]
pub struct ModelRow {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub kind: Kind,
    pub provider_id: i64,
    pub provider_name: String,
    pub provider_enabled: bool,
}

impl ModelRow {
    pub fn usable(&self) -> bool {
        self.enabled && self.provider_enabled
    }
}

/// `SELECT` for [`ModelRow`] followed by the given SQL tail, as one string literal.
macro_rules! select_models {
    ($tail:literal) => {
        concat!(
            "SELECT m.id, m.name, m.enabled, m.kind, p.id AS provider_id, p.name AS provider_name,
                    p.enabled AS provider_enabled
             FROM models m JOIN providers p ON p.id = m.provider_id ",
            $tail
        )
    };
}

pub async fn list_by_kind(db: &SqlitePool, kind: Kind) -> sqlx::Result<Vec<ModelRow>> {
    sqlx::query_as(select_models!("WHERE m.kind = ? ORDER BY p.name, m.name"))
        .bind(kind)
        .fetch_all(db)
        .await
}

pub async fn list_for_provider(db: &SqlitePool, provider_id: i64) -> sqlx::Result<Vec<ModelRow>> {
    sqlx::query_as(select_models!("WHERE p.id = ? ORDER BY m.name"))
        .bind(provider_id)
        .fetch_all(db)
        .await
}

/// Adds the given model ids to a provider as `kind`, skipping ones it already has. Returns how
/// many were new.
pub async fn add_many(
    db: &SqlitePool,
    provider_id: i64,
    names: &[String],
    kind: Kind,
) -> sqlx::Result<u64> {
    let mut added = 0;
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        added +=
            sqlx::query("INSERT OR IGNORE INTO models (provider_id, name, kind) VALUES (?, ?, ?)")
                .bind(provider_id)
                .bind(name)
                .bind(kind)
                .execute(db)
                .await?
                .rows_affected();
    }
    Ok(added)
}

pub async fn provider_of(db: &SqlitePool, id: i64) -> sqlx::Result<i64> {
    let row: (i64,) = sqlx::query_as("SELECT provider_id FROM models WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await?;
    Ok(row.0)
}

/// Changes a model's kind and drops it from combos of the other kind.
pub async fn set_kind(db: &SqlitePool, id: i64, kind: Kind) -> sqlx::Result<i64> {
    let mut tx = db.begin().await?;
    let provider_id: (i64,) =
        sqlx::query_as("UPDATE models SET kind = ? WHERE id = ? RETURNING provider_id")
            .bind(kind)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "DELETE FROM combo_entries WHERE model_id = ?
           AND combo_id IN (SELECT id FROM combos WHERE kind != ?)",
    )
    .bind(id)
    .bind(kind)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(provider_id.0)
}

pub async fn toggle(db: &SqlitePool, id: i64) -> sqlx::Result<i64> {
    let provider_id: (i64,) = sqlx::query_as(
        "UPDATE models SET enabled = NOT enabled WHERE id = ? RETURNING provider_id",
    )
    .bind(id)
    .fetch_one(db)
    .await?;
    Ok(provider_id.0)
}

pub async fn delete(db: &SqlitePool, id: i64) -> sqlx::Result<i64> {
    let provider_id: (i64,) =
        sqlx::query_as("DELETE FROM models WHERE id = ? RETURNING provider_id")
            .bind(id)
            .fetch_one(db)
            .await?;
    Ok(provider_id.0)
}
