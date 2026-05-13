use anyhow::Result;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;
use std::time::Duration;
use tracing::warn;

pub async fn init_pool(url: &str, max_connections: u32) -> Result<MySqlPool> {
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);
    let max_attempts = 10;

    for attempt in 1..=max_attempts {
        match MySqlPoolOptions::new()
            .max_connections(max_connections)
            .connect(url)
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(e) if attempt < max_attempts => {
                warn!(
                    "Database connection attempt {}/{} failed: {}. Retrying in {:?}...",
                    attempt, max_attempts, e, delay
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
            }
            Err(e) => return Err(e.into()),
        }
    }
    // Unreachable, but satisfies the compiler.
    unreachable!()
}
