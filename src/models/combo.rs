use serde::Deserialize;
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, FromRow)]
pub struct Combo {
    pub id: i64,
    pub name: String,
    pub description: String,
}

#[derive(Debug, FromRow)]
pub struct ComboRow {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub entry_count: i64,
    /// First usable model in the chain, if any.
    pub primary: Option<String>,
}

#[derive(Debug, FromRow)]
pub struct EntryRow {
    pub model_id: i64,
    pub model_name: String,
    pub provider_name: String,
    pub usable: bool,
}

#[derive(Debug, Deserialize)]
pub struct ComboForm {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl ComboForm {
    pub fn validate(&self) -> Result<(), &'static str> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Name is required.");
        }
        if name.contains(char::is_whitespace) {
            return Err("Name cannot contain spaces; clients send it as the model name.");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Up,
    Down,
}

pub async fn list(db: &SqlitePool) -> sqlx::Result<Vec<ComboRow>> {
    sqlx::query_as(
        "SELECT c.id, c.name, c.description,
                (SELECT COUNT(*) FROM combo_entries e WHERE e.combo_id = c.id) AS entry_count,
                (SELECT p.name || ' / ' || m.name FROM combo_entries e
                   JOIN models m ON m.id = e.model_id JOIN providers p ON p.id = m.provider_id
                  WHERE e.combo_id = c.id AND m.enabled AND p.enabled
                  ORDER BY e.position LIMIT 1) AS \"primary\"
         FROM combos c ORDER BY c.name",
    )
    .fetch_all(db)
    .await
}

pub async fn get(db: &SqlitePool, id: i64) -> sqlx::Result<Combo> {
    sqlx::query_as("SELECT id, name, description FROM combos WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await
}

pub async fn entries(db: &SqlitePool, combo_id: i64) -> sqlx::Result<Vec<EntryRow>> {
    sqlx::query_as(
        "SELECT e.model_id, m.name AS model_name, p.name AS provider_name,
                (m.enabled AND p.enabled) AS usable
         FROM combo_entries e
         JOIN models m ON m.id = e.model_id JOIN providers p ON p.id = m.provider_id
         WHERE e.combo_id = ? ORDER BY e.position",
    )
    .bind(combo_id)
    .fetch_all(db)
    .await
}

pub async fn create(db: &SqlitePool, form: &ComboForm) -> sqlx::Result<i64> {
    Ok(
        sqlx::query("INSERT INTO combos (name, description) VALUES (?, ?)")
            .bind(form.name.trim())
            .bind(form.description.trim())
            .execute(db)
            .await?
            .last_insert_rowid(),
    )
}

pub async fn update(db: &SqlitePool, id: i64, form: &ComboForm) -> sqlx::Result<()> {
    sqlx::query("UPDATE combos SET name = ?, description = ? WHERE id = ?")
        .bind(form.name.trim())
        .bind(form.description.trim())
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM combos WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// Appends a model to the end of the chain (no-op if it is already there).
pub async fn add_entry(db: &SqlitePool, combo_id: i64, model_id: i64) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO combo_entries (combo_id, model_id, position)
         VALUES (?, ?, COALESCE((SELECT MAX(position) + 1 FROM combo_entries WHERE combo_id = ?), 0))",
    )
    .bind(combo_id)
    .bind(model_id)
    .bind(combo_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn remove_entry(db: &SqlitePool, combo_id: i64, model_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM combo_entries WHERE combo_id = ? AND model_id = ?")
        .bind(combo_id)
        .bind(model_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Swaps an entry with its neighbour above or below.
pub async fn move_entry(
    db: &SqlitePool,
    combo_id: i64,
    model_id: i64,
    dir: Direction,
) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    let ids: Vec<(i64,)> =
        sqlx::query_as("SELECT model_id FROM combo_entries WHERE combo_id = ? ORDER BY position")
            .bind(combo_id)
            .fetch_all(&mut *tx)
            .await?;
    let mut ids: Vec<i64> = ids.into_iter().map(|r| r.0).collect();
    if let Some(i) = ids.iter().position(|&id| id == model_id) {
        let j = match dir {
            Direction::Up if i > 0 => i - 1,
            Direction::Down if i + 1 < ids.len() => i + 1,
            _ => i,
        };
        ids.swap(i, j);
        // Rewrite positions densely so gaps from deletions never accumulate.
        for (pos, id) in ids.iter().enumerate() {
            sqlx::query(
                "UPDATE combo_entries SET position = ? WHERE combo_id = ? AND model_id = ?",
            )
            .bind(pos as i64)
            .bind(combo_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await
}
