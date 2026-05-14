use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use bcrypt::{hash, DEFAULT_COST};
use serde_json::{json, Value};

use super::AppState;
use crate::models::{AdminUser, CreateAdminUser, UpdateAdminUser};

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let users: Vec<AdminUser> = sqlx::query_as(
        "SELECT id, username, password_hash, created_at, updated_at FROM admin_users ORDER BY created_at ASC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": users })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateAdminUser>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "username and password are required".to_string(),
        ));
    }
    if body.password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            "password must be at least 6 characters".to_string(),
        ));
    }

    let password_hash = hash(&body.password, DEFAULT_COST)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result = sqlx::query("INSERT INTO admin_users (username, password_hash) VALUES (?, ?)")
        .bind(&body.username)
        .bind(&password_hash)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(db_err) = &e {
                if db_err.is_unique_violation() {
                    return (
                        StatusCode::CONFLICT,
                        "Admin user with this username already exists".to_string(),
                    );
                }
            }
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(
        json!({ "id": result.last_insert_id(), "message": "Admin user created" }),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(body): Json<UpdateAdminUser>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let password = match &body.password {
        Some(p) if !p.is_empty() => p,
        _ => return Err((StatusCode::BAD_REQUEST, "password is required".to_string())),
    };

    if password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            "password must be at least 6 characters".to_string(),
        ));
    }

    let password_hash = hash(password, DEFAULT_COST)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result = sqlx::query("UPDATE admin_users SET password_hash = ? WHERE id = ?")
        .bind(&password_hash)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Admin user not found".to_string()));
    }

    Ok(Json(json!({ "message": "Password updated" })))
}

pub async fn delete_user(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Prevent deletion if this is the last admin user.
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM admin_users")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if count.0 <= 1 {
        return Err((
            StatusCode::CONFLICT,
            "Cannot delete the last admin user".to_string(),
        ));
    }

    let result = sqlx::query("DELETE FROM admin_users WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Admin user not found".to_string()));
    }

    Ok(Json(json!({ "message": "Admin user deleted" })))
}
