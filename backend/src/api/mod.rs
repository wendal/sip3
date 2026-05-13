use anyhow::Result;
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, post, put},
    Router,
};
use sqlx::MySqlPool;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::config::Config;

pub mod accounts;
pub mod admin_users;
pub mod auth;
pub mod jwt;
pub mod stats;
pub mod status;

/// Combined application state passed to all handlers
#[derive(Clone)]
pub struct AppState {
    pub pool: MySqlPool,
    pub config: Config,
    /// Effective JWT signing secret (may be randomly generated at startup if not configured).
    pub jwt_secret: String,
}

/// Extract a Bearer token from the `Authorization` header.
fn extract_bearer_token(req: &Request) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Middleware: require a valid JWT Bearer token.
/// On success, injects `jwt::Claims` into request extensions so handlers can access it.
async fn require_jwt(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    let claims =
        jwt::verify_token(&token, &state.jwt_secret).map_err(|_| StatusCode::UNAUTHORIZED)?;
    req.extensions_mut().insert(claims);
    Ok(next.run(req).await)
}

/// Middleware: accept either a valid JWT Bearer token **or** a matching `X-Api-Key`.
///
/// - If a Bearer token is provided and valid, injects `jwt::Claims` into extensions.
/// - If `X-Api-Key` is configured and matches, passes through (no claims injected).
/// - Otherwise returns 401.
async fn require_auth(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Try Bearer JWT first.
    if let Some(token) = extract_bearer_token(&req) {
        if let Ok(claims) = jwt::verify_token(&token, &state.jwt_secret) {
            req.extensions_mut().insert(claims);
            return Ok(next.run(req).await);
        }
    }

    // Try X-Api-Key.
    if let Some(api_key) = &state.config.auth.api_key {
        let provided = req.headers().get("x-api-key").and_then(|v| v.to_str().ok());
        if provided == Some(api_key.as_str()) {
            return Ok(next.run(req).await);
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

pub async fn run(cfg: Config, pool: MySqlPool) -> Result<()> {
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

    // Resolve the JWT secret: use configured value or generate a random one at startup.
    let jwt_secret = if cfg.auth.jwt_secret.is_empty() {
        use rand::Rng;
        let bytes: Vec<u8> = rand::thread_rng().gen::<[u8; 32]>().to_vec();
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    } else {
        cfg.auth.jwt_secret.clone()
    };

    let state = AppState {
        pool,
        config: cfg.clone(),
        jwt_secret,
    };

    // JWT-only routes: caller must present a valid Bearer token; claims are injected.
    let jwt_routes = Router::new()
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/change-password", post(auth::change_password))
        .layer(middleware::from_fn_with_state(state.clone(), require_jwt));

    // Dual-auth routes: accept Bearer JWT or X-Api-Key.
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
        .route("/api/stats", get(stats::get_stats))
        .route(
            "/api/admin/users",
            get(admin_users::list).post(admin_users::create),
        )
        .route(
            "/api/admin/users/:id",
            put(admin_users::update).delete(admin_users::delete_user),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ));

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login", post(auth::login))
        .merge(jwt_routes)
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
