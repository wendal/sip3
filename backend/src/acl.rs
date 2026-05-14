use anyhow::Result;
use ipnet::IpNet;
use sqlx::MySqlPool;
use std::net::IpAddr;
use std::str::FromStr;
use tracing::warn;

#[derive(Debug, Clone, PartialEq)]
pub enum AclAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
struct AclRule {
    action: AclAction,
    network: IpNet,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DefaultPolicy {
    Allow,
    Deny,
}

impl DefaultPolicy {
    pub fn from_config(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "deny" => DefaultPolicy::Deny,
            _ => DefaultPolicy::Allow,
        }
    }

    fn is_allowed(&self) -> bool {
        matches!(self, DefaultPolicy::Allow)
    }
}

/// In-memory ACL evaluator. Loaded from `sip_acl` table; refreshed periodically.
#[derive(Debug, Clone)]
pub struct AclChecker {
    rules: Vec<AclRule>,
    default_policy: DefaultPolicy,
}

impl AclChecker {
    /// Create an empty checker (allows all traffic if policy is Allow).
    pub fn new(default_policy: DefaultPolicy) -> Self {
        Self {
            rules: Vec::new(),
            default_policy,
        }
    }

    /// Returns `true` if the IP is permitted by the current rule set.
    pub fn is_allowed(&self, ip: IpAddr) -> bool {
        for rule in &self.rules {
            if rule.network.contains(&ip) {
                return rule.action == AclAction::Allow;
            }
        }
        self.default_policy.is_allowed()
    }

    /// Load enabled rules from the database, sorted by priority ascending.
    pub async fn load_from_db(pool: &MySqlPool, default_policy: DefaultPolicy) -> Result<Self> {
        let rows: Vec<(u32, String, String)> = sqlx::query_as(
            "SELECT id, action, cidr FROM sip_acl WHERE enabled = 1 ORDER BY priority ASC, id ASC",
        )
        .fetch_all(pool)
        .await?;

        let mut rules = Vec::with_capacity(rows.len());
        for (id, action_str, cidr) in rows {
            let network = match IpNet::from_str(&cidr) {
                Ok(n) => n,
                Err(e) => {
                    warn!("ACL rule {}: invalid CIDR '{}': {} — skipped", id, cidr, e);
                    continue;
                }
            };
            let action = match action_str.as_str() {
                "allow" => AclAction::Allow,
                "deny" => AclAction::Deny,
                other => {
                    warn!("ACL rule {}: unknown action '{}' — skipped", id, other);
                    continue;
                }
            };
            rules.push(AclRule { action, network });
        }

        Ok(AclChecker {
            rules,
            default_policy,
        })
    }
}

/// Validate that a string is a syntactically valid CIDR (IPv4 or IPv6).
/// Returns the host-bits-zeroed canonical form on success.
pub fn parse_cidr(s: &str) -> Result<String, String> {
    IpNet::from_str(s)
        .map(|n| n.to_string())
        .map_err(|e| format!("invalid CIDR '{}': {}", s, e))
}
