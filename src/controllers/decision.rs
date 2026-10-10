//! Decision models: providers and combos that speak TypeSafe System One (Jev-style).

use actix_web::{HttpResponse, get, web};
use askama::Template;

use super::Admin;
use crate::AppState;
use crate::error::WebResult;
use crate::models::combo::{self, ComboRow};
use crate::models::kind::Kind;
use crate::models::provider::{self, ProviderRow};
use crate::views::{self, Nav, Page};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(index);
}

#[derive(Template)]
#[template(path = "decision.html")]
struct DecisionView {
    page: Page,
    providers: Vec<ProviderRow>,
    combos: Vec<ComboRow>,
}

#[get("/decision")]
async fn index(admin: Admin, state: web::Data<AppState>) -> WebResult<HttpResponse> {
    let (providers, combos) = tokio::try_join!(
        provider::list(&state.db, Kind::Decision),
        combo::list(&state.db, Kind::Decision),
    )?;
    views::render(DecisionView {
        page: Page::new(&admin.0, "Decision models", Nav::Decision),
        providers,
        combos,
    })
}
