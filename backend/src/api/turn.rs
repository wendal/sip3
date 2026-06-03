use axum::{Json, extract::State, http::StatusCode};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::Sha1;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpSocket;

use super::AppState;

type HmacSha1 = Hmac<Sha1>;

#[derive(Deserialize)]
pub struct TurnCredentialsRequest {
    pub username: String,
    pub password: String,
}

/// `POST /api/turn/credentials`
///
/// Authenticates using SIP account credentials (HA1 digest), then returns
/// time-limited TURN credentials using coturn's HMAC-SHA1 mechanism.
///
/// Returns 503 when the TURN secret is not configured.
/// Returns 401 when the SIP credentials are invalid.
pub async fn credentials(
    State(state): State<AppState>,
    Json(body): Json<TurnCredentialsRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let secret = &state.config.load().turn.secret;
    if secret.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "TURN service not configured".to_string(),
        ));
    }

    if body.username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "username and password are required".to_string(),
        ));
    }

    // Determine realm: turn.realm overrides auth.realm.
    let realm = if state.config.load().turn.realm.is_empty() {
        state.config.load().auth.realm.clone()
    } else {
        state.config.load().turn.realm.clone()
    };

    // Verify SIP credentials by comparing against the stored HA1 hash.
    let ha1_computed = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", body.username, realm, body.password))
    );

    let stored: Option<String> = sqlx::query_scalar(
        "SELECT ha1_hash FROM sip_accounts WHERE username = ? AND domain = ? AND enabled = 1",
    )
    .bind(&body.username)
    .bind(&realm)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let stored_ha1 =
        stored.ok_or_else(|| (StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()))?;

    if ha1_computed != stored_ha1 {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()));
    }

    // Generate coturn time-limited credentials.
    // Format: username = "{expiry_unix_ts}:{sip_username}"
    //         password = base64(HMAC-SHA1(secret, username))
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expires = now + state.config.load().turn.ttl_secs;
    let turn_username = format!("{}:{}", expires, body.username);
    let turn_password = hmac_sha1_base64(secret, &turn_username);

    // Build the list of TURN URIs.
    let uris: Vec<String> = if state.config.load().turn.server.is_empty() {
        vec![
            format!("stun:{}:3478", state.config.load().server.public_ip),
            format!(
                "turn:{}:3478?transport=udp",
                state.config.load().server.public_ip
            ),
            format!(
                "turn:{}:3478?transport=tcp",
                state.config.load().server.public_ip
            ),
        ]
    } else {
        state
            .config
            .load()
            .turn
            .server
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    Ok(Json(json!({
        "username": turn_username,
        "password": turn_password,
        "ttl": state.config.load().turn.ttl_secs,
        "uris": uris,
    })))
}

fn hmac_sha1_base64(key: &str, data: &str) -> String {
    let mut mac =
        HmacSha1::new_from_slice(key.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(data.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

#[derive(Serialize)]
pub struct TurnHealthResponse {
    pub enabled: bool,
    pub servers: Vec<String>,
    pub reachable: bool,
}

fn parse_turn_uri(uri: &str) -> Option<SocketAddr> {
    let uri = uri
        .trim_start_matches("turn:")
        .trim_start_matches("stun:")
        .trim_start_matches("turns:");
    let (host, port) = if let Some(idx) = uri.rfind(':') {
        (&uri[..idx], &uri[idx + 1..])
    } else {
        (uri, "3478")
    };
    let port: u16 = port.parse().ok()?;
    let addr: SocketAddr = format!("{}:{}", host, port).parse().ok()?;
    Some(addr)
}

async fn check_tcp_reachable(addr: SocketAddr) -> bool {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    };
    match socket {
        Ok(socket) => {
            let connect =
                tokio::time::timeout(std::time::Duration::from_secs(2), socket.connect(addr));
            matches!(connect.await, Ok(Ok(_)))
        }
        Err(_) => false,
    }
}

pub async fn health(State(state): State<AppState>) -> Json<TurnHealthResponse> {
    let enabled = !state.config.load().turn.secret.is_empty();

    let servers: Vec<String> = if state.config.load().turn.server.is_empty() {
        vec![
            format!("{}:3478", state.config.load().server.public_ip),
            format!("{}:5349", state.config.load().server.public_ip),
        ]
    } else {
        state
            .config
            .load()
            .turn
            .server
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let mut reachable = false;
    if enabled {
        for server in &servers {
            if let Some(addr) = parse_turn_uri(server)
                && check_tcp_reachable(addr).await
            {
                reachable = true;
                break;
            }
        }
    }

    Json(TurnHealthResponse {
        enabled,
        servers,
        reachable,
    })
}
