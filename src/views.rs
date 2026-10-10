//! View helpers: page chrome, flash messages, number formatting and the activity chart geometry.
//! Templates call these as methods so no logic lives in the HTML.

use actix_session::Session;
use actix_web::HttpResponse;
use actix_web::http::header;
use askama::Template;

use crate::error::WebResult;
use crate::models::api_key::ApiKeyRow;
use crate::models::request_log::{AttemptRow, RequestRow};
use crate::models::stats::{Bucket, KeyStat, ModelStat, RequestedStat, Totals};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Dashboard,
    Requests,
    Providers,
    Combos,
    Keys,
}

pub struct Flash {
    pub ok: bool,
    pub text: String,
}

/// Shared page chrome.
pub struct Page {
    pub title: String,
    pub nav: Nav,
    pub flash: Option<Flash>,
}

impl Page {
    pub fn new(session: &Session, title: impl Into<String>, nav: Nav) -> Self {
        Self {
            title: title.into(),
            nav,
            flash: take_flash(session),
        }
    }

    pub fn is(&self, nav: Nav) -> bool {
        self.nav == nav
    }
}

const FLASH_KEY: &str = "flash";

pub fn flash(session: &Session, ok: bool, text: impl Into<String>) {
    let _ = session.insert(FLASH_KEY, (ok, text.into()));
}

fn take_flash(session: &Session) -> Option<Flash> {
    let (ok, text) = session.remove_as::<(bool, String)>(FLASH_KEY)?.ok()?;
    Some(Flash { ok, text })
}

pub fn render(t: impl Template) -> WebResult<HttpResponse> {
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(t.render()?))
}

/// POST/redirect/GET.
pub fn redirect(to: &str) -> HttpResponse {
    HttpResponse::SeeOther()
        .insert_header((header::LOCATION, to))
        .finish()
}

/// 1234567 -> "1,234,567".
pub fn int(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// 1234567 -> "1.2M"; small numbers stay exact.
pub fn compact(n: i64) -> String {
    let f = n as f64;
    match n.abs() {
        0..=9_999 => int(n),
        10_000..=999_999 => format!("{:.1}K", f / 1e3),
        1_000_000..=999_999_999 => format!("{:.1}M", f / 1e6),
        _ => format!("{:.2}B", f / 1e9),
    }
}

pub fn ms(v: f64) -> String {
    if v <= 0.0 {
        "—".into()
    } else if v < 1000.0 {
        format!("{v:.0} ms")
    } else {
        format!("{:.1} s", v / 1000.0)
    }
}

pub fn pct(part: i64, whole: i64) -> String {
    if whole == 0 {
        "—".into()
    } else {
        format!("{:.1}%", part as f64 * 100.0 / whole as f64)
    }
}

/// "2026-10-10T05:36:43Z" -> "2026-10-10 05:36" (UTC).
pub fn when(ts: &str) -> String {
    ts.get(..16).unwrap_or(ts).replace('T', " ")
}

impl Totals {
    pub fn requests_s(&self) -> String {
        int(self.requests)
    }
    pub fn success_rate(&self) -> String {
        pct(self.ok, self.requests)
    }
    pub fn failed_s(&self) -> String {
        int(self.failed)
    }
    pub fn fallbacks_s(&self) -> String {
        int(self.fallbacks)
    }
    pub fn tokens_s(&self) -> String {
        compact(self.total_tokens)
    }
    pub fn prompt_s(&self) -> String {
        compact(self.prompt_tokens)
    }
    pub fn completion_s(&self) -> String {
        compact(self.completion_tokens)
    }
    pub fn latency_s(&self) -> String {
        ms(self.avg_latency_ms)
    }
}

impl ModelStat {
    pub fn answered_s(&self) -> String {
        int(self.answered)
    }
    pub fn attempts_s(&self) -> String {
        int(self.attempts)
    }
    pub fn failed_s(&self) -> String {
        int(self.failed)
    }
    pub fn success_rate(&self) -> String {
        pct(self.ok, self.attempts)
    }
    pub fn tokens_s(&self) -> String {
        compact(self.total_tokens)
    }
    pub fn latency_s(&self) -> String {
        ms(self.avg_latency_ms)
    }
    /// Share of all answered requests, 0–100, for the inline bar.
    pub fn share(&self, total_answered: &i64) -> f64 {
        if *total_answered == 0 {
            0.0
        } else {
            self.answered as f64 * 100.0 / *total_answered as f64
        }
    }
    pub fn share_s(&self, total_answered: &i64) -> String {
        pct(self.answered, *total_answered)
    }
}

impl KeyStat {
    pub fn requests_s(&self) -> String {
        int(self.requests)
    }
    pub fn failed_s(&self) -> String {
        int(self.failed)
    }
    pub fn tokens_s(&self) -> String {
        compact(self.total_tokens)
    }
}

impl RequestedStat {
    pub fn requests_s(&self) -> String {
        int(self.requests)
    }
    pub fn failed_s(&self) -> String {
        int(self.failed)
    }
}

impl RequestRow {
    pub fn when(&self) -> String {
        when(&self.created_at)
    }
    pub fn ok(&self) -> bool {
        self.status == "ok"
    }
    pub fn latency_s(&self) -> String {
        ms(self.latency_ms as f64)
    }
    pub fn tokens_s(&self) -> String {
        if self.total_tokens == 0 {
            "—".into()
        } else {
            int(self.total_tokens)
        }
    }
    /// Provider and model that answered; failed requests have none.
    pub fn answered_by(&self) -> String {
        match (&self.provider_name, &self.model_name) {
            (Some(p), Some(m)) if self.ok() => format!("{p} / {m}"),
            _ => "—".into(),
        }
    }
}

impl ApiKeyRow {
    pub fn created_s(&self) -> String {
        when(&self.created_at)
    }
    pub fn last_used_s(&self) -> String {
        self.last_used_at
            .as_deref()
            .map(when)
            .unwrap_or_else(|| "never".into())
    }
}

impl AttemptRow {
    pub fn latency_s(&self) -> String {
        ms(self.latency_ms as f64)
    }
    pub fn status_s(&self) -> String {
        self.http_status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".into())
    }
}

/// Server-rendered stacked bar chart: succeeded and failed requests per bucket.
pub struct Chart {
    pub width: f64,
    pub height: f64,
    pub plot_bottom: f64,
    pub plot_left: f64,
    pub bars: Vec<Bar>,
    pub ticks: Vec<Tick>,
    pub labels: Vec<XLabel>,
    pub empty: bool,
}

pub struct Bar {
    pub x: f64,
    pub w: f64,
    /// Succeeded segment (sits on the baseline).
    pub ok_y: f64,
    pub ok_h: f64,
    /// Failed segment, stacked on top with a 2px gap.
    pub fail_y: f64,
    pub fail_h: f64,
    /// Full-height hit target.
    pub hit_y: f64,
    pub hit_h: f64,
    pub tooltip: String,
}

pub struct Tick {
    pub y: f64,
    pub label: String,
}

pub struct XLabel {
    pub x: f64,
    pub text: String,
}

impl Chart {
    pub fn build(buckets: &[Bucket], hourly: bool) -> Self {
        let (width, height) = (960.0, 220.0);
        let (plot_left, plot_right, plot_top, plot_bottom) = (44.0, 952.0, 12.0, 192.0);
        let plot_h = plot_bottom - plot_top;

        let max = buckets.iter().map(|b| b.requests).max().unwrap_or(0);
        let top = nice_ceiling(max.max(1));
        let scale = |v: i64| v as f64 / top as f64 * plot_h;

        let n = buckets.len().max(1) as f64;
        let slot = (plot_right - plot_left) / n;
        let w = (slot * 0.62).clamp(4.0, 28.0);
        let gap = 2.0;

        let bars = buckets
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let x = plot_left + slot * i as f64 + (slot - w) / 2.0;
                let ok = b.requests - b.failed;
                let ok_h = scale(ok);
                let ok_y = plot_bottom - ok_h;
                let fail_h = if b.failed > 0 {
                    (scale(b.failed) - gap).max(2.0)
                } else {
                    0.0
                };
                let fail_y = ok_y - gap - fail_h;
                let label = bucket_label(&b.key, hourly, true);
                Bar {
                    x,
                    w,
                    ok_y,
                    ok_h,
                    fail_y,
                    fail_h,
                    hit_y: plot_top,
                    hit_h: plot_h,
                    tooltip: format!(
                        "{label}\n{} requests · {} failed · {} tokens",
                        int(b.requests),
                        int(b.failed),
                        compact(b.tokens)
                    ),
                }
            })
            .collect();

        let ticks = (0..=4)
            .map(|i| {
                let v = top * i / 4;
                Tick {
                    y: plot_bottom - scale(v),
                    label: compact(v),
                }
            })
            .collect();

        // Label every Nth bucket so labels never collide.
        let every = if hourly {
            4
        } else if buckets.len() > 10 {
            5
        } else {
            1
        };
        let labels = buckets
            .iter()
            .enumerate()
            .filter(|(i, _)| (buckets.len() - 1 - i).is_multiple_of(every))
            .map(|(i, b)| XLabel {
                x: plot_left + slot * i as f64 + slot / 2.0,
                text: bucket_label(&b.key, hourly, false),
            })
            .collect();

        Self {
            width,
            height,
            plot_bottom,
            plot_left,
            bars,
            ticks,
            labels,
            empty: max == 0,
        }
    }
}

/// Axis maximum: four gridline steps of a round size (1, 2, 2.5 or 5 × 10^k) covering `v`.
fn nice_ceiling(v: i64) -> i64 {
    let need = (v + 3) / 4; // smallest step that reaches v in four ticks
    let mut base = 1;
    loop {
        // 2.5 only once it is a whole number.
        let half_step = if base >= 10 { Some(5 * base / 2) } else { None };
        for step in [Some(base), Some(2 * base), half_step, Some(5 * base)]
            .into_iter()
            .flatten()
        {
            if step >= need {
                return step * 4;
            }
        }
        base *= 10;
    }
}

fn bucket_label(key: &str, hourly: bool, long: bool) -> String {
    if hourly {
        // "2026-10-10T05" -> "05:00" (UTC)
        let hour = key.get(11..13).unwrap_or("00");
        if long {
            format!("{} {hour}:00 UTC", key.get(..10).unwrap_or(key))
        } else {
            format!("{hour}:00")
        }
    } else {
        // "2026-10-10" -> "Oct 10"
        chrono::NaiveDate::parse_from_str(key, "%Y-%m-%d")
            .map(|d| {
                d.format(if long { "%a, %b %-d" } else { "%b %-d" })
                    .to_string()
            })
            .unwrap_or_else(|_| key.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers() {
        assert_eq!(int(1234567), "1,234,567");
        assert_eq!(int(-1000), "-1,000");
        assert_eq!(int(999), "999");
        assert_eq!(compact(1_234_567), "1.2M");
        assert_eq!(compact(9_999), "9,999");
        assert_eq!(pct(1, 4), "25.0%");
        assert_eq!(pct(1, 0), "—");
        assert_eq!(ms(1500.0), "1.5 s");
    }

    #[test]
    fn nice_ceiling_is_round_and_divisible_into_four_ticks() {
        for (v, want) in [
            (1, 4),
            (3, 4),
            (7, 8),
            (13, 20),
            (97, 100),
            (101, 200),
            (4321, 8000),
        ] {
            assert_eq!(nice_ceiling(v), want, "v={v}");
            assert_eq!(nice_ceiling(v) % 4, 0);
        }
    }

    #[test]
    fn chart_marks_empty_and_stacks_failures_above_success() {
        let buckets = vec![
            Bucket {
                key: "2026-10-10T04".into(),
                requests: 0,
                failed: 0,
                tokens: 0,
            },
            Bucket {
                key: "2026-10-10T05".into(),
                requests: 10,
                failed: 2,
                tokens: 100,
            },
        ];
        let c = Chart::build(&buckets, true);
        assert!(!c.empty);
        let b = &c.bars[1];
        assert!(
            b.fail_y + b.fail_h < b.ok_y,
            "failed segment sits above the success segment"
        );
        assert!(Chart::build(&buckets[..1], true).empty);
    }
}
