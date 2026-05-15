use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use bcrypt::verify;
use serde_json::{Value, json};
use std::net::IpAddr;
use tracing::warn;

use super::{AppState, jwt};
use crate::models::AdminUser;
use crate::security_guard::{AuthSurface, SecurityEventType, persist_security_event};

#[derive(Debug, serde::Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// `POST /api/auth/login` — public endpoint.
/// Verifies admin credentials and returns a signed JWT.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "username and password are required".to_string(),
        ));
    }

    let source_ip = extract_client_ip(&headers);
    if state.auth_guard.lock().await.is_blocked(&source_ip) {
        warn!("Blocked admin login attempt from {}", source_ip);
        if let Err(e) = persist_security_event(
            &state.pool,
            AuthSurface::ApiLogin,
            SecurityEventType::AuthFailed,
            &source_ip,
            Some(&body.username),
            "api login blocked by guard",
        )
        .await
        {
            warn!("Failed to persist security event: {}", e);
        }
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()));
    }

    let user: Option<AdminUser> =
        sqlx::query_as("SELECT id, username, password_hash, created_at, updated_at FROM admin_users WHERE username = ?")
            .bind(&body.username)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user = match user {
        Some(u) => u,
        None => {
            let blocked = state.auth_guard.lock().await.record_failure(
                AuthSurface::ApiLogin,
                &source_ip,
                Some(&body.username),
            );
            if let Err(e) = persist_security_event(
                &state.pool,
                AuthSurface::ApiLogin,
                SecurityEventType::AuthFailed,
                &source_ip,
                Some(&body.username),
                "api login failed: user not found",
            )
            .await
            {
                warn!("Failed to persist security event: {}", e);
            }
            if blocked && state.config.security.persist_acl_bans {
                if let Some(ip) = parse_ip(&source_ip)
                    && let Err(e) = persist_acl_ban(
                        &state.pool,
                        ip,
                        state.config.security.acl_ban_priority,
                        "auto-ban: api login brute force",
                    )
                    .await
                {
                    warn!(
                        "Failed to persist API login auto-ban ACL for {}: {}",
                        source_ip, e
                    );
                }
                if let Err(e) = persist_security_event(
                    &state.pool,
                    AuthSurface::ApiLogin,
                    SecurityEventType::IpBlocked,
                    &source_ip,
                    Some(&body.username),
                    "api login source blocked by threshold",
                )
                .await
                {
                    warn!("Failed to persist security event: {}", e);
                }
            }
            return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()));
        }
    };

    let valid = verify(&body.password, &user.password_hash)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !valid {
        let blocked = state.auth_guard.lock().await.record_failure(
            AuthSurface::ApiLogin,
            &source_ip,
            Some(&body.username),
        );
        if let Err(e) = persist_security_event(
            &state.pool,
            AuthSurface::ApiLogin,
            SecurityEventType::AuthFailed,
            &source_ip,
            Some(&body.username),
            "api login failed: bad password",
        )
        .await
        {
            warn!("Failed to persist security event: {}", e);
        }
        if blocked && state.config.security.persist_acl_bans {
            if let Some(ip) = parse_ip(&source_ip)
                && let Err(e) = persist_acl_ban(
                    &state.pool,
                    ip,
                    state.config.security.acl_ban_priority,
                    "auto-ban: api login brute force",
                )
                .await
            {
                warn!(
                    "Failed to persist API login auto-ban ACL for {}: {}",
                    source_ip, e
                );
            }
            if let Err(e) = persist_security_event(
                &state.pool,
                AuthSurface::ApiLogin,
                SecurityEventType::IpBlocked,
                &source_ip,
                Some(&body.username),
                "api login source blocked by threshold",
            )
            .await
            {
                warn!("Failed to persist security event: {}", e);
            }
        }
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()));
    }
    state
        .auth_guard
        .lock()
        .await
        .record_success(&source_ip, Some(&body.username));
    if let Err(e) = persist_security_event(
        &state.pool,
        AuthSurface::ApiLogin,
        SecurityEventType::AuthSucceeded,
        &source_ip,
        Some(&body.username),
        "api login succeeded",
    )
    .await
    {
        warn!("Failed to persist security event: {}", e);
    }

    let token = jwt::issue_token(
        &user.username,
        &state.jwt_secret,
        state.config.auth.jwt_expiry_secs,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "token": token,
        "username": user.username,
        "expires_in": state.config.auth.jwt_expiry_secs,
    })))
}

fn extract_client_ip(headers: &HeaderMap) -> String {
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first) = forwarded.split(',').map(str::trim).find(|s| !s.is_empty())
        && parse_ip(first).is_some()
    {
        return first.to_string();
    }
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok())
        && parse_ip(real_ip).is_some()
    {
        return real_ip.to_string();
    }
    "0.0.0.0".to_string()
}

fn parse_ip(raw: &str) -> Option<IpAddr> {
    raw.parse().ok()
}

async fn persist_acl_ban(
    pool: &sqlx::MySqlPool,
    ip: IpAddr,
    priority: i32,
    description: &str,
) -> anyhow::Result<()> {
    let cidr = match ip {
        IpAddr::V4(v4) => format!("{}/32", v4),
        IpAddr::V6(v6) => format!("{}/128", v6),
    };
    sqlx::query(
        "INSERT INTO sip_acl (action, cidr, description, priority, enabled)
         SELECT 'deny', ?, ?, ?, 1
         WHERE NOT EXISTS (
             SELECT 1 FROM sip_acl WHERE action = 'deny' AND cidr = ? AND enabled = 1
         )",
    )
    .bind(&cidr)
    .bind(description)
    .bind(priority)
    .bind(&cidr)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// `POST /api/auth/change-password` — JWT-protected.
/// Allows the currently logged-in admin to change their own password.
pub async fn change_password(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<jwt::Claims>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.new_password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            "new password must be at least 6 characters".to_string(),
        ));
    }

    let user: Option<AdminUser> =
        sqlx::query_as("SELECT id, username, password_hash, created_at, updated_at FROM admin_users WHERE username = ?")
            .bind(&claims.sub)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user = match user {
        Some(u) => u,
        None => return Err((StatusCode::NOT_FOUND, "Admin user not found".to_string())),
    };

    let valid = verify(&body.current_password, &user.password_hash)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !valid {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Current password is incorrect".to_string(),
        ));
    }

    let new_hash = bcrypt::hash(&body.new_password, bcrypt::DEFAULT_COST)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query("UPDATE admin_users SET password_hash = ? WHERE id = ?")
        .bind(&new_hash)
        .bind(user.id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "message": "Password changed successfully" })))
}

/// `GET /api/auth/me` — JWT-protected.
/// Returns basic info about the currently logged-in admin.
pub async fn me(axum::Extension(claims): axum::Extension<jwt::Claims>) -> Json<Value> {
    Json(json!({ "username": claims.sub }))
}
