use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub acl: AclConfig,
    pub security: SecurityConfig,
    pub turn: TurnConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub sip_host: String,
    pub sip_port: u16,
    pub sip_domain: String,
    pub api_host: String,
    pub api_port: u16,
    /// Comma-separated allowed CORS origins. Empty string = allow all (development).
    pub allowed_origins: String,
    /// Public IPv4 address (or resolvable hostname) of this server, written into
    /// rewritten SDP `c=IN IP4 <addr>` lines for media relay.
    pub public_ip: String,
    /// Lower bound of the UDP port range used for RTP media relay.
    pub rtp_port_min: u16,
    /// Upper bound of the UDP port range used for RTP media relay.
    pub rtp_port_max: u16,
    /// Lower bound of the UDP port range used for WebRTC ICE (must be Docker-mapped).
    pub webrtc_port_min: u16,
    /// Upper bound of the UDP port range used for WebRTC ICE (must be Docker-mapped).
    pub webrtc_port_max: u16,
    /// Lower bound of the UDP port range used for conference RTP sockets.
    pub conference_rtp_port_min: u16,
    /// Upper bound of the UDP port range used for conference RTP sockets.
    pub conference_rtp_port_max: u16,
    /// TCP+TLS SIP port (default: 5061). TLS is enabled only when tls_cert is set.
    pub tls_port: u16,
    /// Path to PEM-encoded TLS certificate chain. Empty = TLS disabled.
    pub tls_cert: String,
    /// Path to PEM-encoded TLS private key (PKCS#8 PEM). Empty = TLS disabled.
    pub tls_key: String,
    /// Plain WebSocket SIP port (ws://). Default 5080, empty/0 = disabled.
    pub ws_port: u16,
    /// Secure WebSocket SIP port (wss://). Requires tls_cert+tls_key. Default 5443, 0 = disabled.
    pub wss_port: u16,
    pub voicemail_access_extension: String,
    pub voicemail_no_answer_secs: u64,
    pub voicemail_max_message_secs: u64,
    pub voicemail_idle_timeout_secs: u64,
    pub voicemail_storage_dir: String,
    pub voicemail_prompt_dir: String,
    pub voicemail_rtp_port_min: u16,
    pub voicemail_rtp_port_max: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    pub realm: String,
    pub registration_expires: u32,
    /// Max age (seconds) for a nonce before it is rejected. Default: 300.
    pub nonce_max_age_secs: u64,
    /// HMAC secret for nonce signing. Empty = random value generated at startup.
    pub nonce_secret: String,
    /// If set, all /api/ routes (except /api/health) require `X-Api-Key: <value>`.
    pub api_key: Option<String>,
    /// Secret for signing JWT tokens. Empty = random value generated at startup.
    pub jwt_secret: String,
    /// JWT token lifetime in seconds. Default: 86400 (24 hours).
    pub jwt_expiry_secs: u64,
}

/// Configuration for the IP ACL (CIDR allow/deny list).
#[derive(Debug, Deserialize, Clone)]
pub struct AclConfig {
    /// Default action when no rule matches: "allow" or "deny" (default: "allow").
    pub default_policy: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SecurityConfig {
    /// Sliding window in seconds for counting auth failures.
    pub window_secs: u64,
    /// Failed REGISTER attempts from the same IP needed to trigger a block.
    pub sip_ip_fail_threshold: u32,
    /// Failed REGISTER attempts for same IP+username needed to trigger a block.
    pub sip_user_ip_fail_threshold: u32,
    /// Failed admin login attempts from the same IP needed to trigger a block.
    pub api_ip_fail_threshold: u32,
    /// Failed admin login attempts for same IP+username needed to trigger a block.
    pub api_user_ip_fail_threshold: u32,
    /// Block duration in seconds.
    pub block_secs: u64,
    /// Whether to persist auto blocks into sip_acl deny rules.
    pub persist_acl_bans: bool,
    /// Priority used when inserting auto-generated deny ACL rules.
    pub acl_ban_priority: i32,
}

/// Configuration for the built-in TURN credentials API.
#[derive(Debug, Deserialize, Clone)]
pub struct TurnConfig {
    /// Realm for TURN credentials (defaults to `auth.realm` when empty).
    pub realm: String,
    /// `static-auth-secret` shared with coturn.
    /// Empty string disables the `/api/turn/credentials` endpoint.
    pub secret: String,
    /// Credential TTL in seconds. coturn uses the embedded expiry timestamp.
    pub ttl_secs: u64,
    /// TURN server URI(s) returned to the browser, comma-separated.
    /// Example: `turn:sip.example.com:3478,turns:sip.example.com:5349`
    /// Defaults to `turn:{server.public_ip}:3478` when empty.
    pub server: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        let settings = config::Config::builder()
            .add_source(config::File::with_name("config").required(false))
            .add_source(
                config::Environment::with_prefix("SIP3")
                    .separator("__")
                    .try_parsing(true),
            )
            .set_default("server.sip_host", "0.0.0.0")?
            .set_default("server.sip_port", 5060)?
            .set_default("server.sip_domain", "sip.air32.cn")?
            .set_default("server.api_host", "0.0.0.0")?
            .set_default("server.api_port", 3000)?
            .set_default("server.allowed_origins", "")?
            .set_default("server.public_ip", "sip.air32.cn")?
            .set_default("server.rtp_port_min", 10000)?
            .set_default("server.rtp_port_max", 10099)?
            .set_default("server.webrtc_port_min", 20000)?
            .set_default("server.webrtc_port_max", 20099)?
            .set_default("server.conference_rtp_port_min", 10100)?
            .set_default("server.conference_rtp_port_max", 10199)?
            .set_default("server.tls_port", 5061)?
            .set_default("server.tls_cert", "")?
            .set_default("server.tls_key", "")?
            .set_default("server.ws_port", 5080)?
            .set_default("server.wss_port", 5443)?
            .set_default("server.voicemail_access_extension", "*97")?
            .set_default("server.voicemail_no_answer_secs", 25)?
            .set_default("server.voicemail_max_message_secs", 120)?
            .set_default("server.voicemail_idle_timeout_secs", 10)?
            .set_default("server.voicemail_storage_dir", "voicemail")?
            .set_default("server.voicemail_prompt_dir", "voicemail/prompts")?
            .set_default("server.voicemail_rtp_port_min", 10200)?
            .set_default("server.voicemail_rtp_port_max", 10299)?
            .set_default("database.url", "mysql://root:root@localhost:3306/sip3")?
            .set_default("database.max_connections", 10)?
            .set_default("auth.realm", "sip.air32.cn")?
            .set_default("auth.registration_expires", 3600)?
            .set_default("auth.nonce_max_age_secs", 300)?
            .set_default("auth.nonce_secret", "")?
            .set_default("auth.jwt_secret", "")?
            .set_default("auth.jwt_expiry_secs", 86400)?
            .set_default("acl.default_policy", "allow")?
            .set_default("security.window_secs", 300)?
            .set_default("security.sip_ip_fail_threshold", 20)?
            .set_default("security.sip_user_ip_fail_threshold", 8)?
            .set_default("security.api_ip_fail_threshold", 20)?
            .set_default("security.api_user_ip_fail_threshold", 8)?
            .set_default("security.block_secs", 900)?
            .set_default("security.persist_acl_bans", true)?
            .set_default("security.acl_ban_priority", 5)?
            .set_default("turn.realm", "")?
            .set_default("turn.secret", "")?
            .set_default("turn.ttl_secs", 86400u64)?
            .set_default("turn.server", "")?
            .build()?;

        let cfg: Config = settings.try_deserialize()?;
        Ok(cfg)
    }
}
