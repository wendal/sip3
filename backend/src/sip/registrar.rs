use anyhow::Result;
use chrono::{Duration, Utc};
use rand::Rng;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use tracing::{info, warn};

use super::handler::{
    base_response, extract_uri, make_www_authenticate, md5_hex, parse_auth_params, uri_username,
    SipMessage,
};
use crate::config::Config;

#[derive(Clone)]
pub struct Registrar {
    pool: MySqlPool,
    cfg: Config,
}

impl Registrar {
    pub fn new(pool: MySqlPool, cfg: Config) -> Self {
        Self { pool, cfg }
    }

    pub async fn handle_register(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let from = msg.from_header().unwrap_or("");
        let from_uri = extract_uri(from).unwrap_or_default();
        let username = uri_username(&from_uri).unwrap_or_default();

        if username.is_empty() {
            warn!("REGISTER with no username from {}", src);
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        // Fetch account
        let row: Option<(i64, String, i8)> = sqlx::query_as(
            "SELECT id, COALESCE(ha1_hash, ''), enabled FROM sip_accounts WHERE username = ?",
        )
        .bind(&username)
        .fetch_optional(&self.pool)
        .await?;

        let (_, ha1, enabled) = match row {
            Some(r) => r,
            None => {
                warn!("REGISTER for unknown user: {}", username);
                return Ok(base_response(msg, 404, "Not Found").build());
            }
        };

        if enabled == 0 {
            return Ok(base_response(msg, 403, "Forbidden").build());
        }

        if ha1.is_empty() {
            warn!("Account {} has no ha1_hash configured", username);
            return Ok(base_response(msg, 500, "Internal Server Error").build());
        }

        if let Some(auth_header) = msg.authorization() {
            let auth_params = parse_auth_params(auth_header);

            if !verify_digest_with_ha1(&auth_params, &ha1, "REGISTER") {
                warn!("Authentication failed for user: {}", username);
                let nonce = generate_nonce();
                return Ok(base_response(msg, 401, "Unauthorized")
                    .header(
                        "WWW-Authenticate",
                        &make_www_authenticate(&self.cfg.auth.realm, &nonce),
                    )
                    .build());
            }

            // Auth OK - process registration
            let contact = msg.contact().unwrap_or("*");
            let expires = msg
                .expires()
                .or_else(|| {
                    contact
                        .split(';')
                        .find(|p| p.trim().to_lowercase().starts_with("expires="))
                        .and_then(|p| p.split('=').nth(1))
                        .and_then(|s| s.trim().parse().ok())
                })
                .unwrap_or(self.cfg.auth.registration_expires);

            if contact.trim() == "*" || expires == 0 {
                sqlx::query("DELETE FROM sip_registrations WHERE username = ? AND domain = ?")
                    .bind(&username)
                    .bind(&self.cfg.server.sip_domain)
                    .execute(&self.pool)
                    .await?;
                info!("Unregistered user: {}", username);
                return Ok(base_response(msg, 200, "OK").build());
            }

            let contact_uri = extract_uri(contact).unwrap_or_else(|| contact.to_string());
            let user_agent = msg.user_agent().unwrap_or("");
            let expires_at = (Utc::now() + Duration::seconds(expires as i64)).naive_utc();
            let source_ip = src.ip().to_string();
            let source_port = src.port();
            let domain = &self.cfg.server.sip_domain;

            sqlx::query(
                r#"INSERT INTO sip_registrations
                    (username, domain, contact_uri, user_agent, expires_at, source_ip, source_port)
                   VALUES (?, ?, ?, ?, ?, ?, ?)
                   ON DUPLICATE KEY UPDATE
                    contact_uri = VALUES(contact_uri),
                    user_agent = VALUES(user_agent),
                    expires_at = VALUES(expires_at),
                    source_ip = VALUES(source_ip),
                    source_port = VALUES(source_port),
                    registered_at = CURRENT_TIMESTAMP"#,
            )
            .bind(&username)
            .bind(domain)
            .bind(&contact_uri)
            .bind(user_agent)
            .bind(expires_at)
            .bind(&source_ip)
            .bind(source_port)
            .execute(&self.pool)
            .await?;

            info!(
                "Registered {} at {} (expires in {}s)",
                username, contact_uri, expires
            );

            Ok(base_response(msg, 200, "OK")
                .header("Contact", &format!("<{}>;expires={}", contact_uri, expires))
                .build())
        } else {
            // No auth header - send challenge
            let nonce = generate_nonce();
            Ok(base_response(msg, 401, "Unauthorized")
                .header(
                    "WWW-Authenticate",
                    &make_www_authenticate(&self.cfg.auth.realm, &nonce),
                )
                .build())
        }
    }
}

fn verify_digest_with_ha1(
    auth_params: &std::collections::HashMap<String, String>,
    ha1: &str,
    method: &str,
) -> bool {
    let uri = auth_params.get("uri").map(|s| s.as_str()).unwrap_or("");
    let response = auth_params
        .get("response")
        .map(|s| s.as_str())
        .unwrap_or("");
    let nonce = auth_params.get("nonce").map(|s| s.as_str()).unwrap_or("");
    let qop = auth_params.get("qop").map(|s| s.as_str()).unwrap_or("");
    let nc = auth_params.get("nc").map(|s| s.as_str()).unwrap_or("");
    let cnonce = auth_params.get("cnonce").map(|s| s.as_str()).unwrap_or("");

    let ha2 = md5_hex(&format!("{}:{}", method, uri));

    let expected = if qop == "auth" {
        md5_hex(&format!(
            "{}:{}:{}:{}:{}:{}",
            ha1, nonce, nc, cnonce, qop, ha2
        ))
    } else {
        md5_hex(&format!("{}:{}:{}", ha1, nonce, ha2))
    };

    expected == response
}

fn generate_nonce() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
