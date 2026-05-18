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
    let call_id = header_value_case_insensitive(raw, "Call-ID").unwrap_or_default();
    let from = header_value_case_insensitive(raw, "From").unwrap_or_default();
    let cseq = header_value_case_insensitive(raw, "CSeq").unwrap_or_default();
    let cseq_method = cseq.split_whitespace().last().unwrap_or("").to_string();
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_default();

    if raw.starts_with("SIP/2.0 200") {
        if cseq_method == "REGISTER" {
            return Some(SipEvent::Registered);
        }
        if cseq_method == "INVITE" {
            return Some(SipEvent::Answered { call_id, sdp: body });
        }
        if !cseq_method.is_empty() {
            return Some(SipEvent::Ok { cseq_method });
        }
    }

    if raw.starts_with("SIP/2.0 180") && cseq_method == "INVITE" {
        return Some(SipEvent::Ringing { call_id });
    }

    if raw.starts_with("INVITE ") {
        return Some(SipEvent::InviteReceived {
            call_id,
            from,
            sdp: body,
        });
    }

    if raw.starts_with("ACK ") {
        return Some(SipEvent::AckReceived { call_id });
    }

    if raw.starts_with("BYE ") {
        return Some(SipEvent::ByeReceived { call_id });
    }

    if raw.starts_with("CANCEL ") {
        return Some(SipEvent::CancelReceived { call_id });
    }

    if raw.starts_with("MESSAGE ") {
        return Some(SipEvent::MessageReceived { from, body });
    }

    None
}

fn header_value_case_insensitive(raw: &str, name: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        let (header_name, value) = line.split_once(':')?;
        header_name
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
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

    #[test]
    fn parse_event_200_register_becomes_registered() {
        assert!(matches!(
            parse_event(
                "SIP/2.0 200 OK\r\nCSeq: 1 REGISTER\r\nCall-ID: reg-1\r\nContent-Length: 0\r\n\r\n"
            ),
            Some(SipEvent::Registered)
        ));
    }

    #[test]
    fn parse_event_200_message_becomes_ok() {
        match parse_event(
            "SIP/2.0 200 OK\r\nCSeq: 7 MESSAGE\r\nCall-ID: msg-1\r\nContent-Length: 0\r\n\r\n",
        ) {
            Some(SipEvent::Ok { cseq_method }) => assert_eq!(cseq_method, "MESSAGE"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_event_invite_request_becomes_invite_received() {
        match parse_event(
            "INVITE sip:1001@sip.air32.cn SIP/2.0\r\nCall-ID: inv-1\r\nFrom: <sip:1002@sip.air32.cn>;tag=x\r\nContent-Type: application/sdp\r\nContent-Length: 7\r\n\r\nv=0\r\n\r\n",
        ) {
            Some(SipEvent::InviteReceived { call_id, from, sdp }) => {
                assert_eq!(call_id, "inv-1");
                assert_eq!(from, "<sip:1002@sip.air32.cn>;tag=x");
                assert_eq!(sdp, "v=0\r\n\r\n");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_event_ack_request_becomes_ack_received() {
        match parse_event("ACK sip:1001@sip.air32.cn SIP/2.0\r\nCall-ID: ack-1\r\n\r\n") {
            Some(SipEvent::AckReceived { call_id }) => assert_eq!(call_id, "ack-1"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_event_bye_request_becomes_bye_received() {
        match parse_event("BYE sip:1001@sip.air32.cn SIP/2.0\r\nCall-ID: bye-1\r\n\r\n") {
            Some(SipEvent::ByeReceived { call_id }) => assert_eq!(call_id, "bye-1"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_event_cancel_request_becomes_cancel_received() {
        match parse_event("CANCEL sip:1001@sip.air32.cn SIP/2.0\r\nCall-ID: cancel-1\r\n\r\n") {
            Some(SipEvent::CancelReceived { call_id }) => assert_eq!(call_id, "cancel-1"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_event_handles_lowercase_header_names_case_insensitively() {
        match parse_event(
            "INVITE sip:1001@sip.air32.cn SIP/2.0\r\ncall-id: lc-1\r\nfrom: <sip:1002@sip.air32.cn>;tag=x\r\ncseq: 1 INVITE\r\nContent-Length: 7\r\n\r\nv=0\r\n\r\n",
        ) {
            Some(SipEvent::InviteReceived { call_id, from, sdp }) => {
                assert_eq!(call_id, "lc-1");
                assert_eq!(from, "<sip:1002@sip.air32.cn>;tag=x");
                assert_eq!(sdp, "v=0\r\n\r\n");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
