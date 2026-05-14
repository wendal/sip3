use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use super::AppState;
use crate::acl::parse_cidr;
use crate::models::{AclEntry, CreateAclEntry, UpdateAclEntry};

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let entries: Vec<AclEntry> = sqlx::query_as(
        "SELECT id, action, cidr, description, priority, enabled, created_at
         FROM sip_acl ORDER BY priority ASC, id ASC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": entries })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateAclEntry>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.action != "allow" && body.action != "deny" {
        return Err((
            StatusCode::BAD_REQUEST,
            "action must be 'allow' or 'deny'".to_string(),
        ));
    }
    let canonical_cidr = parse_cidr(&body.cidr).map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let priority = body.priority.unwrap_or(100);
    let enabled = body.enabled.unwrap_or(1);

    let result = sqlx::query(
        "INSERT INTO sip_acl (action, cidr, description, priority, enabled) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&body.action)
    .bind(&canonical_cidr)
    .bind(&body.description)
    .bind(priority)
    .bind(enabled)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(
        json!({ "id": result.last_insert_id(), "message": "ACL rule created" }),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<u32>,
    Json(body): Json<UpdateAclEntry>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if let Some(action) = &body.action {
        if action != "allow" && action != "deny" {
            return Err((
                StatusCode::BAD_REQUEST,
                "action must be 'allow' or 'deny'".to_string(),
            ));
        }
    }
    let canonical_cidr = if let Some(cidr) = &body.cidr {
        Some(parse_cidr(cidr).map_err(|e| (StatusCode::BAD_REQUEST, e))?)
    } else {
        None
    };

    sqlx::query(
        "UPDATE sip_acl SET
            action      = COALESCE(?, action),
            cidr        = COALESCE(?, cidr),
            description = COALESCE(?, description),
            priority    = COALESCE(?, priority),
            enabled     = COALESCE(?, enabled)
         WHERE id = ?",
    )
    .bind(&body.action)
    .bind(&canonical_cidr)
    .bind(&body.description)
    .bind(body.priority)
    .bind(body.enabled)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "message": "ACL rule updated" })))
}

pub async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<Json<Value>, (StatusCode, String)> {
    sqlx::query("DELETE FROM sip_acl WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "message": "ACL rule deleted" })))
}
