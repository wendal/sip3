use anyhow::{Context, Result};
use sqlx::MySqlPool;
use sqlx::mysql::MySqlPoolOptions;
use std::time::Duration;
use tracing::{info, warn};

pub async fn init_pool(url: &str, max_connections: u32) -> Result<MySqlPool> {
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);
    let max_attempts = 10;

    let pool = {
        let mut attempt = 0u32;
        loop {
            match MySqlPoolOptions::new()
                .max_connections(max_connections)
                .connect(url)
                .await
            {
                Ok(pool) => break pool,
                Err(e) if attempt + 1 < max_attempts => {
                    attempt += 1;
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
    };

    info!("Running database migrations...");
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;
    info!("Database migrations complete.");

    Ok(pool)
}
