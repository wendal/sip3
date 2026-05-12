use anyhow::Result;
use axum::{
    routing::{get, put},
    Router,
};
use sqlx::MySqlPool;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::config::Config;

pub mod accounts;
pub mod status;

/// Combined application state passed to all handlers
#[derive(Clone)]
pub struct AppState {
    pub pool: MySqlPool,
    pub config: Config,
}

pub async fn run(cfg: Config, pool: MySqlPool) -> Result<()> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let state = AppState {
        pool,
        config: cfg.clone(),
    };

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/accounts", get(accounts::list).post(accounts::create))
        .route(
            "/api/accounts/:id",
            put(accounts::update).delete(accounts::delete_account),
        )
        .route("/api/registrations", get(status::list_registrations))
        .route("/api/calls", get(status::list_calls))
        .layer(cors)
        .with_state(state);

    let addr = format!("{}:{}", cfg.server.api_host, cfg.server.api_port);
    info!("API server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> &'static str {
    "OK"
}
