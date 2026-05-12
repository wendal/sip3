use anyhow::Result;
use sqlx::MySqlPool;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::info;

use super::handler::SipHandler;
use crate::config::Config;

pub async fn run(cfg: Config, pool: MySqlPool) -> Result<()> {
    let addr = format!("{}:{}", cfg.server.sip_host, cfg.server.sip_port);
    let socket = Arc::new(UdpSocket::bind(&addr).await?);
    info!("SIP server listening on udp://{}", addr);

    let handler = SipHandler::with_socket(cfg, pool, socket.clone());
    let mut buf = vec![0u8; 65535];

    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;
        let data = buf[..len].to_vec();
        let handler_clone = handler.clone();

        tokio::spawn(async move {
            if let Err(e) = handler_clone.handle_datagram(data, src).await {
                tracing::error!("Error handling SIP datagram from {}: {}", src, e);
            }
        });
    }
}
