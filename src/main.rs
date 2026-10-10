use std::time::Duration;

use actix_web::{App, HttpServer, middleware, web};
use ai_gateway::config::Config;
use ai_gateway::services::upstream::Upstream;
use ai_gateway::{AppState, configure, db};
use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[actix_web::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url).await?;
    db::spawn_retention(pool.clone(), config.retention_days);

    let bind = config.bind.clone();
    let state = web::Data::new(AppState {
        db: pool,
        config,
        upstream: Upstream::new()?,
    });

    tracing::info!(%bind, "ai-gateway listening");
    HttpServer::new(move || {
        App::new()
            .wrap(
                middleware::Logger::new("%r %s %Dms")
                    .exclude("/health")
                    .exclude("/static/app.css"),
            )
            .configure(configure(state.clone()))
    })
    .keep_alive(Duration::from_secs(75))
    .bind(&bind)?
    .run()
    .await?;
    Ok(())
}
