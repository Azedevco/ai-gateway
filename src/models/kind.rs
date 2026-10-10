use serde::Deserialize;

/// What an upstream speaks. Providers and combos have one kind; a combo only holds models of
/// its own kind.
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

    /// Admin page that lists providers and combos of this kind.
    pub fn home(self) -> &'static str {
        match self {
            Kind::Chat => "/providers",
            Kind::Decision => "/decision",
        }
    }

    /// Admin page that lists combos of this kind.
    pub fn combos_home(self) -> &'static str {
        match self {
            Kind::Chat => "/combos",
            Kind::Decision => "/decision",
        }
    }

    pub fn home_label(self) -> &'static str {
        match self {
            Kind::Chat => "Providers",
            Kind::Decision => "Decision models",
        }
    }
}
