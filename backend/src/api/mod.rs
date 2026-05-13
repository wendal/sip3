use anyhow::Result;
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
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

/// Middleware: if `auth.api_key` is configured, require `X-Api-Key: <value>` on
/// every request. The `/api/health` route bypasses this check (see route setup).
async fn require_api_key(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if let Some(api_key) = &state.config.auth.api_key {
        let provided = req.headers().get("x-api-key").and_then(|v| v.to_str().ok());
        if provided != Some(api_key.as_str()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(next.run(req).await)
}

pub async fn run(cfg: Config, pool: MySqlPool) -> Result<()> {
    // Configure CORS: if `server.allowed_origins` is set (comma-separated), restrict
    // to those origins; otherwise allow any origin (suitable for development).
    let cors = {
        let origins: Vec<&str> = cfg
            .server
            .allowed_origins
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        if origins.is_empty() {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        } else {
            let header_values: Vec<axum::http::HeaderValue> =
                origins.iter().filter_map(|o| o.parse().ok()).collect();
            CorsLayer::new()
                .allow_origin(header_values)
                .allow_methods(Any)
                .allow_headers(Any)
        }
    };

    let state = AppState {
        pool,
        config: cfg.clone(),
    };

    // Protected routes require an API key (if configured).
    let protected = Router::new()
        .route("/api/accounts", get(accounts::list).post(accounts::create))
        .route(
            "/api/accounts/:id",
            put(accounts::update).delete(accounts::delete_account),
        )
        .route("/api/registrations", get(status::list_registrations))
        .route(
            "/api/registrations/:id",
            axum::routing::delete(status::delete_registration),
        )
        .route("/api/calls", get(status::list_calls))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    let app = Router::new()
        .route("/api/health", get(health))
        .merge(protected)
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
