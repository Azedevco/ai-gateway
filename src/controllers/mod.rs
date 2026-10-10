//! HTTP handlers. `api` serves clients; the rest is the admin UI.

pub mod api;
mod auth;
mod combos;
mod dashboard;
mod decision;
mod keys;
mod providers;
mod requests;

use std::future::{Ready, ready};

use actix_session::{Session, SessionExt};
use actix_web::{FromRequest, HttpRequest, HttpResponse, dev::Payload, get, web};

use crate::views;

pub fn web_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(stylesheet)
        .configure(auth::routes)
        .configure(dashboard::routes)
        .configure(requests::routes)
        .configure(providers::routes)
        .configure(combos::routes)
        .configure(decision::routes)
        .configure(keys::routes);
}

#[get("/static/app.css")]
async fn stylesheet() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/css; charset=utf-8")
        .insert_header(("Cache-Control", "public, max-age=3600"))
        .body(include_str!("../../static/app.css"))
}

const SESSION_ADMIN: &str = "admin";

/// Extractor that only succeeds for a logged-in admin; otherwise redirects to the login page.
pub struct Admin(pub Session);

impl FromRequest for Admin {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let session = req.get_session();
        if session.get::<bool>(SESSION_ADMIN).ok().flatten() == Some(true) {
            return ready(Ok(Admin(session)));
        }
        let response = views::redirect("/login");
        ready(Err(actix_web::error::InternalError::from_response(
            "login required",
            response,
        )
        .into()))
    }
}
