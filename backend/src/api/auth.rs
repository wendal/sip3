use axum::{extract::State, http::StatusCode, Json};
use bcrypt::verify;
use serde_json::{json, Value};

use super::{jwt, AppState};
use crate::models::AdminUser;

#[derive(Debug, serde::Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// `POST /api/auth/login` — public endpoint.
/// Verifies admin credentials and returns a signed JWT.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "username and password are required".to_string(),
        ));
    }

    let user: Option<AdminUser> =
        sqlx::query_as("SELECT id, username, password_hash, created_at, updated_at FROM admin_users WHERE username = ?")
            .bind(&body.username)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user = match user {
        Some(u) => u,
        None => return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string())),
    };

    let valid = verify(&body.password, &user.password_hash)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !valid {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()));
    }

    let token = jwt::issue_token(&user.username, &state.jwt_secret, state.config.auth.jwt_expiry_secs)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "token": token,
        "username": user.username,
        "expires_in": state.config.auth.jwt_expiry_secs,
    })))
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
        return Err((StatusCode::UNAUTHORIZED, "Current password is incorrect".to_string()));
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
pub async fn me(
    axum::Extension(claims): axum::Extension<jwt::Claims>,
) -> Json<Value> {
    Json(json!({ "username": claims.sub }))
}
