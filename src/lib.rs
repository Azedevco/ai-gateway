//! AI Gateway: one OpenAI-compatible endpoint in front of many providers, with priority
//! fallback and usage statistics.
//!
//! Layout (MVC): `models` talk to SQLite, `controllers` handle HTTP, `templates/` + `views`
//! render the admin UI, and `services` hold routing and upstream calls.

pub mod config;
pub mod controllers;
pub mod db;
pub mod error;
pub mod models;
pub mod services;
pub mod views;

use actix_session::{SessionMiddleware, config::PersistentSession, storage::CookieSessionStore};
use actix_web::{cookie::Key, web};

use crate::config::Config;
use crate::services::probes::Probes;
use crate::services::upstream::Upstream;

/// Shared application state.
pub struct AppState {
    pub db: sqlx::SqlitePool,
    pub config: Config,
    pub upstream: Upstream,
    /// Latest admin test result per model.
    pub probes: Probes,
}

/// Registers every route. The admin UI is wrapped in the session middleware; the API is not.
pub fn configure(state: web::Data<AppState>) -> impl FnOnce(&mut web::ServiceConfig) {
    move |cfg| {
        let session = SessionMiddleware::builder(
            CookieSessionStore::default(),
            Key::from(&state.config.session_secret),
        )
        .cookie_name("gateway_session".into())
        .cookie_secure(state.config.secure_cookies)
        .session_lifecycle(
            PersistentSession::default().session_ttl(actix_web::cookie::time::Duration::days(7)),
        )
        .build();

        cfg.app_data(state)
            // Chat requests carry whole conversations.
            .app_data(web::JsonConfig::default().limit(32 * 1024 * 1024))
            .app_data(web::PayloadConfig::new(32 * 1024 * 1024))
            .configure(controllers::api::routes)
            .service(
                web::scope("")
                    .wrap(session)
                    .configure(controllers::web_routes),
            );
    }
}
