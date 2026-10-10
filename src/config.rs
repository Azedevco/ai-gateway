use std::env;

use anyhow::{Context, Result, bail};

/// Runtime configuration, read from the environment (and `.env` if present).
#[derive(Clone, Debug)]
pub struct Config {
    /// Address the HTTP server binds to.
    pub bind: String,
    /// SQLite URL, e.g. `sqlite://data/gateway.db`.
    pub database_url: String,
    /// Password for the admin UI.
    pub admin_password: String,
    /// 64+ byte secret used to sign the admin session cookie.
    pub session_secret: Vec<u8>,
    /// Set the `Secure` flag on the session cookie (enable behind HTTPS).
    pub secure_cookies: bool,
    /// Request log rows older than this many days are deleted. 0 keeps everything.
    pub retention_days: i64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let admin_password = env::var("ADMIN_PASSWORD").context("ADMIN_PASSWORD is required")?;
        if admin_password.len() < 8 {
            bail!("ADMIN_PASSWORD must be at least 8 characters");
        }

        let session_secret = env::var("SESSION_SECRET").context("SESSION_SECRET is required")?;
        if session_secret.len() < 64 {
            bail!("SESSION_SECRET must be at least 64 characters (e.g. `openssl rand -hex 32`)");
        }

        Ok(Self {
            bind: env::var("GATEWAY_BIND").unwrap_or_else(|_| "127.0.0.1:8090".into()),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://data/gateway.db".into()),
            admin_password,
            session_secret: session_secret.into_bytes(),
            secure_cookies: env::var("SECURE_COOKIES").is_ok_and(|v| v == "1" || v == "true"),
            retention_days: env::var("RETENTION_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(90),
        })
    }
}
