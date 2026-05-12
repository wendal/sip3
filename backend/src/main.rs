use anyhow::Result;
use tracing::info;

mod api;
mod config;
mod db;
mod models;
mod sip;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = config::Config::load()?;
    info!(
        "Configuration loaded, SIP domain: {}",
        cfg.server.sip_domain
    );

    let pool = db::init_pool(&cfg.database.url, cfg.database.max_connections).await?;
    info!("Database pool initialized");

    let sip_cfg = cfg.clone();
    let sip_pool = pool.clone();
    let sip_handle = tokio::spawn(async move {
        if let Err(e) = sip::server::run(sip_cfg, sip_pool).await {
            tracing::error!("SIP server error: {}", e);
        }
    });

    let api_cfg = cfg.clone();
    let api_pool = pool.clone();
    let api_handle = tokio::spawn(async move {
        if let Err(e) = api::run(api_cfg, api_pool).await {
            tracing::error!("API server error: {}", e);
        }
    });

    let (sip_result, api_result) = tokio::join!(sip_handle, api_handle);
    if let Err(e) = sip_result {
        tracing::error!("SIP task panicked: {}", e);
    }
    if let Err(e) = api_result {
        tracing::error!("API task panicked: {}", e);
    }
    Ok(())
}
