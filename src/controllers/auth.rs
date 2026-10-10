use actix_session::Session;
use actix_web::{HttpResponse, get, post, web};
use askama::Template;
use serde::Deserialize;
use subtle::ConstantTimeEq;

use super::{Admin, SESSION_ADMIN};
use crate::AppState;
use crate::error::WebResult;
use crate::views;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(login_page).service(login).service(logout);
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginView {
    error: Option<&'static str>,
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

#[get("/login")]
async fn login_page() -> WebResult<HttpResponse> {
    views::render(LoginView { error: None })
}

#[post("/login")]
async fn login(
    state: web::Data<AppState>,
    session: Session,
    form: web::Form<LoginForm>,
) -> WebResult<HttpResponse> {
    let expected = state.config.admin_password.as_bytes();
    if bool::from(form.password.as_bytes().ct_eq(expected)) {
        session.renew();
        session
            .insert(SESSION_ADMIN, true)
            .map_err(|e| anyhow::anyhow!("session: {e}"))?;
        return Ok(views::redirect("/"));
    }
    // Slow down guessing.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    views::render(LoginView {
        error: Some("Wrong password."),
    })
}

#[post("/logout")]
async fn logout(admin: Admin) -> HttpResponse {
    admin.0.purge();
    views::redirect("/login")
}
