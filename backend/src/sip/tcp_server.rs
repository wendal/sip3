use anyhow::{Result, anyhow};
use native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_native_tls::TlsAcceptor;
use tracing::{debug, info, warn};

use super::handler::SipHandler;
use super::transport::TransportRegistry;
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
    pump_registered_stream(
        BufReader::new(reader),
        &mut writer,
        src,
        handler.transport_registry(),
        |raw, src| {
            let handler = handler.clone();
            async move { handler.handle_tcp_msg(&raw, src).await }
        },
    )
    .await
}

async fn pump_registered_stream<R, W, F, Fut>(
    mut reader: BufReader<R>,
    mut writer: W,
    src: SocketAddr,
    registry: TransportRegistry,
    mut process_inbound: F,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnMut(String, SocketAddr) -> Fut,
    Fut: std::future::Future<Output = Result<Option<String>>>,
{
    let mut outbound = registry.register(src);
    let result = async {
        loop {
            tokio::select! {
                inbound = read_next_sip_message(&mut reader) => {
                    let Some(raw) = inbound? else {
                        break;
                    };
                    debug!("TLS SIP from {} ({} bytes)", src, raw.len());

                    match process_inbound(raw, src).await {
                        Ok(Some(resp)) => writer.write_all(resp.as_bytes()).await?,
                        Ok(None) => {}
                        Err(e) => warn!("Error processing SIP/TLS from {}: {}", src, e),
                    }
                }
                Some(message) = outbound.recv() => {
                    writer.write_all(message.as_bytes()).await?;
                }
            }
        }
        Ok(())
    }
    .await;
    registry.unregister(src);
    result
}

async fn read_next_sip_message<R>(reader: &mut BufReader<R>) -> Result<Option<String>>
where
    R: AsyncRead + Unpin,
{
    loop {
        let mut headers_raw = String::new();
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(None);
            }
            let is_blank = line == "\r\n" || line == "\n";
            headers_raw.push_str(&line);
            if is_blank {
                break;
            }
        }

        if headers_raw.trim().is_empty() {
            continue;
        }

        let content_length = extract_content_length(&headers_raw);
        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            reader.read_exact(&mut body).await?;
        }

        return Ok(Some(format!(
            "{}{}",
            headers_raw,
            String::from_utf8_lossy(&body)
        )));
    }
}

/// Extract the numeric value of the Content-Length header from raw SIP headers.
fn extract_content_length(headers: &str) -> usize {
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if (lower.starts_with("content-length:") || lower.starts_with("l:"))
            && let Some(colon) = line.find(':')
            && let Ok(n) = line[colon + 1..].trim().parse::<usize>()
        {
            return n;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::pump_registered_stream;
    use crate::sip::transport::TransportRegistry;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, BufReader, split};
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn tls_stream_pumps_registry_messages_and_unregisters_on_close() {
        let registry = TransportRegistry::default();
        let src: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let (client, server) = tokio::io::duplex(1024);
        let (server_read, server_write) = split(server);
        let task_registry = registry.clone();

        let task = tokio::spawn(async move {
            pump_registered_stream(
                BufReader::new(server_read),
                server_write,
                src,
                task_registry,
                |_raw, _src| async { Ok::<Option<String>, anyhow::Error>(None) },
            )
            .await
            .expect("pump stream");
        });

        for _ in 0..20 {
            if registry.contains(src) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            registry.contains(src),
            "stream should be registered for outbound SIP"
        );

        let outbound = "MESSAGE sip:1003@sip.air32.cn SIP/2.0\r\nContent-Length: 0\r\n\r\n";
        assert!(registry.send(src, outbound.to_string()));

        let mut client = client;
        let mut buf = vec![0u8; outbound.len()];
        timeout(Duration::from_millis(200), client.read_exact(&mut buf))
            .await
            .expect("read timeout")
            .expect("read outbound");
        assert_eq!(String::from_utf8(buf).unwrap(), outbound);

        drop(client);
        timeout(Duration::from_millis(200), task)
            .await
            .expect("task timeout")
            .expect("task join");
        assert!(
            !registry.contains(src),
            "stream should be unregistered after the TCP/TLS connection closes"
        );
    }
}
