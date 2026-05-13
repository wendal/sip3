use anyhow::Result;
use sqlx::MySqlPool;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Semaphore;
use tracing::{info, warn};

use super::handler::SipHandler;
use crate::config::Config;

/// Maximum number of datagrams being processed concurrently.
/// Excess datagrams are dropped to prevent memory/CPU exhaustion under flood.
const MAX_CONCURRENT_TASKS: usize = 512;

pub async fn run(cfg: Config, pool: MySqlPool) -> Result<()> {
    let addr = format!("{}:{}", cfg.server.sip_host, cfg.server.sip_port);
    let socket = Arc::new(UdpSocket::bind(&addr).await?);
    info!("SIP server listening on udp://{}", addr);

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_TASKS));
    let handler = SipHandler::with_socket(cfg, pool, socket.clone());
    let mut buf = vec![0u8; 65535];

    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;
        let data = buf[..len].to_vec();

        // Try to acquire a concurrency permit; drop the packet if we're overloaded.
        let permit = match Arc::clone(&semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                warn!(
                    "Server overloaded ({} concurrent tasks), dropping packet from {}",
                    MAX_CONCURRENT_TASKS, src
                );
                continue;
            }
        };

        let handler_clone = handler.clone();
        tokio::spawn(async move {
            let _permit = permit; // released automatically when the task completes
            if let Err(e) = handler_clone.handle_datagram(data, src).await {
                tracing::error!("Error handling SIP datagram from {}: {}", src, e);
            }
        });
    }
}
