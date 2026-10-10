//! Decision models: models that speak TypeSafe System One (Jev-style), from any provider.

use actix_web::{HttpResponse, get, post, web};
use askama::Template;

use super::Admin;
use super::providers::ModelsForm;
use crate::AppState;
use crate::error::WebResult;
use crate::models::combo::{self, ComboRow};
use crate::models::kind::Kind;
use crate::models::model::{self, ModelRow};
use crate::models::provider::{self, ProviderRow};
use crate::views::{self, Nav, Page, flash};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(index).service(add_models);
}

#[derive(Template)]
#[template(path = "decision.html")]
struct DecisionView {
    page: Page,
    models: Vec<ModelRow>,
    combos: Vec<ComboRow>,
    providers: Vec<ProviderRow>,
}

#[get("/decision")]
async fn index(admin: Admin, state: web::Data<AppState>) -> WebResult<HttpResponse> {
    let (models, combos, providers) = tokio::try_join!(
        model::list_by_kind(&state.db, Kind::Decision),
        combo::list(&state.db, Some(Kind::Decision)),
        provider::list(&state.db),
    )?;
    views::render(DecisionView {
        page: Page::new(&admin.0, "Decision models", Nav::Decision),
        models,
        combos,
        providers,
    })
}

#[derive(serde::Deserialize)]
struct AddForm {
    provider_id: i64,
    #[serde(flatten)]
    models: ModelsForm,
}

/// Adds decision models to an existing provider (one provider can serve chat and decision).
#[post("/decision/models")]
async fn add_models(
    admin: Admin,
    state: web::Data<AppState>,
    form: web::Form<AddForm>,
) -> WebResult<HttpResponse> {
    let added = model::add_many(
        &state.db,
        form.provider_id,
        &form.models.names(),
        Kind::Decision,
    )
    .await?;
    flash(&admin.0, true, format!("{added} decision model(s) added."));
    Ok(views::redirect("/decision"))
}
