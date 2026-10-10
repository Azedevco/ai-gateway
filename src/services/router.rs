//! Turns the `model` a client asked for into an ordered list of upstreams to try.

use sqlx::{FromRow, SqlitePool};

use crate::models::kind::Kind;

#[derive(Debug, Clone, FromRow)]
pub struct Target {
    pub model_id: i64,
    pub model_name: String,
    pub provider_name: String,
    pub base_url: String,
    pub api_key: String,
    pub timeout_secs: i64,
    pub kind: Kind,
}

#[derive(Debug)]
pub enum Resolved {
    /// Upstreams to try, in order.
    Targets(Vec<Target>),
    /// A combo with that name exists but none of its models are enabled.
    EmptyCombo,
    NotFound,
}

/// `SELECT` for [`Target`] followed by the given SQL tail, as one string literal.
macro_rules! select_targets {
    ($tail:literal) => {
        concat!(
            "SELECT m.id AS model_id, m.name AS model_name, p.name AS provider_name,
                    p.base_url, p.api_key, p.timeout_secs, p.kind
             FROM models m JOIN providers p ON p.id = m.provider_id ",
            $tail
        )
    };
}

/// Resolution order, among providers and combos of `kind` only:
/// 1. a combo with that name: its models in position order (priority fallback);
/// 2. a model id served by one or more providers: every enabled provider, oldest first;
/// 3. `provider/model`: that one model.
pub async fn resolve(db: &SqlitePool, requested: &str, kind: Kind) -> sqlx::Result<Resolved> {
    let combo: Option<(i64,)> = sqlx::query_as("SELECT id FROM combos WHERE name = ? AND kind = ?")
        .bind(requested)
        .bind(kind)
        .fetch_optional(db)
        .await?;
    if let Some((combo_id,)) = combo {
        let targets = targets_for_combo(db, combo_id).await?;
        return Ok(if targets.is_empty() {
            Resolved::EmptyCombo
        } else {
            Resolved::Targets(targets)
        });
    }

    let targets: Vec<Target> = sqlx::query_as(select_targets!(
        "WHERE m.name = ? AND p.kind = ? AND m.enabled AND p.enabled ORDER BY p.id"
    ))
    .bind(requested)
    .bind(kind)
    .fetch_all(db)
    .await?;
    if !targets.is_empty() {
        return Ok(Resolved::Targets(targets));
    }

    if let Some((provider, model)) = requested.split_once('/') {
        let targets: Vec<Target> = sqlx::query_as(select_targets!(
            "WHERE p.name = ? AND m.name = ? AND p.kind = ? AND m.enabled AND p.enabled"
        ))
        .bind(provider)
        .bind(model)
        .bind(kind)
        .fetch_all(db)
        .await?;
        if !targets.is_empty() {
            return Ok(Resolved::Targets(targets));
        }
    }
    Ok(Resolved::NotFound)
}

/// Names a client can send as `model`: combos first, then models.
pub async fn public_model_ids(db: &SqlitePool) -> sqlx::Result<Vec<(String, String)>> {
    let mut ids: Vec<(String, String)> =
        sqlx::query_as::<_, (String,)>("SELECT name FROM combos ORDER BY name")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|(n,)| (n, "ai-gateway".to_string()))
            .collect();
    let models: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT m.name, p.name FROM models m JOIN providers p ON p.id = m.provider_id
         WHERE m.enabled AND p.enabled ORDER BY m.name",
    )
    .fetch_all(db)
    .await?;
    for (model, provider) in models {
        if !ids.iter().any(|(id, _)| *id == model) {
            ids.push((model, provider));
        }
    }
    Ok(ids)
}

/// One model as a target, whether or not it is enabled (for admin tests).
pub async fn target_for_model(db: &SqlitePool, model_id: i64) -> sqlx::Result<Target> {
    sqlx::query_as(select_targets!("WHERE m.id = ?"))
        .bind(model_id)
        .fetch_one(db)
        .await
}

/// Every model of a provider as targets, enabled or not (for admin tests).
pub async fn targets_for_provider(db: &SqlitePool, provider_id: i64) -> sqlx::Result<Vec<Target>> {
    sqlx::query_as(select_targets!("WHERE p.id = ? ORDER BY m.name"))
        .bind(provider_id)
        .fetch_all(db)
        .await
}

/// The combo's enabled models in fallback order, exactly as a client request would try them.
pub async fn targets_for_combo(db: &SqlitePool, combo_id: i64) -> sqlx::Result<Vec<Target>> {
    sqlx::query_as(select_targets!(
        "JOIN combo_entries e ON e.model_id = m.id
         WHERE e.combo_id = ? AND m.enabled AND p.enabled ORDER BY e.position"
    ))
    .bind(combo_id)
    .fetch_all(db)
    .await
}
