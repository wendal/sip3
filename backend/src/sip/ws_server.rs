use anyhow::{Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio_native_tls::TlsAcceptor;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tracing::{debug, info, warn};

use super::handler::SipHandler;
use crate::config::Config;

/// Run the plain WebSocket SIP server (ws://) on `cfg.server.ws_port`.
pub async fn run_ws(cfg: Config, _pool: MySqlPool, handler: SipHandler) -> Result<()> {
    let addr = format!("{}:{}", cfg.server.sip_host, cfg.server.ws_port);
    let listener = TcpListener::bind(&addr).await?;
    info!("SIP/WS server listening on ws://{}", addr);

    loop {
        let (stream, src) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("WS accept error: {}", e);
                continue;
            }
        };
        let handler = handler.clone();
        tokio::spawn(async move {
            match accept_ws(stream).await {
                Ok(ws) => {
                    if let Err(e) = process_ws(ws, src, handler).await {
                        warn!("WS session error from {}: {}", src, e);
                    }
                }
                Err(e) => warn!("WS handshake failed from {}: {}", src, e),
            }
        });
    }
}

/// Run the secure WebSocket SIP server (wss://) on `cfg.server.wss_port`.
/// Requires `cfg.server.tls_cert` and `cfg.server.tls_key` to be set.
pub async fn run_wss(cfg: Config, _pool: MySqlPool, handler: SipHandler) -> Result<()> {
    let identity = load_identity(&cfg.server.tls_cert, &cfg.server.tls_key)?;
    let native_acceptor = NativeTlsAcceptor::new(identity)
        .map_err(|e| anyhow!("Failed to build WSS TLS acceptor: {}", e))?;
    let tls_acceptor = Arc::new(TlsAcceptor::from(native_acceptor));

    let addr = format!("{}:{}", cfg.server.sip_host, cfg.server.wss_port);
    let listener = TcpListener::bind(&addr).await?;
    info!("SIP/WSS server listening on wss://{}", addr);

    loop {
        let (stream, src) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("WSS accept error: {}", e);
                continue;
            }
        };
        let tls_acceptor = Arc::clone(&tls_acceptor);
        let handler = handler.clone();
        tokio::spawn(async move {
            let tls_stream = match tls_acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("WSS TLS handshake failed from {}: {}", src, e);
                    return;
                }
            };
            match accept_ws(tls_stream).await {
                Ok(ws) => {
                    if let Err(e) = process_ws(ws, src, handler).await {
                        warn!("WSS session error from {}: {}", src, e);
                    }
                }
                Err(e) => warn!("WSS WS handshake failed from {}: {}", src, e),
            }
        });
    }
}

/// Perform the WebSocket handshake, advertising "sip" as the subprotocol per RFC 7118.
#[allow(clippy::result_large_err)]
async fn accept_ws<S>(stream: S) -> Result<WebSocketStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio_tungstenite::accept_hdr_async(stream, |req: &Request, mut resp: Response| {
        // Echo back "sip" subprotocol if the client requested it (RFC 7118 §5).
        let wants_sip = req
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').any(|p| p.trim().eq_ignore_ascii_case("sip")))
            .unwrap_or(false);
        if wants_sip {
            resp.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                tokio_tungstenite::tungstenite::http::HeaderValue::from_static("sip"),
            );
        }
        Ok(resp)
    })
    .await
    .map_err(|e| anyhow!("WS handshake error: {}", e))
}

/// Receive SIP messages from a WebSocket connection, process each, and send replies.
async fn process_ws<S>(
    ws_stream: WebSocketStream<S>,
    src: SocketAddr,
    handler: SipHandler,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    debug!("WS/WSS connection from {}", src);
    let (mut sink, mut stream) = ws_stream.split();
    let mut outbound = handler.register_stream(src);

    loop {
        tokio::select! {
            frame = stream.next() => {
                let Some(frame) = frame else { break };
                let frame = match frame {
                    Ok(f) => f,
                    Err(e) => {
                        debug!("WS read error from {}: {}", src, e);
                        break;
                    }
                };

                let text = match frame {
                    Message::Text(t) => t,
                    Message::Binary(b) => match String::from_utf8(b) {
                        Ok(s) => s,
                        Err(_) => continue,
                    },
                    Message::Ping(data) => {
                        let _ = sink.send(Message::Pong(data)).await;
                        continue;
                    }
                    Message::Close(_) => break,
                    _ => continue,
                };

                debug!("WS SIP from {} ({} bytes)", src, text.len());
                match handler.handle_tcp_msg(&text, src).await {
                    Ok(Some(resp)) => {
                        if let Err(e) = sink.send(Message::Text(resp)).await {
                            warn!("WS write error to {}: {}", src, e);
                            break;
                        }
                    }
                    Ok(None) => {} // ACK, relayed response — no reply needed
                    Err(e) => warn!("WS SIP processing error from {}: {}", src, e),
                }
            }
            Some(message) = outbound.recv() => {
                if let Err(e) = sink.send(Message::Text(message)).await {
                    warn!("WS write error to {}: {}", src, e);
                    break;
                }
            }
        }
    }

    handler.unregister_stream(src);
    debug!("WS/WSS connection closed from {}", src);
    Ok(())
}

fn load_identity(cert_path: &str, key_path: &str) -> Result<Identity> {
    let cert_pem = std::fs::read(cert_path)
        .map_err(|e| anyhow!("Failed to read WSS cert '{}': {}", cert_path, e))?;
    let key_pem = std::fs::read(key_path)
        .map_err(|e| anyhow!("Failed to read WSS key '{}': {}", key_path, e))?;
    Identity::from_pkcs8(&cert_pem, &key_pem)
        .map_err(|e| anyhow!("Failed to create WSS identity: {}", e))
}
