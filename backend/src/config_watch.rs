//! Runtime config hot-reload.
//!
//! Wraps an `Arc<ArcSwap<Config>>` and exposes:
//! - [`ConfigWatcher::reload_now`] — synchronous reload from env + optional
//!   `config.toml`, returning the new `Config` snapshot for inspection.
//! - [`ConfigWatcher::spawn_periodic_reload`] — spawns a `tokio` background
//!   task that polls on the `acl_refresh_interval_secs` cadence. Polling is
//!   cheap because we re-use the same `Config::load()` path as startup.
//!
//! Reload is "soft": if a port range, TLS path, or other structural value
//! changes, the warning is logged but the listener sockets are not re-bound.
//! In practice operators will restart the binary for port-class changes;
//! polling is for runtime-tunable items like `security.*` and `turn.*`.

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::config::Config;

pub struct ConfigWatcher {
    inner: Arc<ArcSwap<Config>>,
    interval_secs: u64,
}

impl ConfigWatcher {
    pub fn new(inner: Arc<ArcSwap<Config>>, interval_secs: u64) -> Self {
        Self {
            inner,
            interval_secs,
        }
    }

    /// Replace the live config with a fresh `Config::load()` result.
    /// Returns the new `Config` snapshot.
    pub async fn reload_now(&self) -> Result<Config> {
        let new_cfg = tokio::task::spawn_blocking(Config::load)
            .await
            .context("config reload task join")??;
        self.inner.store(Arc::new(new_cfg.clone()));
        self.warn_structural_changes(&new_cfg);
        Ok(new_cfg)
    }

    /// Spawn a tokio task that calls `reload_now` every `interval_secs`.
    /// Cheap on the steady state (no env change ⇒ load returns the same
    /// defaults; we still store it because `Config::load` is idempotent).
    pub fn spawn_periodic_reload(self: &Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            let interval = Duration::from_secs(me.interval_secs.max(5));
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // skip the first immediate tick
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = me.reload_now().await {
                    warn!("periodic config reload failed: {}", e);
                } else {
                    info!("periodic config reload complete");
                }
            }
        });
    }

    /// Log a warning if a port-class config value differs from the previous
    /// snapshot. We do not actually rebind sockets — operators must restart
    /// for those changes to take effect.
    fn warn_structural_changes(&self, new_cfg: &Config) {
        let prev = self.inner.load();
        let structural_changed = prev.server.sip_port != new_cfg.server.sip_port
            || prev.server.api_port != new_cfg.server.api_port
            || prev.server.tls_port != new_cfg.server.tls_port
            || prev.server.ws_port != new_cfg.server.ws_port
            || prev.server.wss_port != new_cfg.server.wss_port
            || prev.server.rtp_port_min != new_cfg.server.rtp_port_min
            || prev.server.rtp_port_max != new_cfg.server.rtp_port_max
            || prev.server.tls_cert != new_cfg.server.tls_cert
            || prev.server.tls_key != new_cfg.server.tls_key;
        if structural_changed {
            warn!(
                "config reloaded with structural changes (ports/TLS); \
                 restart the binary to apply them"
            );
        }
    }
}
