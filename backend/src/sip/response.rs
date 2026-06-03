use super::message::SipMessage;
use anyhow::Result;

pub struct SipResponseBuilder {
    status_code: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl SipResponseBuilder {
    pub fn new(status_code: u16, reason: &str) -> Self {
        Self {
            status_code,
            reason: reason.to_string(),
            headers: Vec::new(),
            body: String::new(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn body(mut self, body: &str) -> Self {
        self.body = body.to_string();
        self
    }

    pub fn build(self) -> String {
        let content_len = self.body.len();
        let mut msg = format!("SIP/2.0 {} {}\r\n", self.status_code, self.reason);
        for (name, value) in &self.headers {
            msg.push_str(&format!("{}: {}\r\n", name, value));
        }
        msg.push_str(&format!("Content-Length: {}\r\n", content_len));
        msg.push_str("\r\n");
        msg.push_str(&self.body);
        msg
    }
}

pub fn base_response(req: &SipMessage, status_code: u16, reason: &str) -> SipResponseBuilder {
    let mut builder = SipResponseBuilder::new(status_code, reason);

    for via in req.via_headers() {
        builder = builder.header("Via", via);
    }
    if let Some(from) = req.from_header() {
        builder = builder.header("From", from);
    }
    if let Some(to) = req.to_header() {
        builder = builder.header("To", to);
    }
    if let Some(call_id) = req.call_id() {
        builder = builder.header("Call-ID", call_id);
    }
    if let Some(cseq) = req.cseq() {
        builder = builder.header("CSeq", cseq);
    }
    builder.header("Server", "SIP3/0.1.0")
}

pub fn finalize_response(
    msg: &SipMessage,
    response: Result<String>,
    method: &str,
) -> Result<Option<String>> {
    match response {
        Ok(resp) => Ok(Some(resp)),
        Err(e) => {
            tracing::warn!("Error handling {}: {}", method, e);
            Ok(Some(
                base_response(msg, 500, "Internal Server Error").build(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sip::message::SipMessage;

    fn parse_request(raw: &str) -> SipMessage {
        SipMessage::parse(raw).expect("parse failed")
    }

    #[test]
    fn test_response_builder_basic() {
        let response = SipResponseBuilder::new(200, "OK")
            .header("Via", "SIP/2.0/UDP 192.168.1.1")
            .header("Call-ID", "test-call")
            .build();

        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Via: SIP/2.0/UDP 192.168.1.1\r\n"));
        assert!(response.contains("Call-ID: test-call\r\n"));
        assert!(response.contains("Content-Length: 0\r\n"));
    }

    #[test]
    fn test_response_builder_with_body() {
        let sdp = "v=0\r\ns=-\r\nc=IN IP4 10.0.0.1\r\n";
        let response = SipResponseBuilder::new(200, "OK")
            .header("Content-Type", "application/sdp")
            .body(sdp)
            .build();

        assert!(response.contains("Content-Length: 29\r\n"));
        assert!(response.ends_with(sdp));
    }

    #[test]
    fn test_response_builder_chaining() {
        let response = SipResponseBuilder::new(180, "Ringing")
            .header("Via", "SIP/2.0/UDP 192.168.1.1")
            .header("Call-ID", "test-call")
            .build();

        assert!(response.starts_with("SIP/2.0 180 Ringing\r\n"));
    }

    #[test]
    fn test_base_response_copies_via() {
        let raw = "INVITE sip:1001@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKtest\r\n\
                   From: <sip:1000@sip.example.com>;tag=from-tag\r\n\
                   To: <sip:1001@sip.example.com>\r\n\
                   Call-ID: call-123\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let req = parse_request(raw);
        let response = base_response(&req, 100, "Trying").build();

        assert!(response.contains("Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKtest"));
        assert!(response.contains("From: <sip:1000@sip.example.com>;tag=from-tag"));
        assert!(response.contains("To: <sip:1001@sip.example.com>"));
        assert!(response.contains("Call-ID: call-123"));
        assert!(response.contains("CSeq: 1 INVITE"));
        assert!(response.contains("Server: SIP3/0.1.0"));
        assert!(response.starts_with("SIP/2.0 100 Trying\r\n"));
    }

    #[test]
    fn test_base_response_handles_missing_headers() {
        let raw = "INVITE sip:1001@sip.example.com SIP/2.0\r\n\
                   Call-ID: call-456\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let req = parse_request(raw);
        let response = base_response(&req, 200, "OK").build();

        assert!(response.contains("Call-ID: call-456"));
        assert!(response.contains("CSeq: 1 INVITE"));
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
    }

    #[test]
    fn test_finalize_response_passes_through_ok() {
        let raw = "REGISTER sip:sip.example.com SIP/2.0\r\n\
                   Call-ID: reg-123\r\n\
                   CSeq: 1 REGISTER\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let req = parse_request(raw);
        let result = finalize_response(&req, Ok("SIP/2.0 200 OK\r\n".to_string()), "REGISTER");

        assert!(result.is_ok());
        assert!(result.unwrap() == Some("SIP/2.0 200 OK\r\n".to_string()));
    }

    #[test]
    fn test_finalize_response_converts_err_to_500() {
        let raw = "REGISTER sip:sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.1\r\n\
                   From: <sip:1000@sip.example.com>;tag=t\r\n\
                   To: <sip:1000@sip.example.com>\r\n\
                   Call-ID: reg-456\r\n\
                   CSeq: 1 REGISTER\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let req = parse_request(raw);
        let result = finalize_response(&req, Err(anyhow::anyhow!("Database error")), "REGISTER");

        assert!(result.is_ok());
        let response = result.unwrap().unwrap();
        assert!(response.starts_with("SIP/2.0 500 Internal Server Error\r\n"));
    }

    #[test]
    fn test_response_builder_multiple_via_headers() {
        let response = SipResponseBuilder::new(200, "OK")
            .header("Via", "SIP/2.0/UDP proxy1.com;branch=1")
            .header("Via", "SIP/2.0/UDP 192.168.1.1;branch=2")
            .build();

        assert!(response.contains("Via: SIP/2.0/UDP proxy1.com;branch=1\r\n"));
        assert!(response.contains("Via: SIP/2.0/UDP 192.168.1.1;branch=2\r\n"));
    }

    #[test]
    fn test_response_builder_with_special_chars_in_header() {
        let response = SipResponseBuilder::new(200, "OK")
            .header(
                "WWW-Authenticate",
                r#"Digest realm="test\"realm", nonce="abc123""#,
            )
            .build();

        assert!(
            response.contains(r#"WWW-Authenticate: Digest realm="test\"realm", nonce="abc123""#)
        );
    }

    #[test]
    fn test_response_builder_empty_body() {
        let response = SipResponseBuilder::new(200, "OK")
            .header("Via", "SIP/2.0/UDP 192.168.1.1")
            .build();

        assert!(response.contains("Content-Length: 0\r\n"));
        assert!(response.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_base_response_with_multiple_vias() {
        let raw = "INVITE sip:1001@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP proxy1.com;branch=z9hG4bK1\r\n\
                   Via: SIP/2.0/UDP proxy2.com;branch=z9hG4bK2\r\n\
                   From: <sip:1000@sip.example.com>;tag=abc\r\n\
                   To: <sip:1001@sip.example.com>\r\n\
                   Call-ID: multi-via-req\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let req = parse_request(raw);
        let response = base_response(&req, 180, "Ringing").build();

        let vias: Vec<&str> = response.lines().filter(|l| l.starts_with("Via:")).collect();
        assert_eq!(vias.len(), 2);
    }

    #[test]
    fn test_response_builder_with_unicode_header_value() {
        let response = SipResponseBuilder::new(200, "OK")
            .header("X-Custom", "value with spaces")
            .build();

        assert!(response.contains("X-Custom: value with spaces\r\n"));
    }

    #[test]
    fn test_finalize_response_err_preserves_server_header() {
        let raw = "REGISTER sip:sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.1\r\n\
                   From: <sip:1000@sip.example.com>;tag=t\r\n\
                   To: <sip:1000@sip.example.com>\r\n\
                   Call-ID: err-test\r\n\
                   CSeq: 1 REGISTER\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let req = parse_request(raw);
        let result = finalize_response(&req, Err(anyhow::anyhow!("Error")), "REGISTER");

        let response = result.unwrap().unwrap();
        assert!(response.contains("Server: SIP3/0.1.0"));
    }
}
