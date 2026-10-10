//! Calls upstream providers, falling back down the target list until one answers.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use actix_web::HttpResponse;
use actix_web::http::StatusCode;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::error::ApiError;
use crate::models::kind::Kind;
use crate::models::request_log::{self, Attempt, RequestRecord, Usage};
use crate::services::router::Target;

/// Path of the TypeSafe System One endpoint, relative to a provider's base URL.
pub const DECISION_ENDPOINT: &str = "systemone";

/// Upstream error bodies are kept short in the log.
const MAX_ERROR_CHARS: usize = 300;

pub struct Upstream {
    client: reqwest::Client,
}

/// What the client asked for, minus the routing decision.
pub struct ProxyRequest {
    /// Path after `/v1/`, e.g. `chat/completions`.
    pub endpoint: String,
    pub body: Value,
    pub api_key_id: Option<i64>,
}

/// Result of a model test from the admin UI.
pub struct Probe {
    pub ok: bool,
    pub status: Option<u16>,
    pub latency_ms: i64,
    /// Start of the reply, or the error.
    pub detail: String,
}

struct Answer {
    status: StatusCode,
    content_type: Option<reqwest::header::HeaderValue>,
    bytes: Bytes,
    json: Value,
}

struct Failure {
    status: Option<StatusCode>,
    error: String,
}

impl Failure {
    fn new(status: Option<StatusCode>, error: String) -> Self {
        Self { status, error }
    }
}

enum Outcome {
    Done(HttpResponse),
    /// Try the next target. `status` is the upstream HTTP status when there was one.
    Retry {
        status: Option<StatusCode>,
        error: String,
    },
}

impl Upstream {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("ai-gateway/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client })
    }

    pub async fn proxy(
        &self,
        db: &SqlitePool,
        req: ProxyRequest,
        targets: Vec<Target>,
    ) -> Result<HttpResponse, ApiError> {
        let started = Instant::now();
        let requested_model = req.body["model"].as_str().unwrap_or_default().to_string();
        let stream = req.body["stream"].as_bool().unwrap_or(false);
        let mut record = RequestRecord {
            api_key_id: req.api_key_id,
            requested_model,
            stream,
            http_status: 502,
            latency_ms: 0,
            usage: Usage::default(),
            attempts: Vec::with_capacity(targets.len()),
            error: None,
        };

        let mut last_status = None;
        let mut last_error = String::new();
        for target in &targets {
            let attempt_started = Instant::now();
            let body = upstream_body(&req.body, target, stream);
            let outcome = if stream {
                self.send_stream(db, target, &req.endpoint, body, &mut record, started)
                    .await
            } else {
                self.send_once(target, &req.endpoint, body, &mut record)
                    .await
            };
            let latency_ms = attempt_started.elapsed().as_millis() as i64;

            match outcome {
                Outcome::Done(response) => {
                    if !stream {
                        // Streams record themselves when the body finishes.
                        record.latency_ms = started.elapsed().as_millis() as i64;
                        request_log::record(db.clone(), record);
                    }
                    return Ok(response);
                }
                Outcome::Retry { status, error } => {
                    tracing::warn!(
                        provider = %target.provider_name, model = %target.model_name,
                        status = ?status, error = %error, "upstream attempt failed, falling back"
                    );
                    record.attempts.push(Attempt {
                        model_id: target.model_id,
                        model_name: target.model_name.clone(),
                        provider_name: target.provider_name.clone(),
                        ok: false,
                        http_status: status.map(|s| s.as_u16()),
                        latency_ms,
                        error: Some(error.clone()),
                    });
                    last_status = status;
                    last_error = format!("{}/{}: {error}", target.provider_name, target.model_name);
                }
            }
        }

        // Every target failed. 4xx from the last upstream is passed through (most likely a client
        // error); anything else becomes 502.
        let status = last_status
            .filter(|s| s.is_client_error())
            .unwrap_or(StatusCode::BAD_GATEWAY);
        let message = format!(
            "all {} upstream attempt(s) failed; last: {last_error}",
            record.attempts.len()
        );
        record.http_status = status.as_u16();
        record.latency_ms = started.elapsed().as_millis() as i64;
        record.error = Some(message.clone());
        request_log::record(db.clone(), record);
        Err(ApiError::Upstream { status, message })
    }

    /// Model ids a provider advertises at `GET {base_url}/models`.
    pub async fn fetch_models(&self, base_url: &str, api_key: &str) -> anyhow::Result<Vec<String>> {
        let mut rb = self
            .client
            .get(format!("{}/models", base_url.trim_end_matches('/')))
            .timeout(Duration::from_secs(20));
        if !api_key.is_empty() {
            rb = rb.bearer_auth(api_key);
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!(
                "{status}: {}",
                truncate(&resp.text().await.unwrap_or_default())
            );
        }
        let json: Value = resp.json().await?;
        let mut ids: Vec<String> = json["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("response has no `data` list"))?
            .iter()
            .filter_map(|m| m["id"].as_str().map(str::to_string))
            .collect();
        ids.sort();
        Ok(ids)
    }

    fn request(&self, target: &Target, endpoint: &str, body: Value) -> reqwest::RequestBuilder {
        let url = format!("{}/{endpoint}", target.base_url.trim_end_matches('/'));
        let mut rb = self.client.post(url).json(&body);
        if !target.api_key.is_empty() {
            rb = rb.bearer_auth(&target.api_key);
        }
        rb
    }

    /// One non-streaming call with the same success rules the proxy uses: a 2xx JSON answer,
    /// and for completion endpoints at least one choice.
    async fn call_json(
        &self,
        target: &Target,
        endpoint: &str,
        body: Value,
    ) -> Result<Answer, Failure> {
        let timeout = Duration::from_secs(target.timeout_secs as u64);
        let call = async {
            let resp = self.request(target, endpoint, body).send().await?;
            let status = resp.status();
            let content_type = resp.headers().get(reqwest::header::CONTENT_TYPE).cloned();
            let bytes = resp.bytes().await?;
            Ok::<_, reqwest::Error>((status, content_type, bytes))
        };
        let (status, content_type, bytes) = match tokio::time::timeout(timeout, call).await {
            Err(_) => {
                return Err(Failure::new(
                    None,
                    format!("timed out after {}s", target.timeout_secs),
                ));
            }
            Ok(Err(e)) => return Err(Failure::new(e.status().map(to_actix_status), describe(&e))),
            Ok(Ok(parts)) => parts,
        };
        let status = to_actix_status(status);
        if !status.is_success() {
            return Err(Failure::new(
                Some(status),
                truncate(&String::from_utf8_lossy(&bytes)),
            ));
        }

        // A 2xx without a usable answer (HTML error page, empty choices/answers) is a failure too.
        let json: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Failure::new(Some(status), "response is not JSON".into()))?;
        if endpoint.ends_with("completions")
            && json["choices"].as_array().is_none_or(|c| c.is_empty())
        {
            return Err(Failure::new(Some(status), "response has no choices".into()));
        }
        if endpoint == DECISION_ENDPOINT && json["answers"].as_object().is_none_or(|a| a.is_empty())
        {
            return Err(Failure::new(Some(status), "response has no answers".into()));
        }
        Ok(Answer {
            status,
            content_type,
            bytes,
            json,
        })
    }

    /// Sends a tiny chat request straight to one model. Used by the admin "Test" buttons;
    /// nothing is recorded in the request log.
    pub async fn probe(&self, target: &Target) -> Probe {
        let started = Instant::now();
        let (endpoint, body) = match target.kind {
            Kind::Chat => (
                "chat/completions",
                serde_json::json!({
                    "model": target.model_name,
                    "messages": [{"role": "user", "content": "Reply with the single word: OK"}],
                    "max_tokens": 16,
                }),
            ),
            Kind::Decision => (
                DECISION_ENDPOINT,
                serde_json::json!({
                    "model": target.model_name,
                    "state": {"check": "gateway health check"},
                    "questions": {"ok": {"type": "noul", "instructions": "Is 2 + 2 equal to 4?"}},
                }),
            ),
        };
        let result = self.call_json(target, endpoint, body).await;
        let latency_ms = started.elapsed().as_millis() as i64;
        match result {
            Ok(answer) => {
                let reply = match target.kind {
                    Kind::Chat => chat_reply(&answer.json),
                    Kind::Decision => decision_reply(&answer.json),
                };
                Probe {
                    ok: true,
                    status: Some(answer.status.as_u16()),
                    latency_ms,
                    detail: reply,
                }
            }
            Err(f) => Probe {
                ok: false,
                status: f.status.map(|s| s.as_u16()),
                latency_ms,
                detail: f.error,
            },
        }
    }

    async fn send_once(
        &self,
        target: &Target,
        endpoint: &str,
        body: Value,
        record: &mut RequestRecord,
    ) -> Outcome {
        let started = Instant::now();
        let Answer {
            status,
            content_type,
            bytes,
            json,
        } = match self.call_json(target, endpoint, body).await {
            Ok(a) => a,
            Err(f) => return retry(f.status, f.error),
        };

        record.usage = Usage::from_json(&json).unwrap_or_default();
        record.http_status = status.as_u16();
        record.attempts.push(success(target, started));
        let mut response = HttpResponse::build(status);
        gateway_headers(&mut response, target, record.attempts.len());
        if let Some(ct) = content_type {
            response.insert_header((actix_web::http::header::CONTENT_TYPE, ct.as_bytes()));
        }
        Outcome::Done(response.body(bytes))
    }

    /// Fallback is only possible until the upstream returns its status line; after that the
    /// response is streamed to the client as-is.
    async fn send_stream(
        &self,
        db: &SqlitePool,
        target: &Target,
        endpoint: &str,
        body: Value,
        record: &mut RequestRecord,
        request_started: Instant,
    ) -> Outcome {
        let started = Instant::now();
        let timeout = Duration::from_secs(target.timeout_secs as u64);
        let resp = match tokio::time::timeout(timeout, self.request(target, endpoint, body).send())
            .await
        {
            Err(_) => return retry(None, format!("no response within {}s", target.timeout_secs)),
            Ok(Err(e)) => return retry(e.status().map(to_actix_status), describe(&e)),
            Ok(Ok(resp)) => resp,
        };
        let status = to_actix_status(resp.status());
        if !status.is_success() {
            let text = tokio::time::timeout(Duration::from_secs(10), resp.text()).await;
            return retry(
                Some(status),
                truncate(&text.ok().and_then(Result::ok).unwrap_or_default()),
            );
        }

        let mut finished = record.clone();
        finished.http_status = status.as_u16();
        finished.attempts.push(success(target, started));
        let tap = UsageTap::new(
            resp.bytes_stream().boxed(),
            db.clone(),
            finished,
            request_started,
        );

        let mut response = HttpResponse::build(status);
        gateway_headers(&mut response, target, record.attempts.len() + 1);
        response
            .insert_header((actix_web::http::header::CONTENT_TYPE, "text/event-stream"))
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-cache"))
            .insert_header(("X-Accel-Buffering", "no"));
        Outcome::Done(response.streaming(tap))
    }
}

/// The client body with `model` rewritten to the upstream id. Streams ask for usage in the
/// final chunk so tokens can be counted.
fn upstream_body(body: &Value, target: &Target, stream: bool) -> Value {
    let mut body = body.clone();
    body["model"] = Value::String(target.model_name.clone());
    if stream && target.kind == Kind::Chat && body.get("stream_options").is_none() {
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
    body
}

/// Start of a chat reply. Reasoning models may spend the budget on `reasoning_content` and
/// leave `content` empty.
fn chat_reply(json: &Value) -> String {
    let message = &json["choices"][0]["message"];
    [&message["content"], &message["reasoning_content"]]
        .into_iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .find(|t| !t.is_empty())
        .map(|t| truncate(t).chars().take(80).collect())
        .unwrap_or_else(|| "(empty reply)".into())
}

/// The health-check Noul answer, e.g. "P(yes) 0.99".
fn decision_reply(json: &Value) -> String {
    match json["answers"]["ok"]["noul"].as_f64() {
        Some(p) => format!("P(yes) {p:.2}"),
        None => "answered".into(),
    }
}

fn gateway_headers(
    response: &mut actix_web::HttpResponseBuilder,
    target: &Target,
    attempts: usize,
) {
    response
        .insert_header(("X-Gateway-Provider", target.provider_name.as_str()))
        .insert_header(("X-Gateway-Model", target.model_name.as_str()))
        .insert_header(("X-Gateway-Attempts", attempts.to_string()));
}

fn success(target: &Target, started: Instant) -> Attempt {
    Attempt {
        model_id: target.model_id,
        model_name: target.model_name.clone(),
        provider_name: target.provider_name.clone(),
        ok: true,
        http_status: Some(200),
        latency_ms: started.elapsed().as_millis() as i64,
        error: None,
    }
}

fn retry(status: Option<StatusCode>, error: String) -> Outcome {
    Outcome::Retry { status, error }
}

fn to_actix_status(s: reqwest::StatusCode) -> StatusCode {
    StatusCode::from_u16(s.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY)
}

fn describe(e: &reqwest::Error) -> String {
    if e.is_connect() {
        "connection failed".into()
    } else if e.is_timeout() {
        "timed out".into()
    } else {
        truncate(&e.to_string())
    }
}

fn truncate(s: &str) -> String {
    let s = s.trim();
    match s.char_indices().nth(MAX_ERROR_CHARS) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Passes an SSE body through unchanged while reading `usage` from its `data:` lines, and
/// records the request when the stream ends (or the client disconnects).
struct UsageTap {
    inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    line: Vec<u8>,
    db: SqlitePool,
    record: Option<RequestRecord>,
    started: Instant,
}

impl UsageTap {
    fn new(
        inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
        db: SqlitePool,
        record: RequestRecord,
        started: Instant,
    ) -> Self {
        Self {
            inner,
            line: Vec::new(),
            db,
            record: Some(record),
            started,
        }
    }

    fn scan_usage(&mut self, chunk: &[u8]) {
        for &b in chunk {
            if b != b'\n' {
                self.line.push(b);
                continue;
            }
            let line = std::mem::take(&mut self.line);
            let Some(data) = line.strip_prefix(b"data:") else {
                continue;
            };
            // Cheap pre-check: only the final chunk carries usage.
            if !data.windows(7).any(|w| w == b"\"usage\"") {
                continue;
            }
            if let (Ok(v), Some(record)) = (
                serde_json::from_slice::<Value>(data.trim_ascii()),
                self.record.as_mut(),
            ) && let Some(usage) = Usage::from_json(&v)
            {
                record.usage = usage;
            }
        }
    }

    fn finish(&mut self, error: Option<String>) {
        if let Some(mut record) = self.record.take() {
            record.latency_ms = self.started.elapsed().as_millis() as i64;
            if let Some(e) = error {
                record.error = Some(e);
            }
            request_log::record(self.db.clone(), record);
        }
    }
}

impl Stream for UsageTap {
    type Item = Result<Bytes, actix_web::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                self.scan_usage(&chunk);
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(e))) => {
                let message = format!("stream interrupted: {}", describe(&e));
                self.finish(Some(message));
                Poll::Ready(Some(Err(actix_web::error::ErrorBadGateway(e))))
            }
            Poll::Ready(None) => {
                self.finish(None);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for UsageTap {
    fn drop(&mut self) {
        // Client went away before the upstream finished.
        if self.record.is_some() {
            self.finish(Some("client disconnected before the stream ended".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        Target {
            model_id: 1,
            model_name: "deepseek-chat".into(),
            provider_name: "deepseek".into(),
            base_url: "https://api.example.com/v1".into(),
            api_key: String::new(),
            timeout_secs: 30,
            kind: Kind::Chat,
        }
    }

    #[test]
    fn body_uses_upstream_model_and_requests_stream_usage() {
        let body = serde_json::json!({"model": "priority", "messages": []});
        let out = upstream_body(&body, &target(), true);
        assert_eq!(out["model"], "deepseek-chat");
        assert_eq!(out["stream_options"]["include_usage"], true);

        let out = upstream_body(&body, &target(), false);
        assert!(out.get("stream_options").is_none());

        let decision = Target {
            kind: Kind::Decision,
            ..target()
        };
        let out = upstream_body(&body, &decision, true);
        assert!(
            out.get("stream_options").is_none(),
            "System One has no streaming"
        );
    }

    #[test]
    fn truncate_is_char_safe() {
        let s = "ə".repeat(MAX_ERROR_CHARS + 10);
        assert!(truncate(&s).ends_with('…'));
        assert_eq!(truncate("short"), "short");
    }
}
