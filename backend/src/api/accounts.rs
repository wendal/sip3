use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use bcrypt::{DEFAULT_COST, hash};
use serde_json::{Value, json};

use super::AppState;
use crate::models::{Account, CreateAccount, UpdateAccount};

pub const SIP_USERNAME_RULE_MESSAGE: &str = "SIP username must be a 3-6 digit extension";

pub fn validate_sip_username(username: &str) -> Result<(), &'static str> {
    if (3..=6).contains(&username.len()) && username.chars().all(|c| c.is_ascii_digit()) {
        Ok(())
    } else {
        Err(SIP_USERNAME_RULE_MESSAGE)
    }
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let accounts: Vec<Account> = sqlx::query_as(
        "SELECT a.id, a.username, a.password_hash, a.ha1_hash, a.display_name,
                a.domain, a.enabled, a.created_at, a.updated_at,
                (SELECT MAX(c.started_at) FROM sip_calls c
                 WHERE c.caller = CONCAT(a.username,'@',a.domain)
                    OR c.callee = CONCAT(a.username,'@',a.domain)) AS last_call_at,
                (SELECT COUNT(*) FROM sip_calls c
                 WHERE c.caller = CONCAT(a.username,'@',a.domain)
                    OR c.callee = CONCAT(a.username,'@',a.domain)) AS call_count
         FROM sip_accounts a ORDER BY a.created_at DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": accounts })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateAccount>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "username and password are required".to_string(),
        ));
    }
    validate_sip_username(&body.username)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.to_string()))?;

    let password_hash = hash(&body.password, DEFAULT_COST)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let realm = &state.config.auth.realm;
    let domain = body.domain.as_deref().unwrap_or(realm.as_str()).to_string();

    // Pre-compute HA1 for SIP Digest auth: MD5(username:realm:password)
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", body.username, realm, body.password).as_bytes())
    );

    let result = sqlx::query(
        "INSERT INTO sip_accounts (username, password_hash, ha1_hash, display_name, domain)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&body.username)
    .bind(&password_hash)
    .bind(&ha1)
    .bind(&body.display_name)
    .bind(&domain)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        // Use SQLx's structured error API instead of string-matching.
        if let sqlx::Error::Database(db_err) = &e
            && db_err.is_unique_violation()
        {
            return (
                StatusCode::CONFLICT,
                "Account already exists for this username and domain".to_string(),
            );
        }
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    Ok(Json(
        json!({ "id": result.last_insert_id(), "message": "Account created" }),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(body): Json<UpdateAccount>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Fetch the current username (needed to recompute HA1 if password changes).
    let row: Option<(String,)> = sqlx::query_as("SELECT username FROM sip_accounts WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (username,) = match row {
        Some(r) => r,
        None => return Err((StatusCode::NOT_FOUND, "Account not found".to_string())),
    };

    let realm = &state.config.auth.realm;

    // Compute updated hashes only if a new password was provided.
    let (new_password_hash, new_ha1) = if let Some(password) = &body.password {
        let hash = bcrypt::hash(password, DEFAULT_COST)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let ha1 = format!(
            "{:x}",
            md5::compute(format!("{}:{}:{}", username, realm, password).as_bytes())
        );
        (Some(hash), Some(ha1))
    } else {
        (None, None)
    };

    // Single atomic UPDATE using COALESCE so that only provided fields are changed.
    // NULL bindings leave the existing column value unchanged.
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query(
        "UPDATE sip_accounts SET
             password_hash = COALESCE(?, password_hash),
             ha1_hash      = COALESCE(?, ha1_hash),
             display_name  = COALESCE(?, display_name),
             domain        = COALESCE(?, domain),
             enabled       = COALESCE(?, enabled)
         WHERE id = ?",
    )
    .bind(new_password_hash)
    .bind(new_ha1)
    .bind(&body.display_name)
    .bind(&body.domain)
    .bind(body.enabled)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "message": "Account updated" })))
}

pub async fn delete_account(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let result = sqlx::query("DELETE FROM sip_accounts WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Account not found".to_string()));
    }

    Ok(Json(json!({ "message": "Account deleted" })))
}
