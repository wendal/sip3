use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, WriteHalf};
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
            if headers.is_empty() {
                return Ok(None);
            }
            return Err(anyhow::anyhow!("unexpected EOF while reading SIP headers"));
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

    pub async fn register(&mut self) -> anyhow::Result<()> {
        let call_id = format!("register-{}", self.cfg.username);
        let raw = format!(
            "REGISTER sip:{domain} SIP/2.0\r\n\
             Via: SIP/2.0/TLS tester.invalid;branch=z9hG4bK-{call_id};rport\r\n\
             From: <sip:{username}@{domain}>;tag=register\r\n\
             To: <sip:{username}@{domain}>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:{username}@{domain};transport=tls>\r\n\
             Expires: 300\r\n\
             Content-Length: 0\r\n\
             \r\n",
            domain = self.cfg.domain,
            username = self.cfg.username,
        );
        self.send_raw(&raw).await?;
        self.expect_event(
            "REGISTER",
            std::time::Duration::from_secs(5),
            |event| match event {
                SipEvent::Registered => true,
                SipEvent::Ok { cseq_method } => cseq_method == "REGISTER",
                _ => false,
            },
        )
        .await
    }

    pub async fn send_message(&mut self, to_username: &str, body: &str) -> anyhow::Result<()> {
        let raw = build_message_request(
            &self.cfg,
            to_username,
            body,
            &format!("message-{}-{}", self.cfg.username, to_username),
            1,
        );
        self.send_raw(&raw).await?;
        self.expect_event(
            "MESSAGE",
            std::time::Duration::from_secs(5),
            |event| match event {
                SipEvent::Ok { cseq_method } => cseq_method == "MESSAGE",
                _ => false,
            },
        )
        .await
    }

    async fn send_raw(&mut self, raw: &str) -> anyhow::Result<()> {
        self.writer.write_all(raw.as_bytes()).await?;
        Ok(())
    }

    async fn expect_event<F>(
        &mut self,
        label: &str,
        timeout: std::time::Duration,
        predicate: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(&SipEvent) -> bool,
    {
        let event = tokio::time::timeout(timeout, self.events.recv())
            .await
            .map_err(|_| anyhow::anyhow!("{label} timed out"))?
            .ok_or_else(|| anyhow::anyhow!("{label} channel closed"))?;

        if predicate(&event) {
            Ok(())
        } else {
            anyhow::bail!("{label} received unexpected event: {:?}", event);
        }
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
    use tokio::sync::mpsc;

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
        assert!(raw.contains("Via: SIP/2.0/TLS tester.invalid;branch=z9hG4bK-call-1;rport\r\n"));
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
    fn parse_event_401_register_becomes_auth_challenge() {
        match parse_event(
            "SIP/2.0 401 Unauthorized\r\nCSeq: 1 REGISTER\r\nCall-ID: reg-1\r\nWWW-Authenticate: Digest realm=\"sip.air32.cn\", nonce=\"abc123\", algorithm=MD5, qop=\"auth\"\r\nContent-Length: 0\r\n\r\n",
        ) {
            Some(SipEvent::AuthChallenge {
                cseq_method,
                auth_params,
            }) => {
                assert_eq!(cseq_method, "REGISTER");
                assert_eq!(auth_params.get("realm"), Some(&"sip.air32.cn".to_string()));
                assert_eq!(auth_params.get("nonce"), Some(&"abc123".to_string()));
                assert_eq!(auth_params.get("qop"), Some(&"auth".to_string()));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_event_180_invite_becomes_ringing() {
        match parse_event(
            "SIP/2.0 180 Ringing\r\nCSeq: 1 INVITE\r\nCall-ID: ring-1\r\nContent-Length: 0\r\n\r\n",
        ) {
            Some(SipEvent::Ringing { call_id }) => assert_eq!(call_id, "ring-1"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_event_200_invite_becomes_answered() {
        match parse_event(
            "SIP/2.0 200 OK\r\nCSeq: 1 INVITE\r\nCall-ID: ans-1\r\nContent-Type: application/sdp\r\nContent-Length: 7\r\n\r\nv=0\r\n\r\n",
        ) {
            Some(SipEvent::Answered { call_id, sdp }) => {
                assert_eq!(call_id, "ans-1");
                assert_eq!(sdp, "v=0\r\n\r\n");
            }
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
    fn parse_event_message_request_becomes_message_received() {
        match parse_event(
            "MESSAGE sip:1001@sip.air32.cn SIP/2.0\r\nFrom: <sip:1002@sip.air32.cn>;tag=x\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello",
        ) {
            Some(SipEvent::MessageReceived { from, body }) => {
                assert_eq!(from, "<sip:1002@sip.air32.cn>;tag=x");
                assert_eq!(body, "hello");
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

    #[test]
    fn build_register_authorization_uses_digest_auth_fields() {
        let cfg = SipEndpointConfig {
            label: "caller".into(),
            host: "sip.air32.cn".into(),
            tls_port: 5061,
            domain: "sip.air32.cn".into(),
            realm: "sip.air32.cn".into(),
            username: "1001".into(),
            password: "secret1".into(),
            insecure_tls: true,
        };

        let auth = build_register_authorization(
            &cfg,
            "abc123",
            "sip.air32.cn",
            "00000001",
            "deadbeef",
        );

        assert!(auth.starts_with("Digest "));
        assert!(auth.contains("username=\"1001\""));
        assert!(auth.contains("realm=\"sip.air32.cn\""));
        assert!(auth.contains("nonce=\"abc123\""));
        assert!(auth.contains("uri=\"sip:sip.air32.cn\""));
        assert!(auth.contains("qop=auth"));
        assert!(auth.contains("nc=00000001"));
        assert!(auth.contains("cnonce=\"deadbeef\""));
        assert!(auth.contains("response=\"40f1d2334df8ca4ff05f3bed0270116f\""));
    }

    #[tokio::test]
    async fn recv_matching_event_skips_unrelated_events() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(SipEvent::MessageReceived {
            from: "<sip:1002@sip.air32.cn>;tag=x".into(),
            body: "hello".into(),
        })
        .expect("queue unrelated event");
        tx.send(SipEvent::Ok {
            cseq_method: "MESSAGE".into(),
        })
        .expect("queue matching event");

        let event = recv_matching_event(&mut rx, "MESSAGE", std::time::Duration::from_secs(1), |event| {
            matches!(event, SipEvent::Ok { cseq_method } if cseq_method == "MESSAGE")
        })
        .await
        .expect("should skip unrelated event");

        match event {
            SipEvent::Ok { cseq_method } => assert_eq!(cseq_method, "MESSAGE"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_one_sip_message_errors_on_truncated_headers() {
        let (mut client, server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            client
                .write_all(b"SIP/2.0 200 OK\r\nContent-L")
                .await
                .unwrap();
        });

        let mut reader = BufReader::new(server);
        let result = read_one_sip_message(&mut reader).await;

        assert!(result.is_err(), "expected truncated headers to fail");
    }
}
