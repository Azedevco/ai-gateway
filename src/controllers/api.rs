//! Client API: OpenAI-compatible chat endpoints and the TypeSafe System One endpoint.

use actix_web::{HttpRequest, HttpResponse, get, web};
use serde_json::{Value, json};

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use crate::models::api_key;
use crate::models::kind::Kind;
use crate::services::router::{self, Resolved};
use crate::services::upstream::{DECISION_ENDPOINT, ProxyRequest};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(health)
        .service(list_models)
        .service(proxy("chat/completions", Kind::Chat))
        .service(proxy("completions", Kind::Chat))
        .service(proxy("embeddings", Kind::Chat))
        .service(proxy(DECISION_ENDPOINT, Kind::Decision));
}

fn proxy(endpoint: &'static str, kind: Kind) -> actix_web::Resource {
    web::resource(format!("/v1/{endpoint}")).route(web::post().to(
        move |state: web::Data<AppState>, req: HttpRequest, body: web::Bytes| async move {
            forward(state, req, body, endpoint, kind).await
        },
    ))
}

#[get("/health")]
async fn health(state: web::Data<AppState>) -> HttpResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => HttpResponse::Ok().json(json!({ "status": "ok" })),
        Err(_) => {
            HttpResponse::ServiceUnavailable().json(json!({ "status": "database unavailable" }))
        }
    }
}

#[get("/v1/models")]
async fn list_models(state: web::Data<AppState>, req: HttpRequest) -> ApiResult<HttpResponse> {
    authenticate(&state, &req).await?;
    let data: Vec<Value> = router::public_model_ids(&state.db)
        .await?
        .into_iter()
        .map(|(id, owner)| json!({ "id": id, "object": "model", "created": 0, "owned_by": owner }))
        .collect();
    Ok(HttpResponse::Ok().json(json!({ "object": "list", "data": data })))
}

async fn forward(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Bytes,
    endpoint: &str,
    kind: Kind,
) -> ApiResult<HttpResponse> {
    let api_key_id = authenticate(&state, &req).await?;

    let body: Value = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("request body is not valid JSON: {e}")))?;
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| ApiError::BadRequest("`model` is required".into()))?
        .to_string();

    let targets = match router::resolve(&state.db, &model, kind).await? {
        Resolved::Targets(t) => t,
        Resolved::EmptyCombo => {
            return Err(ApiError::NotFound(format!(
                "combo `{model}` has no enabled models"
            )));
        }
        Resolved::NotFound => {
            return Err(ApiError::NotFound(format!(
                "model `{model}` does not exist"
            )));
        }
    };

    let mut body = body;
    if kind == Kind::Decision
        && let Some(obj) = body.as_object_mut()
    {
        obj.remove("stream");
    }
    let request = ProxyRequest {
        endpoint: endpoint.to_string(),
        body,
        api_key_id: Some(api_key_id),
    };
    state.upstream.proxy(&state.db, request, targets).await
}

async fn authenticate(state: &AppState, req: &HttpRequest) -> ApiResult<i64> {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| req.headers().get("x-api-key").and_then(|v| v.to_str().ok()))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            ApiError::Unauthorized("missing API key (Authorization: Bearer agw_...)".into())
        })?;
    api_key::verify(&state.db, token)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("invalid or disabled API key".into()))
}
