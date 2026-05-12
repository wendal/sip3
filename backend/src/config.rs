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
            .set_default("server.sip_domain", "sip.example.com")?
            .set_default("server.api_host", "0.0.0.0")?
            .set_default("server.api_port", 3000)?
            .set_default("database.url", "mysql://sip3:sip3pass@localhost:3306/sip3")?
            .set_default("database.max_connections", 10)?
            .set_default("auth.realm", "sip.example.com")?
            .set_default("auth.registration_expires", 3600)?
            .build()?;

        let cfg: Config = settings.try_deserialize()?;
        Ok(cfg)
    }
}
