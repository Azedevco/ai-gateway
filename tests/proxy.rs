//! End-to-end: a real mock upstream over HTTP, the gateway app, and the stats it records.

use std::time::Duration;

use actix_web::{App, HttpResponse, HttpServer, test, web};
use ai_gateway::config::Config;
use ai_gateway::models::{api_key, combo, model, provider, stats};
use ai_gateway::services::upstream::Upstream;
use ai_gateway::{AppState, configure, db};
use serde_json::{Value, json};

/// OpenAI-compatible mock: model `broken` returns 500, `good` answers (optionally streaming).
async fn mock_chat(body: web::Json<Value>) -> HttpResponse {
    match body["model"].as_str() {
        Some("good") if body["stream"] == true => {
            assert_eq!(body["stream_options"]["include_usage"], true, "gateway asks for stream usage");
            let sse = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}\n\n",
                "data: [DONE]\n\n",
            );
            HttpResponse::Ok().content_type("text/event-stream").body(sse)
        }
        Some("good") => HttpResponse::Ok().json(json!({
            "id": "x", "object": "chat.completion", "model": "good",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        })),
        _ => HttpResponse::InternalServerError().body("boom"),
    }
}

async fn mock_models() -> HttpResponse {
    HttpResponse::Ok().json(json!({"data": [{"id": "good"}, {"id": "broken"}]}))
}

/// Starts the mock upstream on a free port and returns its `/v1` base URL.
fn start_upstream() -> String {
    let server = HttpServer::new(|| {
        App::new()
            .route("/v1/chat/completions", web::post().to(mock_chat))
            .route("/v1/models", web::get().to(mock_models))
    })
    .workers(1)
    .bind("127.0.0.1:0")
    .unwrap();
    let addr = server.addrs()[0];
    actix_web::rt::spawn(server.run());
    format!("http://{addr}/v1")
}

struct Fixture {
    state: web::Data<AppState>,
    key: String,
    db_path: std::path::PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.db_path.display()));
        }
    }
}

/// Provider `mock` with models `broken` and `good`; combo `priority` = [broken, good].
async fn fixture(name: &str) -> Fixture {
    let db_path =
        std::env::temp_dir().join(format!("ai-gateway-test-{name}-{}.db", std::process::id()));
    let pool = db::connect(&format!("sqlite://{}", db_path.display()))
        .await
        .unwrap();

    let provider_id = provider::create(
        &pool,
        &provider::ProviderForm {
            name: "mock".into(),
            base_url: start_upstream(),
            api_key: String::new(),
            timeout_secs: Some(10),
            enabled: Some("1".into()),
        },
    )
    .await
    .unwrap();
    model::add_many(&pool, provider_id, &["broken".into(), "good".into()])
        .await
        .unwrap();
    let models = model::list_for_provider(&pool, provider_id).await.unwrap();
    let id_of = |n: &str| models.iter().find(|m| m.name == n).unwrap().id;

    let combo_id = combo::create(
        &pool,
        &combo::ComboForm {
            name: "priority".into(),
            description: String::new(),
        },
    )
    .await
    .unwrap();
    combo::add_entry(&pool, combo_id, id_of("broken"))
        .await
        .unwrap();
    combo::add_entry(&pool, combo_id, id_of("good"))
        .await
        .unwrap();

    let only_broken = combo::create(
        &pool,
        &combo::ComboForm {
            name: "doomed".into(),
            description: String::new(),
        },
    )
    .await
    .unwrap();
    combo::add_entry(&pool, only_broken, id_of("broken"))
        .await
        .unwrap();

    let key = api_key::create(&pool, "test").await.unwrap();
    let state = web::Data::new(AppState {
        db: pool,
        config: Config {
            bind: String::new(),
            database_url: String::new(),
            admin_password: "password123".into(),
            session_secret: vec![7; 64],
            secure_cookies: false,
            retention_days: 0,
        },
        upstream: Upstream::new().unwrap(),
        probes: Default::default(),
    });
    Fixture {
        state,
        key,
        db_path,
    }
}

/// Request logs are written in the background; give them a moment.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[actix_web::test]
async fn combo_falls_back_and_records_usage() {
    let f = fixture("fallback").await;
    let app = test::init_service(App::new().configure(configure(f.state.clone()))).await;

    let req = test::TestRequest::post()
        .uri("/v1/chat/completions")
        .insert_header(("Authorization", format!("Bearer {}", f.key)))
        .set_json(json!({"model": "priority", "messages": [{"role": "user", "content": "hi"}]}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("X-Gateway-Model").unwrap(), "good");
    assert_eq!(resp.headers().get("X-Gateway-Attempts").unwrap(), "2");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["choices"][0]["message"]["content"], "Hello");

    settle().await;
    let totals = stats::totals(&f.state.db, stats::Range::Day).await.unwrap();
    assert_eq!((totals.requests, totals.ok, totals.fallbacks), (1, 1, 1));
    assert_eq!(
        (
            totals.prompt_tokens,
            totals.completion_tokens,
            totals.total_tokens
        ),
        (10, 5, 15)
    );

    let by_model = stats::by_model(&f.state.db, stats::Range::Day)
        .await
        .unwrap();
    let broken = by_model.iter().find(|m| m.model_name == "broken").unwrap();
    let good = by_model.iter().find(|m| m.model_name == "good").unwrap();
    assert_eq!((broken.attempts, broken.failed, broken.answered), (1, 1, 0));
    assert_eq!(
        (good.attempts, good.answered, good.total_tokens),
        (1, 1, 15)
    );
}

#[actix_web::test]
async fn streaming_passes_through_and_counts_tokens() {
    let f = fixture("stream").await;
    let app = test::init_service(App::new().configure(configure(f.state.clone()))).await;

    let req = test::TestRequest::post()
        .uri("/v1/chat/completions")
        .insert_header(("Authorization", format!("Bearer {}", f.key)))
        .set_json(json!({"model": "priority", "stream": true, "messages": []}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let body = String::from_utf8(test::read_body(resp).await.to_vec()).unwrap();
    assert!(body.contains("\"Hel\"") && body.ends_with("data: [DONE]\n\n"));

    settle().await;
    let totals = stats::totals(&f.state.db, stats::Range::Day).await.unwrap();
    assert_eq!(
        (
            totals.requests,
            totals.ok,
            totals.fallbacks,
            totals.total_tokens
        ),
        (1, 1, 1, 10)
    );
}

#[actix_web::test]
async fn all_failed_unknown_model_and_bad_key() {
    let f = fixture("errors").await;
    let app = test::init_service(App::new().configure(configure(f.state.clone()))).await;
    let call = |model: &str, key: &str| {
        test::TestRequest::post()
            .uri("/v1/chat/completions")
            .insert_header(("Authorization", format!("Bearer {key}")))
            .set_json(json!({"model": model, "messages": []}))
            .to_request()
    };

    let resp = test::call_service(&app, call("doomed", &f.key)).await;
    assert_eq!(resp.status(), 502);
    let body: Value = test::read_body_json(resp).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("mock/broken")
    );

    let resp = test::call_service(&app, call("nope", &f.key)).await;
    assert_eq!(resp.status(), 404);

    let resp = test::call_service(&app, call("priority", "agw_wrong")).await;
    assert_eq!(resp.status(), 401);

    // Direct model name and provider/model both resolve without a combo.
    let resp = test::call_service(&app, call("good", &f.key)).await;
    assert_eq!(resp.status(), 200);
    let resp = test::call_service(&app, call("mock/good", &f.key)).await;
    assert_eq!(resp.status(), 200);

    settle().await;
    let totals = stats::totals(&f.state.db, stats::Range::Day).await.unwrap();
    // 404 and 401 never reach an upstream and are not logged.
    assert_eq!((totals.requests, totals.ok, totals.failed), (3, 2, 1));
}

#[actix_web::test]
async fn models_endpoint_lists_combos_and_models() {
    let f = fixture("models").await;
    let app = test::init_service(App::new().configure(configure(f.state.clone()))).await;
    let req = test::TestRequest::get()
        .uri("/v1/models")
        .insert_header(("Authorization", format!("Bearer {}", f.key)))
        .to_request();
    let body: Value = test::call_and_read_body_json(&app, req).await;
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["doomed", "priority", "broken", "good"]);
}

#[actix_web::test]
async fn admin_pages_require_login_and_render() {
    let f = fixture("admin").await;
    let app = test::init_service(App::new().configure(configure(f.state.clone()))).await;

    let resp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
    assert_eq!(resp.status(), 303);
    assert_eq!(resp.headers().get("location").unwrap(), "/login");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/login")
            .set_form([("password", "password123")])
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 303);
    let cookie = resp
        .response()
        .cookies()
        .find(|c| c.name() == "gateway_session")
        .unwrap()
        .into_owned();

    for page in [
        "/",
        "/?range=7d",
        "/?range=30d",
        "/requests",
        "/providers",
        "/providers/1",
        "/combos",
        "/combos/1",
        "/keys",
    ] {
        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(page)
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200, "{page}");
    }
}

#[actix_web::test]
async fn test_buttons_report_per_model_and_combo_results() {
    let f = fixture("probe").await;
    let app = test::init_service(App::new().configure(configure(f.state.clone()))).await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/login")
            .set_form([("password", "password123")])
            .to_request(),
    )
    .await;
    let mut cookie = resp
        .response()
        .cookies()
        .find(|c| c.name() == "gateway_session")
        .unwrap()
        .into_owned();

    // POST, follow the redirect with the refreshed session cookie, return the page HTML.
    let mut post_then_get = async |post: &str, get: &str| {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri(post)
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 303, "{post}");
        if let Some(c) = resp
            .response()
            .cookies()
            .find(|c| c.name() == "gateway_session")
        {
            cookie = c.into_owned();
        }
        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(get)
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        String::from_utf8(test::read_body(resp).await.to_vec()).unwrap()
    };

    // Model ids from the fixture: 1 = broken, 2 = good.
    let html = post_then_get("/providers/1/test", "/providers/1").await;
    assert!(html.contains("1 of 2 model(s) answered."), "{html}");
    assert!(html.contains("Works · ") && html.contains("Hello"));
    assert!(html.contains("Failed · 500 · boom"));

    let html = post_then_get("/models/2/test", "/providers/1").await;
    assert!(html.contains("good answered in"));

    let html = post_then_get("/combos/1/test", "/combos/1").await;
    assert!(
        html.contains("works via fallback: mock / good answered on attempt 2"),
        "{html}"
    );

    let html = post_then_get("/combos/2/test", "/combos/2").await;
    assert!(html.contains("fails: all 1 model(s) failed"), "{html}");

    // Tests are not traffic.
    settle().await;
    assert_eq!(
        stats::totals(&f.state.db, stats::Range::Day)
            .await
            .unwrap()
            .requests,
        0
    );
}
