use anyhow::Result;
use arc_swap::ArcSwap;
use axum::{
    Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, post, put},
};
use sqlx::MySqlPool;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::api::webhook_dispatcher::WebhookDispatcher;
use crate::config::Config;
use crate::config_watch::ConfigWatcher;
use crate::security_guard::{GuardLimits, SecurityGuard};

pub mod accounts;
pub mod acl;
pub mod admin_users;
pub mod auth;
pub mod conferences;
pub mod email_worker;
pub mod jwt;
pub mod messages;
pub mod metrics;
pub mod openapi;
pub mod rate_limit;
pub mod security;
pub mod stats;
pub mod status;
pub mod turn;
pub mod voicemail;
pub mod webhook_dispatcher;
pub mod webhooks;

/// Combined application state passed to all handlers
#[derive(Clone)]
pub struct AppState {
    pub pool: MySqlPool,
    /// Hot-reloadable config. Read with `state.config.load().server.sip_domain`.
    pub config: Arc<ArcSwap<Config>>,
    /// Effective JWT signing secret (may be randomly generated at startup if not configured).
    pub jwt_secret: String,
    pub auth_guard: Arc<Mutex<SecurityGuard>>,
    pub rate_limiter: Option<rate_limit::RateLimiter>,
    /// Watches the config and broadcasts reloads to the rest of the system.
    pub config_watcher: Arc<ConfigWatcher>,
    /// Webhook outbox dispatcher; enqueue from anywhere, drain from background.
    pub webhook_dispatcher: Arc<WebhookDispatcher>,
    /// Background SMTP outbox worker.
    pub email_worker: Arc<crate::api::email_worker::EmailWorker>,
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
    if let Some(token) = extract_bearer_token(&req)
        && let Ok(claims) = jwt::verify_token(&token, &state.jwt_secret)
    {
        req.extensions_mut().insert(claims);
        return Ok(next.run(req).await);
    }

    // Try X-Api-Key.
    if let Some(api_key) = &state.config.load().auth.api_key {
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
        let bytes: Vec<u8> = rand::rng().random::<[u8; 32]>().to_vec();
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    } else {
        cfg.auth.jwt_secret.clone()
    };

    let rate_limiter = if cfg.security.rate_limit_requests > 0 {
        Some(rate_limit::RateLimiter::new(
            cfg.security.rate_limit_requests,
            cfg.security.rate_limit_window_secs,
        ))
    } else {
        None
    };

    let config_arc = Arc::new(ArcSwap::from_pointee(cfg.clone()));
    let config_watcher = Arc::new(ConfigWatcher::new(
        config_arc.clone(),
        cfg.cleanup.acl_refresh_interval_secs,
    ));

    let state = AppState {
        pool: pool.clone(),
        config: config_arc.clone(),
        jwt_secret,
        auth_guard: Arc::new(Mutex::new(SecurityGuard::new(GuardLimits {
            window_secs: cfg.security.window_secs,
            ip_fail_threshold: cfg.security.api_ip_fail_threshold as usize,
            user_ip_fail_threshold: cfg.security.api_user_ip_fail_threshold as usize,
            block_secs: cfg.security.block_secs,
        }))),
        rate_limiter,
        config_watcher: config_watcher.clone(),
        webhook_dispatcher: Arc::new(WebhookDispatcher::new(pool.clone())),
        email_worker: Arc::new(crate::api::email_worker::EmailWorker::new(
            pool.clone(),
            config_arc.clone(),
        )),
    };

    // Spawn the periodic config reload task. It reloads from
    // `SIP3__*` env vars and `config.toml` (if present) and atomically
    // replaces the live config.
    config_watcher.spawn_periodic_reload();

    // Spawn the webhook outbox drainer.
    state.webhook_dispatcher.clone().spawn_worker();

    // JWT-only routes: caller must present a valid Bearer token; claims are injected.
    let jwt_routes = Router::new()
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/change-password", post(auth::change_password))
        .route("/api/admin/reload", post(admin_reload))
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
        .route(
            "/api/calls",
            get(status::list_calls).post(status::export_calls),
        )
        .route("/api/calls/cleanup", post(status::cleanup_calls))
        .route("/api/messages", get(messages::list_messages))
        .route("/api/stats", get(stats::get_stats))
        .route("/api/security/events", get(security::list_events))
        .route("/api/security/blocks", get(security::list_blocks))
        .route("/api/security/blocks/unblock", post(security::unblock))
        .route("/api/security/summary", get(security::summary))
        .route("/api/security/runtime", get(security::runtime_snapshot))
        .route("/api/acl", get(acl::list).post(acl::create))
        .route("/api/acl/:id", put(acl::update).delete(acl::delete_rule))
        .route(
            "/api/admin/users",
            get(admin_users::list).post(admin_users::create),
        )
        .route(
            "/api/admin/users/:id",
            put(admin_users::update).delete(admin_users::delete_user),
        )
        .route(
            "/api/conferences",
            get(conferences::list).post(conferences::create),
        )
        .route(
            "/api/conferences/:id",
            put(conferences::update).delete(conferences::delete_room),
        )
        .route(
            "/api/conferences/:id/participants",
            get(conferences::list_participants),
        )
        .route(
            "/api/voicemail/boxes",
            get(voicemail::list_boxes).post(voicemail::create_box),
        )
        .route("/api/voicemail/boxes/:id", put(voicemail::update_box))
        .route(
            "/api/voicemail/boxes/:id/greeting",
            post(voicemail::upload_greeting)
                .get(voicemail::download_greeting)
                .delete(voicemail::delete_greeting),
        )
        .route("/api/voicemail/messages", get(voicemail::list_messages))
        .route(
            "/api/voicemail/messages/:id",
            put(voicemail::update_message).delete(voicemail::delete_message),
        )
        .route(
            "/api/voicemail/messages/:id/download",
            get(voicemail::download_message),
        )
        .route("/api/webhooks", get(webhooks::list).post(webhooks::create))
        .route(
            "/api/webhooks/:id",
            put(webhooks::update).delete(webhooks::delete),
        )
        .route("/api/webhooks/deliveries", get(webhooks::list_deliveries))
        .route(
            "/api/webhooks/deliveries/:id/retry",
            post(webhooks::retry_delivery),
        )
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login", post(auth::login))
        .route("/api/turn/credentials", post(turn::credentials))
        .route("/api/turn/health", get(turn::health))
        .route("/api/messages/history", post(messages::history))
        .route("/api/metrics", get(metrics_handler))
        .route("/api/openapi.json", get(openapi::openapi_json))
        .merge(openapi::swagger_ui())
        .merge(jwt_routes)
        .merge(protected)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::rate_limit_middleware,
        ))
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

/// Reload the runtime config from `SIP3__*` env vars and the optional
/// `config.toml`. Replaces the live `Arc<ArcSwap<Config>>` atomically so
/// in-flight handlers see the new values on their next read.
async fn admin_reload(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, String)> {
    match state.config_watcher.reload_now().await {
        Ok(cfg) => {
            crate::api::metrics::inc_config_reload();
            info!("admin: config reloaded");
            Ok(axum::Json(serde_json::json!({
                "status": "reloaded",
                "sip_domain": cfg.server.sip_domain,
                "api_port": cfg.server.api_port,
                "public_ip": cfg.server.public_ip,
            })))
        }
        Err(e) => {
            tracing::warn!("admin: config reload failed: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

/// Prometheus exposition format. Public (no auth) so scrapers can hit it.
async fn metrics_handler() -> (
    axum::http::StatusCode,
    [(axum::http::HeaderName, &'static str); 1],
    Vec<u8>,
) {
    use axum::http::HeaderName;
    (
        axum::http::StatusCode::OK,
        [(
            HeaderName::from_static("content-type"),
            "text/plain; version=0.0.4",
        )],
        metrics::render(),
    )
}
