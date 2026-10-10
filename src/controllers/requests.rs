use actix_web::{HttpResponse, get, web};
use askama::Template;
use serde::Deserialize;

use super::Admin;
use crate::AppState;
use crate::error::WebResult;
use crate::models::request_log::{self, AttemptRow, Filter, RequestRow};
use crate::views::{self, Nav, Page};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(index);
}

const PAGE_SIZE: i64 = 50;

/// A request with its upstream attempts.
pub struct Item {
    pub request: RequestRow,
    pub attempts: Vec<AttemptRow>,
}

#[derive(Template)]
#[template(path = "requests.html")]
struct RequestsView {
    page: Page,
    items: Vec<Item>,
    status: String,
    model: String,
    page_no: i64,
    has_next: bool,
}

impl RequestsView {
    fn page_link(&self, page_no: &i64) -> String {
        let q = serde_urlencoded::to_string([
            ("status", self.status.as_str()),
            ("model", self.model.as_str()),
        ])
        .unwrap_or_default();
        format!("/requests?{q}&page={page_no}")
    }
}

#[derive(Deserialize)]
struct Query {
    #[serde(default)]
    status: String,
    #[serde(default)]
    model: String,
    page: Option<i64>,
}

#[get("/requests")]
async fn index(
    admin: Admin,
    state: web::Data<AppState>,
    q: web::Query<Query>,
) -> WebResult<HttpResponse> {
    let q = q.into_inner();
    let page_no = q.page.unwrap_or(1).max(1);
    let filter = Filter {
        status: Some(q.status.as_str()).filter(|s| *s == "ok" || *s == "error"),
        model: Some(q.model.trim()).filter(|m| !m.is_empty()),
    };
    // Fetch one extra row to know whether there is a next page.
    let mut rows =
        request_log::list(&state.db, &filter, PAGE_SIZE + 1, (page_no - 1) * PAGE_SIZE).await?;
    let has_next = rows.len() as i64 > PAGE_SIZE;
    rows.truncate(PAGE_SIZE as usize);

    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut attempts = request_log::attempts_for(&state.db, &ids).await?;
    let items = rows
        .into_iter()
        .map(|request| {
            let mine = attempts
                .extract_if(.., |(rid, _)| *rid == request.id)
                .map(|(_, a)| a)
                .collect();
            Item {
                request,
                attempts: mine,
            }
        })
        .collect();

    views::render(RequestsView {
        page: Page::new(&admin.0, "Requests", Nav::Requests),
        items,
        status: q.status,
        model: q.model,
        page_no,
        has_next,
    })
}
