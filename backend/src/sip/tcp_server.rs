use anyhow::{anyhow, Result};
use native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_native_tls::TlsAcceptor;
use tracing::{debug, info, warn};

use super::handler::SipHandler;
use crate::config::Config;

fn load_identity(cert_path: &str, key_path: &str) -> Result<Identity> {
    let cert_pem = std::fs::read(cert_path)
        .map_err(|e| anyhow!("Failed to read TLS cert '{}': {}", cert_path, e))?;
    let key_pem = std::fs::read(key_path)
        .map_err(|e| anyhow!("Failed to read TLS key '{}': {}", key_path, e))?;
    Identity::from_pkcs8(&cert_pem, &key_pem)
        .map_err(|e| anyhow!("Failed to create TLS identity: {}", e))
}

pub async fn run(cfg: Config, _pool: MySqlPool, handler: SipHandler) -> Result<()> {
    let identity = load_identity(&cfg.server.tls_cert, &cfg.server.tls_key)?;
    let native_acceptor = NativeTlsAcceptor::new(identity)
        .map_err(|e| anyhow!("Failed to build TLS acceptor: {}", e))?;
    let acceptor = Arc::new(TlsAcceptor::from(native_acceptor));

    let addr = format!("{}:{}", cfg.server.sip_host, cfg.server.tls_port);
    let listener = TcpListener::bind(&addr).await?;
    info!("SIP/TLS server listening on tcp://{}", addr);

    loop {
        let (stream, src) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("TCP accept error: {}", e);
                continue;
            }
        };

        let acceptor = Arc::clone(&acceptor);
        let handler = handler.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(acceptor, stream, src, handler).await {
                warn!("TLS connection error from {}: {}", src, e);
            }
        });
    }
}

async fn handle_connection(
    acceptor: Arc<TlsAcceptor>,
    stream: TcpStream,
    src: SocketAddr,
    handler: SipHandler,
) -> Result<()> {
    let tls_stream = acceptor
        .accept(stream)
        .await
        .map_err(|e| anyhow!("TLS handshake failed from {}: {}", src, e))?;
    debug!("TLS connection established from {}", src);

    let (reader, mut writer) = tokio::io::split(tls_stream);
    let mut reader = BufReader::new(reader);

    loop {
        // Read headers until the blank line that separates headers from body.
        let mut headers_raw = String::new();
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(()); // connection closed cleanly
            }
            let is_blank = line == "\r\n" || line == "\n";
            headers_raw.push_str(&line);
            if is_blank {
                break;
            }
        }

        // RFC 5626 keepalive: a double-CRLF with no request line before it.
        if headers_raw.trim().is_empty() {
            continue;
        }

        // Use Content-Length to read the body without blocking on a delimiter.
        let content_length = extract_content_length(&headers_raw);
        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            reader.read_exact(&mut body).await?;
        }

        let raw = format!("{}{}", headers_raw, String::from_utf8_lossy(&body));
        debug!("TLS SIP from {} ({} bytes)", src, raw.len());

        match handler.handle_tcp_msg(&raw, src).await {
            Ok(Some(resp)) => {
                if let Err(e) = writer.write_all(resp.as_bytes()).await {
                    warn!("Failed to write TLS response to {}: {}", src, e);
                    return Ok(());
                }
            }
            Ok(None) => {} // ACK, relayed response, or keepalive — no reply needed
            Err(e) => {
                warn!("Error processing SIP/TLS from {}: {}", src, e);
            }
        }
    }
}

/// Extract the numeric value of the Content-Length header from raw SIP headers.
fn extract_content_length(headers: &str) -> usize {
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("content-length:") || lower.starts_with("l:") {
            if let Some(colon) = line.find(':') {
                if let Ok(n) = line[colon + 1..].trim().parse::<usize>() {
                    return n;
                }
            }
        }
    }
    0
}
