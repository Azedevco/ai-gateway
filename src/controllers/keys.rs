use actix_web::{HttpResponse, get, post, web};
use askama::Template;
use serde::Deserialize;

use super::Admin;
use crate::AppState;
use crate::error::WebResult;
use crate::models::api_key::{self, ApiKeyRow};
use crate::views::{self, Nav, Page, flash};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(index)
        .service(create)
        .service(toggle)
        .service(destroy);
}

const NEW_KEY: &str = "new_key";

#[derive(Template)]
#[template(path = "keys.html")]
struct KeysView {
    page: Page,
    keys: Vec<ApiKeyRow>,
    /// Plaintext of a key created on the previous request; shown once.
    new_key: Option<String>,
}

#[get("/keys")]
async fn index(admin: Admin, state: web::Data<AppState>) -> WebResult<HttpResponse> {
    let new_key = admin.0.remove_as::<String>(NEW_KEY).and_then(Result::ok);
    views::render(KeysView {
        page: Page::new(&admin.0, "API keys", Nav::Keys),
        keys: api_key::list(&state.db).await?,
        new_key,
    })
}

#[derive(Deserialize)]
struct KeyForm {
    name: String,
}

#[post("/keys")]
async fn create(
    admin: Admin,
    state: web::Data<AppState>,
    form: web::Form<KeyForm>,
) -> WebResult<HttpResponse> {
    if form.name.trim().is_empty() {
        flash(
            &admin.0,
            false,
            "Give the key a name, e.g. the app that will use it.",
        );
        return Ok(views::redirect("/keys"));
    }
    let key = api_key::create(&state.db, &form.name).await?;
    let _ = admin.0.insert(NEW_KEY, key);
    Ok(views::redirect("/keys"))
}

#[post("/keys/{id}/toggle")]
async fn toggle(
    _admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    api_key::toggle(&state.db, *id).await?;
    Ok(views::redirect("/keys"))
}

#[post("/keys/{id}/delete")]
async fn destroy(
    admin: Admin,
    state: web::Data<AppState>,
    id: web::Path<i64>,
) -> WebResult<HttpResponse> {
    api_key::delete(&state.db, *id).await?;
    flash(
        &admin.0,
        true,
        "Key deleted. Clients using it are rejected from now on.",
    );
    Ok(views::redirect("/keys"))
}
