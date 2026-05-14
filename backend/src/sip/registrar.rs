use anyhow::Result;
use chrono::{Duration, Utc};
use rand::Rng;
use sqlx::MySqlPool;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::handler::{
    base_response, extract_uri, make_www_authenticate, md5_hex, parse_auth_params, uri_username,
    SipMessage,
};
use super::presence::{Presence, PresenceStatus};
use crate::config::Config;
use crate::security_guard::{
    persist_security_event, AuthSurface, SecurityEventType, SecurityGuard,
};

pub const ACCOUNT_LOOKUP_SQL: &str = "\
    SELECT COALESCE(ha1_hash, ''), enabled
    FROM sip_accounts WHERE username = ? AND domain = ?";

#[derive(Clone)]
pub struct Registrar {
    pool: MySqlPool,
    cfg: Config,
    /// Secret used to sign nonces (HMAC-MD5). Generated at startup if not configured.
    nonce_secret: String,
    presence: Presence,
    guard: Arc<Mutex<SecurityGuard>>,
}

impl Registrar {
    pub fn new(
        pool: MySqlPool,
        cfg: Config,
        presence: Presence,
        guard: Arc<Mutex<SecurityGuard>>,
    ) -> Self {
        let nonce_secret = if cfg.auth.nonce_secret.is_empty() {
            generate_random_hex(16)
        } else {
            cfg.auth.nonce_secret.clone()
        };
        Self {
            pool,
            cfg,
            nonce_secret,
            presence,
            guard,
        }
    }

    pub async fn handle_register(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let from = msg.from_header().unwrap_or("");
        let from_uri = extract_uri(from).unwrap_or_default();
        let username = uri_username(&from_uri).unwrap_or_default();

        if username.is_empty() {
            warn!("REGISTER with no username from {}", src);
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        let source_ip = src.ip().to_string();
        if self.guard.lock().await.is_blocked(&source_ip) {
            warn!("REGISTER blocked by guard from {}", source_ip);
            return Ok(base_response(msg, 403, "Forbidden").build());
        }

        let domain = &self.cfg.server.sip_domain;
        if let Some(auth_header) = msg.authorization() {
            let auth_params = parse_auth_params(auth_header);

            // Validate nonce authenticity and age before verifying credentials.
            let nonce = auth_params.get("nonce").map(|s| s.as_str()).unwrap_or("");
            if !validate_nonce(nonce, &self.nonce_secret, self.cfg.auth.nonce_max_age_secs) {
                warn!("Stale or invalid nonce for user: {}", username);
                let blocked = self.guard.lock().await.record_failure(
                    AuthSurface::SipRegister,
                    &source_ip,
                    Some(&username),
                );
                if let Err(e) = persist_security_event(
                    &self.pool,
                    AuthSurface::SipRegister,
                    SecurityEventType::AuthFailed,
                    &source_ip,
                    Some(&username),
                    "register auth failed: stale or invalid nonce",
                )
                .await
                {
                    warn!("Failed to persist security event: {}", e);
                }
                if blocked && self.cfg.security.persist_acl_bans {
                    if let Err(e) = persist_acl_ban(
                        &self.pool,
                        src.ip(),
                        self.cfg.security.acl_ban_priority,
                        "auto-ban: sip register brute force",
                    )
                    .await
                    {
                        warn!(
                            "Failed to persist REGISTER auto-ban ACL for {}: {}",
                            source_ip, e
                        );
                    }
                    if let Err(e) = persist_security_event(
                        &self.pool,
                        AuthSurface::SipRegister,
                        SecurityEventType::IpBlocked,
                        &source_ip,
                        Some(&username),
                        "register source blocked by threshold",
                    )
                    .await
                    {
                        warn!("Failed to persist security event: {}", e);
                    }
                }
                return unauthorized_response(msg, domain, &self.nonce_secret);
            }

            // Account lookup by (username, domain) so the same username can exist in
            // multiple domains (AoR = user@domain).
            let row: Option<(String, i8)> = sqlx::query_as(ACCOUNT_LOOKUP_SQL)
                .bind(&username)
                .bind(domain)
                .fetch_optional(&self.pool)
                .await?;
            let account_ok =
                matches!(&row, Some((ha1, enabled)) if *enabled != 0 && !ha1.is_empty());

            let digest_ok = if let Some((ha1, enabled)) = &row {
                *enabled != 0
                    && !ha1.is_empty()
                    && verify_digest_with_ha1(&auth_params, ha1, "REGISTER")
            } else {
                false
            };

            if !account_ok || !digest_ok {
                warn!("Authentication failed for user: {}", username);
                let blocked = self.guard.lock().await.record_failure(
                    AuthSurface::SipRegister,
                    &source_ip,
                    Some(&username),
                );
                if let Err(e) = persist_security_event(
                    &self.pool,
                    AuthSurface::SipRegister,
                    SecurityEventType::AuthFailed,
                    &source_ip,
                    Some(&username),
                    "register auth failed: invalid account or digest",
                )
                .await
                {
                    warn!("Failed to persist security event: {}", e);
                }
                if blocked && self.cfg.security.persist_acl_bans {
                    if let Err(e) = persist_acl_ban(
                        &self.pool,
                        src.ip(),
                        self.cfg.security.acl_ban_priority,
                        "auto-ban: sip register brute force",
                    )
                    .await
                    {
                        warn!(
                            "Failed to persist REGISTER auto-ban ACL for {}: {}",
                            source_ip, e
                        );
                    }
                    if let Err(e) = persist_security_event(
                        &self.pool,
                        AuthSurface::SipRegister,
                        SecurityEventType::IpBlocked,
                        &source_ip,
                        Some(&username),
                        "register source blocked by threshold",
                    )
                    .await
                    {
                        warn!("Failed to persist security event: {}", e);
                    }
                }
                return unauthorized_response(msg, domain, &self.nonce_secret);
            }
            self.guard
                .lock()
                .await
                .record_success(&source_ip, Some(&username));
            if let Err(e) = persist_security_event(
                &self.pool,
                AuthSurface::SipRegister,
                SecurityEventType::AuthSucceeded,
                &source_ip,
                Some(&username),
                "register auth succeeded",
            )
            .await
            {
                warn!("Failed to persist security event: {}", e);
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
                    .bind(domain)
                    .execute(&self.pool)
                    .await?;
                info!("Unregistered user: {}", username);
                self.presence
                    .notify_status_change(&username, domain, PresenceStatus::Closed)
                    .await;
                return Ok(base_response(msg, 200, "OK").build());
            }

            let contact_uri = extract_uri(contact).unwrap_or_else(|| contact.to_string());
            let user_agent = msg.user_agent().unwrap_or("");
            let expires_at = (Utc::now() + Duration::seconds(expires as i64)).naive_utc();
            let source_port = src.port();

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
            self.presence
                .notify_status_change(&username, domain, PresenceStatus::Open)
                .await;

            Ok(base_response(msg, 200, "OK")
                .header("Contact", &format!("<{}>;expires={}", contact_uri, expires))
                .build())
        } else {
            // No auth header - send a fresh challenge with a signed nonce.
            unauthorized_response(msg, domain, &self.nonce_secret)
        }
    }
}

fn unauthorized_response(msg: &SipMessage, domain: &str, nonce_secret: &str) -> Result<String> {
    let nonce = generate_nonce(nonce_secret);
    Ok(base_response(msg, 401, "Unauthorized")
        .header("WWW-Authenticate", &make_www_authenticate(domain, &nonce))
        .build())
}

async fn persist_acl_ban(
    pool: &MySqlPool,
    ip: IpAddr,
    priority: i32,
    description: &str,
) -> Result<()> {
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

/// Generate an HMAC-MD5 signed nonce:
/// nonce = `{data}:{mac}` where
///   data = hex(unix_timestamp_u32)(8 chars) + hex(8 random bytes)(16 chars) = 24 chars
///   mac  = MD5(secret + ":" + data) (32 hex chars)
pub fn generate_nonce(secret: &str) -> String {
    let ts = Utc::now().timestamp() as u32;
    let random_hex = generate_random_hex(8); // 16 hex chars
    let data = format!("{:08x}{}", ts, random_hex); // 24 chars
    let mac = md5_hex(&format!("{}:{}", secret, data));
    format!("{}:{}", data, mac)
}

/// Validate a nonce produced by `generate_nonce`.
/// Returns `false` if the MAC is wrong or the nonce is older than `max_age_secs`.
pub fn validate_nonce(nonce: &str, secret: &str, max_age_secs: u64) -> bool {
    // Expected format: 24-char data + ':' + 32-char mac (57 chars total)
    if nonce.len() < 57 {
        return false;
    }
    // The data part is all hex and contains no ':', so the first ':' is at position 24.
    if nonce.as_bytes().get(24) != Some(&b':') {
        return false;
    }
    let data = &nonce[..24];
    let mac = &nonce[25..];
    if mac.len() != 32 {
        return false;
    }

    // Verify MAC
    let expected_mac = md5_hex(&format!("{}:{}", secret, data));
    if expected_mac != mac {
        return false;
    }

    // Check timestamp age
    let ts = match u64::from_str_radix(&data[..8], 16) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let now = Utc::now().timestamp() as u64;
    now.saturating_sub(ts) < max_age_secs
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

fn generate_random_hex(bytes: usize) -> String {
    let mut rng = rand::rng();
    (0..bytes)
        .map(|_| format!("{:02x}", rng.random::<u8>()))
        .collect()
}
