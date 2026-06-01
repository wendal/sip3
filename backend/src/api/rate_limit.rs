use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<RwLock<RateLimiterState>>,
    limit: usize,
    window_secs: u64,
}

struct RateLimiterState {
    requests: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    pub fn new(limit: usize, window_secs: u64) -> Self {
        Self {
            state: Arc::new(RwLock::new(RateLimiterState {
                requests: HashMap::new(),
            })),
            limit,
            window_secs,
        }
    }

    pub async fn check(&self, key: &str) -> bool {
        let mut state = self.state.write().await;
        let now = Instant::now();
        let window = Duration::from_secs(self.window_secs);

        let timestamps = state.requests.entry(key.to_string()).or_insert_with(Vec::new);

        timestamps.retain(|t| now.duration_since(*t) < window);

        if timestamps.len() >= self.limit {
            return false;
        }

        timestamps.push(now);
        true
    }
}

pub async fn rate_limit_middleware(
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let client_ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let rate_limiter = req
        .extensions()
        .get::<super::AppState>()
        .and_then(|s| s.rate_limiter.clone());

    if let Some(limiter) = rate_limiter && !limiter.check(&client_ip).await {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(req).await)
}
