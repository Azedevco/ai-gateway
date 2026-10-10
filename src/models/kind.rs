use serde::Deserialize;

/// What a model speaks. A provider can serve both kinds; a combo has one kind and only holds
/// models of that kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum Kind {
    /// OpenAI-compatible: `/chat/completions`, `/completions`, `/embeddings`.
    #[default]
    Chat,
    /// TypeSafe System One compatible (Jev-style): `/systemone` with typed questions.
    Decision,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Chat => "chat",
            Kind::Decision => "decision",
        }
    }

    pub fn is_decision(&self) -> bool {
        *self == Kind::Decision
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Chat => "Chat",
            Kind::Decision => "Decision",
        }
    }

    pub const ALL: [Kind; 2] = [Kind::Chat, Kind::Decision];
}
