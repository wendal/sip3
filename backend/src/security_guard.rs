use anyhow::Result;
use sqlx::MySqlPool;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSurface {
    SipRegister,
    ApiLogin,
}

impl AuthSurface {
    pub fn as_db_value(self) -> &'static str {
        match self {
            AuthSurface::SipRegister => "sip_register",
            AuthSurface::ApiLogin => "api_login",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEventType {
    AuthFailed,
    IpBlocked,
    AuthSucceeded,
    IpUnblocked,
}

impl SecurityEventType {
    pub fn as_db_value(self) -> &'static str {
        match self {
            SecurityEventType::AuthFailed => "auth_failed",
            SecurityEventType::IpBlocked => "ip_blocked",
            SecurityEventType::AuthSucceeded => "auth_succeeded",
            SecurityEventType::IpUnblocked => "ip_unblocked",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GuardLimits {
    pub window_secs: u64,
    pub ip_fail_threshold: usize,
    pub user_ip_fail_threshold: usize,
    pub block_secs: u64,
}

#[derive(Debug, Clone)]
pub struct ActiveBlock {
    pub ip: String,
    pub until: Instant,
    pub surface: AuthSurface,
    pub username: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SecurityGuard {
    limits: GuardLimits,
    ip_failures: HashMap<String, VecDeque<Instant>>,
    user_ip_failures: HashMap<(String, String), VecDeque<Instant>>,
    blocks: HashMap<String, ActiveBlock>,
}

impl SecurityGuard {
    pub fn new(limits: GuardLimits) -> Self {
        Self {
            limits,
            ip_failures: HashMap::new(),
            user_ip_failures: HashMap::new(),
            blocks: HashMap::new(),
        }
    }

    pub fn is_blocked(&mut self, ip: &str) -> bool {
        self.prune_expired_blocks();
        self.blocks.contains_key(ip)
    }

    pub fn record_failure(
        &mut self,
        surface: AuthSurface,
        ip: &str,
        username: Option<&str>,
    ) -> bool {
        self.prune_expired_blocks();

        let now = Instant::now();
        let window = Duration::from_secs(self.limits.window_secs);

        let ip_key = ip.to_string();
        let ip_count = {
            let entries = self.ip_failures.entry(ip_key.clone()).or_default();
            entries.push_back(now);
            prune_old(entries, now, window);
            entries.len()
        };

        let user_ip_count = if let Some(user) = username {
            let key = (ip_key.clone(), user.to_string());
            let entries = self.user_ip_failures.entry(key).or_default();
            entries.push_back(now);
            prune_old(entries, now, window);
            entries.len()
        } else {
            0
        };

        if ip_count >= self.limits.ip_fail_threshold
            || user_ip_count >= self.limits.user_ip_fail_threshold
        {
            let until = now + Duration::from_secs(self.limits.block_secs);
            self.blocks.insert(
                ip_key.clone(),
                ActiveBlock {
                    ip: ip_key,
                    until,
                    surface,
                    username: username.map(|s| s.to_string()),
                },
            );
            return true;
        }

        false
    }

    pub fn record_success(&mut self, ip: &str, username: Option<&str>) {
        if let Some(user) = username {
            let key = (ip.to_string(), user.to_string());
            self.user_ip_failures.remove(&key);
        }
    }

    pub fn unblock(&mut self, ip: &str) {
        self.blocks.remove(ip);
    }

    pub fn list_active_blocks(&mut self) -> Vec<ActiveBlock> {
        self.prune_expired_blocks();
        self.blocks.values().cloned().collect()
    }

    fn prune_expired_blocks(&mut self) {
        let now = Instant::now();
        self.blocks.retain(|_, block| block.until > now);
    }
}

fn prune_old(entries: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while let Some(front) = entries.front().copied() {
        if now.saturating_duration_since(front) > window {
            entries.pop_front();
        } else {
            break;
        }
    }
}

pub async fn persist_security_event(
    pool: &MySqlPool,
    surface: AuthSurface,
    event_type: SecurityEventType,
    source_ip: &str,
    username: Option<&str>,
    detail: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO sip_security_events (surface, event_type, source_ip, username, detail)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(surface.as_db_value())
    .bind(event_type.as_db_value())
    .bind(source_ip)
    .bind(username)
    .bind(detail)
    .execute(pool)
    .await?;
    Ok(())
}
