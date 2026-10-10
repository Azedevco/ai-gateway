//! Latest admin test result per model, kept in memory (tests are not part of the request log).

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::services::upstream::Probe;

pub struct ProbeResult {
    pub probe: Probe,
    pub at: DateTime<Utc>,
}

#[derive(Default)]
pub struct Probes(Mutex<HashMap<i64, ProbeResult>>);

impl Probes {
    pub fn store(&self, model_id: i64, probe: Probe) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(
                model_id,
                ProbeResult {
                    probe,
                    at: Utc::now(),
                },
            );
        }
    }

    /// Display-ready results for the given models.
    pub fn view(&self, model_ids: impl IntoIterator<Item = i64>) -> HashMap<i64, ProbeView> {
        let Ok(map) = self.0.lock() else {
            return HashMap::new();
        };
        model_ids
            .into_iter()
            .filter_map(|id| map.get(&id).map(|r| (id, ProbeView::from(r))))
            .collect()
    }
}

/// What the templates show next to a model.
pub struct ProbeView {
    pub ok: bool,
    /// e.g. "820 ms · OK" or "401 · invalid api key".
    pub summary: String,
    /// e.g. "2 min ago".
    pub age: String,
}

impl From<&ProbeResult> for ProbeView {
    fn from(r: &ProbeResult) -> Self {
        let p = &r.probe;
        let latency = crate::views::ms(p.latency_ms as f64);
        let summary = match (p.ok, p.status) {
            (true, _) => format!("{latency} · {}", p.detail),
            (false, Some(status)) => format!("{status} · {}", p.detail),
            (false, None) => p.detail.clone(),
        };
        let secs = (Utc::now() - r.at).num_seconds().max(0);
        let age = match secs {
            0..=59 => "just now".to_string(),
            60..=3599 => format!("{} min ago", secs / 60),
            _ => format!("{} h ago", secs / 3600),
        };
        Self {
            ok: p.ok,
            summary,
            age,
        }
    }
}
