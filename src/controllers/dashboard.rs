use actix_web::{HttpResponse, get, web};
use askama::Template;
use serde::Deserialize;

use super::Admin;
use crate::AppState;
use crate::error::WebResult;
use crate::models::stats::{self, KeyStat, ModelStat, Range, RequestedStat, Totals};
use crate::views::{self, Chart, Nav, Page};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(index);
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardView {
    page: Page,
    range: Range,
    ranges: [Range; 3],
    totals: Totals,
    chart: Chart,
    models: Vec<ModelStat>,
    total_answered: i64,
    requested: Vec<RequestedStat>,
    keys: Vec<KeyStat>,
}

#[derive(Deserialize)]
struct Query {
    #[serde(default)]
    range: Range,
}

#[get("/")]
async fn index(
    admin: Admin,
    state: web::Data<AppState>,
    q: web::Query<Query>,
) -> WebResult<HttpResponse> {
    let range = q.range;
    let db = &state.db;
    let (totals, buckets, models, requested, keys) = tokio::try_join!(
        stats::totals(db, range),
        stats::timeline(db, range),
        stats::by_model(db, range),
        stats::by_requested(db, range),
        stats::by_key(db, range),
    )?;
    let total_answered = models.iter().map(|m| m.answered).sum();
    views::render(DashboardView {
        page: Page::new(&admin.0, "Dashboard", Nav::Dashboard),
        range,
        ranges: Range::ALL,
        totals,
        chart: Chart::build(&buckets, range == Range::Day),
        models,
        total_answered,
        requested,
        keys,
    })
}
