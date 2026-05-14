use anyhow::Result;
use sqlx::MySqlPool;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::{RwLock, Semaphore};
use tracing::{info, warn};

use super::handler::SipHandler;
use crate::acl::{AclChecker, DefaultPolicy};
use crate::config::Config;

/// Maximum number of datagrams being processed concurrently.
/// Excess datagrams are dropped to prevent memory/CPU exhaustion under flood.
const MAX_CONCURRENT_TASKS: usize = 512;

/// Media sessions older than this are considered stale (no BYE received).
const MEDIA_SESSION_MAX_AGE_SECS: u64 = 7200; // 2 hours

/// How often to check for stale media sessions.
const MEDIA_CLEANUP_INTERVAL_SECS: u64 = 60;

/// How often to purge expired registration rows from the database.
const REG_CLEANUP_INTERVAL_SECS: u64 = 3600; // 1 hour

/// How often to purge expired presence subscription rows from the database.
const PRES_CLEANUP_INTERVAL_SECS: u64 = 300; // 5 minutes

/// How often to reload ACL rules from the database.
const ACL_REFRESH_INTERVAL_SECS: u64 = 60;

pub async fn run(cfg: Config, pool: MySqlPool) -> Result<()> {
    let addr = format!("{}:{}", cfg.server.sip_host, cfg.server.sip_port);
    let socket = Arc::new(UdpSocket::bind(&addr).await?);
    info!("SIP server listening on udp://{}", addr);

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_TASKS));
    let handler = SipHandler::with_socket(cfg.clone(), pool.clone(), socket.clone());
    let mut buf = vec![0u8; 65535];

    // Load ACL rules from DB and wrap in a shared reader-writer lock.
    let default_policy = DefaultPolicy::from_config(&cfg.acl.default_policy);
    let acl_checker = match AclChecker::load_from_db(&pool, default_policy.clone()).await {
        Ok(a) => {
            info!("ACL loaded ({} rules, default: {})", 0, &cfg.acl.default_policy);
            Arc::new(RwLock::new(a))
        }
        Err(e) => {
            warn!("Failed to load ACL rules: {} — allowing all traffic", e);
            Arc::new(RwLock::new(AclChecker::new(default_policy.clone())))
        }
    };

    // Background task: abort stale media relay sessions (handles client crashes
    // or network failures where BYE is never received, preventing port leaks).
    let media_relay_cleanup = handler.media_relay().clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(MEDIA_CLEANUP_INTERVAL_SECS));
        loop {
            interval.tick().await;
            media_relay_cleanup
                .cleanup_stale_sessions(MEDIA_SESSION_MAX_AGE_SECS)
                .await;
        }
    });

    // Background task: delete expired registration rows to keep the table tidy.
    let pool_cleanup = pool.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(REG_CLEANUP_INTERVAL_SECS));
        loop {
            interval.tick().await;
            match sqlx::query("DELETE FROM sip_registrations WHERE expires_at < NOW()")
                .execute(&pool_cleanup)
                .await
            {
                Ok(r) if r.rows_affected() > 0 => {
                    info!(
                        "Cleaned up {} expired registration(s)",
                        r.rows_affected()
                    );
                }
                Err(e) => warn!("Registration cleanup error: {}", e),
                _ => {}
            }
        }
    });

    // Background task: delete expired presence subscription rows.
    let pool_pres = pool.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(PRES_CLEANUP_INTERVAL_SECS));
        loop {
            interval.tick().await;
            match sqlx::query(
                "DELETE FROM sip_presence_subscriptions WHERE expires_at < NOW()",
            )
            .execute(&pool_pres)
            .await
            {
                Ok(r) if r.rows_affected() > 0 => {
                    info!(
                        "Cleaned up {} expired presence subscription(s)",
                        r.rows_affected()
                    );
                }
                Err(e) => warn!("Presence cleanup error: {}", e),
                _ => {}
            }
        }
    });

    // Background task: periodically reload ACL rules from the database.
    let acl_refresh = Arc::clone(&acl_checker);
    let pool_acl = pool.clone();
    let acl_default = default_policy;
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(ACL_REFRESH_INTERVAL_SECS));
        loop {
            interval.tick().await;
            match AclChecker::load_from_db(&pool_acl, acl_default.clone()).await {
                Ok(new_acl) => {
                    *acl_refresh.write().await = new_acl;
                    info!("ACL rules refreshed");
                }
                Err(e) => warn!("ACL refresh error: {}", e),
            }
        }
    });

    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;

        // ACL check: drop packets from blocked IPs before any further processing.
        if !acl_checker.read().await.is_allowed(src.ip()) {
            warn!("ACL: blocked packet from {}", src);
            continue;
        }

        let data = buf[..len].to_vec();

        // Try to acquire a concurrency permit; drop the packet if we're overloaded.
        let permit = match Arc::clone(&semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                warn!(
                    "Server overloaded ({} concurrent tasks), dropping packet from {}",
                    MAX_CONCURRENT_TASKS, src
                );
                continue;
            }
        };

        let handler_clone = handler.clone();
        tokio::spawn(async move {
            let _permit = permit; // released automatically when the task completes
            if let Err(e) = handler_clone.handle_datagram(data, src).await {
                tracing::error!("Error handling SIP datagram from {}: {}", src, e);
            }
        });
    }
}
