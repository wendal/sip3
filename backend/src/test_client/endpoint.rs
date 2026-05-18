use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_native_tls::TlsStream;

#[derive(Debug, Clone)]
pub struct SipEndpointConfig {
    pub label: String,
    pub host: String,
    pub tls_port: u16,
    pub domain: String,
    pub realm: String,
    pub username: String,
    pub password: String,
    pub insecure_tls: bool,
}

pub async fn read_one_sip_message<R>(reader: &mut BufReader<R>) -> Result<Option<String>>
where
    R: AsyncRead + Unpin,
{
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(None);
        }

        let blank = line == "\r\n" || line == "\n";
        headers.push_str(&line);
        if blank {
            break;
        }
    }

    let content_length = headers
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-length:") {
                line.split(':').nth(1)?.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }

    Ok(Some(format!(
        "{}{}",
        headers,
        String::from_utf8_lossy(&body)
    )))
}

pub fn build_message_request(
    cfg: &SipEndpointConfig,
    to_username: &str,
    body: &str,
    call_id: &str,
    cseq: u32,
) -> String {
    format!(
        "MESSAGE sip:{to_username}@{domain} SIP/2.0\r\n\
         Via: SIP/2.0/TLS tester.invalid;branch=z9hG4bK-{call_id};rport\r\n\
         From: <sip:{from}@{domain}>;tag=tester\r\n\
         To: <sip:{to_username}@{domain}>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} MESSAGE\r\n\
         Contact: <sip:{from}@{domain};transport=tls>\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {len}\r\n\
         \r\n\
         {body}",
        domain = cfg.domain,
        from = cfg.username,
        len = body.len()
    )
}

#[derive(Debug, Clone)]
pub enum SipEvent {
    Registered,
    MessageReceived {
        from: String,
        body: String,
    },
    InviteReceived {
        call_id: String,
        from: String,
        sdp: String,
    },
    Ringing {
        call_id: String,
    },
    Answered {
        call_id: String,
        sdp: String,
    },
    AckReceived {
        call_id: String,
    },
    ByeReceived {
        call_id: String,
    },
    CancelReceived {
        call_id: String,
    },
    Ok {
        cseq_method: String,
    },
}

pub struct SipEndpoint {
    pub cfg: SipEndpointConfig,
    pub writer: WriteHalf<TlsStream<TcpStream>>,
    pub events: mpsc::UnboundedReceiver<SipEvent>,
}

impl SipEndpoint {
    pub async fn connect(cfg: SipEndpointConfig) -> Result<Self> {
        let stream = tokio::net::TcpStream::connect((cfg.host.as_str(), cfg.tls_port)).await?;
        let mut builder = native_tls::TlsConnector::builder();
        if cfg.insecure_tls {
            builder.danger_accept_invalid_certs(true);
            builder.danger_accept_invalid_hostnames(true);
        }
        let connector = tokio_native_tls::TlsConnector::from(builder.build()?);
        let tls_stream = connector.connect(&cfg.host, stream).await?;
        let (reader, writer) = tokio::io::split(tls_stream);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(reader);
            while let Ok(Some(raw)) = read_one_sip_message(&mut reader).await {
                if let Some(event) = parse_event(&raw) {
                    let _ = tx.send(event);
                }
            }
        });

        Ok(Self {
            cfg,
            writer,
            events: rx,
        })
    }
}

fn parse_event(raw: &str) -> Option<SipEvent> {
    if raw.starts_with("SIP/2.0 200") && raw.contains(" REGISTER") {
        return Some(SipEvent::Registered);
    }
    if raw.starts_with("SIP/2.0 180") && raw.contains(" INVITE") {
        return raw
            .lines()
            .find(|line| line.starts_with("Call-ID:"))
            .map(|line| SipEvent::Ringing {
                call_id: line.trim_start_matches("Call-ID:").trim().to_string(),
            });
    }
    if raw.starts_with("SIP/2.0 200") && raw.contains(" INVITE") {
        return raw
            .lines()
            .find(|line| line.starts_with("Call-ID:"))
            .map(|line| SipEvent::Answered {
                call_id: line.trim_start_matches("Call-ID:").trim().to_string(),
                sdp: raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
            });
    }
    if raw.starts_with("MESSAGE ") {
        return Some(SipEvent::MessageReceived {
            from: raw
                .lines()
                .find(|line| line.starts_with("From:"))
                .unwrap_or("From:")
                .to_string(),
            body: raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn read_one_sip_message_respects_content_length() {
        let (mut client, server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            client
                .write_all(b"SIP/2.0 200 OK\r\nContent-Length: 4\r\n\r\nbody")
                .await
                .unwrap();
        });

        let mut reader = BufReader::new(server);
        let message = read_one_sip_message(&mut reader)
            .await
            .expect("read message")
            .expect("some message");

        assert!(message.ends_with("\r\n\r\nbody"));
    }

    #[test]
    fn build_message_request_keeps_transport_tls() {
        let cfg = SipEndpointConfig {
            label: "caller".into(),
            host: "sip.air32.cn".into(),
            tls_port: 5061,
            domain: "sip.air32.cn".into(),
            realm: "sip.air32.cn".into(),
            username: "1001".into(),
            password: "secret".into(),
            insecure_tls: true,
        };

        let raw = build_message_request(&cfg, "1003", "hello", "call-1", 1);

        assert!(raw.starts_with("MESSAGE sip:1003@sip.air32.cn SIP/2.0\r\n"));
        assert!(raw.contains("Contact: <sip:1001@sip.air32.cn;transport=tls>\r\n"));
        assert!(raw.contains("Content-Length: 5\r\n\r\nhello"));
    }
}
