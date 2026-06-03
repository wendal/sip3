//! OpenAPI 3.1 spec and Swagger UI handler.
//!
//! C2 ships a minimal spec so the `/api/docs` route is functional; per-handler
//! `#[utoipa::path]` annotations are added incrementally as handlers are
//! touched in later commits. The spec still surfaces the high-level API
//! shape, auth schemes, and tags so consumers can already get value from it.

use axum::Json;
use serde_json::{Value, json};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "SIP3 Admin & Telephony API",
        version = "1.8.0",
        description = "REST API for the SIP3 SIP proxy/registrar. See /api/docs for interactive Swagger UI."
    ),
    tags(
        (name = "accounts", description = "SIP account CRUD"),
        (name = "acl", description = "IP allow/deny rules"),
        (name = "conferences", description = "Conference rooms and participants"),
        (name = "voicemail", description = "Mailboxes and messages"),
        (name = "status", description = "Registrations and call detail records"),
        (name = "security", description = "Auth failures, auto-bans, runtime snapshot"),
        (name = "auth", description = "Admin login and password change"),
        (name = "turn", description = "TURN credential issuance and health"),
        (name = "messages", description = "SIP MESSAGE persistence and history"),
    )
)]
pub struct ApiDoc;

/// Return a hand-rolled OpenAPI 3.1 JSON document with the high-level route
/// list, auth schemes, and tags. This is the document fetched by the
/// Swagger UI bundle.
pub async fn openapi_json() -> Json<Value> {
    let mut doc = serde_json::to_value(ApiDoc::openapi()).unwrap_or_else(|_| json!({}));

    if let Some(obj) = doc.as_object_mut() {
        let components = obj
            .entry("components".to_string())
            .or_insert(json!({}))
            .as_object_mut()
            .expect("components object");
        components.insert(
            "securitySchemes".to_string(),
            json!({
                "bearer_jwt": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT"
                },
                "api_key": {
                    "type": "apiKey",
                    "in": "header",
                    "name": "X-Api-Key"
                }
            }),
        );

        let paths = obj
            .entry("paths".to_string())
            .or_insert(json!({}))
            .as_object_mut()
            .expect("paths object");

        let inventory: &[(&str, &str, &[&str])] = &[
            ("/api/health", "Health check", &["get"]),
            ("/api/auth/login", "Admin login", &["post"]),
            ("/api/auth/me", "Current admin user", &["get"]),
            (
                "/api/auth/change-password",
                "Change admin password",
                &["post"],
            ),
            (
                "/api/accounts",
                "List / create SIP accounts",
                &["get", "post"],
            ),
            (
                "/api/accounts/{id}",
                "Update / delete SIP account",
                &["put", "delete"],
            ),
            (
                "/api/registrations",
                "List / delete registrations",
                &["get", "delete"],
            ),
            ("/api/calls", "List / export call detail records", &["get"]),
            ("/api/calls/cleanup", "Close stale active calls", &["post"]),
            (
                "/api/messages",
                "List persisted SIP MESSAGE records",
                &["get"],
            ),
            (
                "/api/messages/history",
                "Phone message history (SIP auth)",
                &["post"],
            ),
            ("/api/stats", "Dashboard statistics", &["get"]),
            ("/api/security/events", "Security event timeline", &["get"]),
            (
                "/api/security/blocks",
                "Active auto-ban ACL entries",
                &["get"],
            ),
            (
                "/api/security/blocks/unblock",
                "Disable one auto-ban entry",
                &["post"],
            ),
            ("/api/security/summary", "Security summary", &["get"]),
            (
                "/api/security/runtime",
                "Runtime troubleshooting snapshot",
                &["get"],
            ),
            ("/api/acl", "List / create IP ACL rules", &["get", "post"]),
            (
                "/api/acl/{id}",
                "Update / delete IP ACL rule",
                &["put", "delete"],
            ),
            (
                "/api/admin/users",
                "List / create admin users",
                &["get", "post"],
            ),
            (
                "/api/admin/users/{id}",
                "Update / delete admin user",
                &["put", "delete"],
            ),
            (
                "/api/conferences",
                "List / create conference rooms",
                &["get", "post"],
            ),
            (
                "/api/conferences/{id}",
                "Update / delete conference room",
                &["put", "delete"],
            ),
            (
                "/api/conferences/{id}/participants",
                "List active conference participants",
                &["get"],
            ),
            (
                "/api/voicemail/boxes",
                "List / create voicemail mailboxes",
                &["get", "post"],
            ),
            (
                "/api/voicemail/boxes/{id}",
                "Update voicemail mailbox",
                &["put"],
            ),
            (
                "/api/voicemail/messages",
                "List voicemail messages",
                &["get"],
            ),
            (
                "/api/voicemail/messages/{id}",
                "Update or soft-delete a message",
                &["put", "delete"],
            ),
            (
                "/api/voicemail/messages/{id}/download",
                "Download message WAV audio",
                &["get"],
            ),
            (
                "/api/turn/credentials",
                "TURN creds (SIP HA1 auth)",
                &["post"],
            ),
            ("/api/turn/health", "TURN server reachability", &["get"]),
            ("/api/metrics", "Prometheus exposition", &["get"]),
        ];

        for (path, summary, methods) in inventory {
            let path_item = paths
                .entry(path.to_string())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap();
            for method in *methods {
                path_item.insert(
                    method.to_string(),
                    json!({
                        "summary": summary,
                        "responses": {
                            "200": { "description": "OK" },
                            "401": { "description": "Unauthorized" },
                            "403": { "description": "Forbidden" }
                        }
                    }),
                );
            }
        }
    }

    Json(doc)
}

/// Build a `SwaggerUi` ready to be merged into a typed `Router<S>`. The UI
/// is mounted at `/api/docs` (and any sub-paths needed by the bundle) and
/// uses `/api/openapi.json` as the spec source.
pub fn swagger_ui() -> utoipa_swagger_ui::SwaggerUi {
    utoipa_swagger_ui::SwaggerUi::new("/api/docs").url("/api/openapi.json", ApiDoc::openapi())
}

/// Handler that returns the OpenAPI 3.1 spec as JSON. Mounted at
/// `/api/openapi.json` from `api/mod.rs`.
pub use openapi_json as openapi_json_handler;
