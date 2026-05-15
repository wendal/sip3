use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// Subject: admin username.
    pub sub: String,
    /// Issued-at (Unix timestamp).
    pub iat: i64,
    /// Expiry (Unix timestamp).
    pub exp: i64,
}

/// Issue a signed JWT for the given admin username.
pub fn issue_token(username: &str, secret: &str, expiry_secs: u64) -> anyhow::Result<String> {
    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: username.to_string(),
        iat: now,
        exp: now + expiry_secs as i64,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;
    Ok(token)
}

/// Verify and decode a JWT. Returns the claims on success.
pub fn verify_token(token: &str, secret: &str) -> anyhow::Result<Claims> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )?;
    Ok(data.claims)
}
