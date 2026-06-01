use thiserror::Error;

#[derive(Error, Debug)]
pub enum RegistrarError {
    #[error("invalid username in From header")]
    InvalidUsername,

    #[error("authentication failed for user: {0}")]
    AuthFailed(String),

    #[error("nonce expired or invalid")]
    NonceExpired,

    #[error("source IP blocked: {0}")]
    IpBlocked(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("security guard error: {0}")]
    SecurityGuard(String),
}

#[derive(Error, Debug)]
pub enum ProxyError {
    #[error("no registered contact for {0}")]
    NoContactFound(String),

    #[error("invalid SDP: {0}")]
    InvalidSdp(String),

    #[error("media relay error: {0}")]
    MediaRelay(String),

    #[error("no available media port")]
    NoAvailablePort,

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("target resolution failed: {0}")]
    TargetResolution(String),
}
