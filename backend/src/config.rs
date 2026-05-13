use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
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
            .set_default("server.rtp_port_max", 20000)?
            .set_default("database.url", "mysql://root:root@localhost:3306/sip3")?
            .set_default("database.max_connections", 10)?
            .set_default("auth.realm", "sip.air32.cn")?
            .set_default("auth.registration_expires", 3600)?
            .set_default("auth.nonce_max_age_secs", 300)?
            .set_default("auth.nonce_secret", "")?
            .build()?;

        let cfg: Config = settings.try_deserialize()?;
        Ok(cfg)
    }
}
