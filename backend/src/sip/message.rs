use std::collections::HashMap;
use anyhow::{Result, anyhow};

pub const SIP_ALLOW_METHODS: &str =
    "REGISTER, INVITE, ACK, BYE, CANCEL, OPTIONS, INFO, REFER, NOTIFY, SUBSCRIBE, MESSAGE";

#[derive(Debug, Clone)]
pub struct SipMessage {
    pub method: Option<String>,
    pub request_uri: Option<String>,
    pub status_code: Option<u16>,
    pub headers: HashMap<String, Vec<String>>,
    pub body: String,
    pub raw: String,
}

impl SipMessage {
    pub fn parse(raw: &str) -> Result<Self> {
        let mut lines = raw.lines();
        let first_line = lines.next().ok_or_else(|| anyhow!("Empty SIP message"))?;

        let mut method = None;
        let mut request_uri = None;
        let mut status_code = None;

        let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
        if parts.len() < 2 {
            return Err(anyhow!("Invalid SIP first line: {}", first_line));
        }

        if first_line.starts_with("SIP/2.0") {
            status_code = parts.get(1).and_then(|s| s.parse::<u16>().ok());
        } else {
            method = Some(parts[0].to_string());
            request_uri = parts.get(1).map(|s| s.to_string());
        }

        let mut headers: HashMap<String, Vec<String>> = HashMap::new();
        let mut in_headers = true;
        let mut body_lines: Vec<&str> = Vec::new();
        let mut last_key: Option<String> = None;

        for line in lines {
            if in_headers {
                if line.is_empty() {
                    in_headers = false;
                    continue;
                }
                if line.starts_with(' ') || line.starts_with('\t') {
                    if let Some(key) = &last_key
                        && let Some(vals) = headers.get_mut(key)
                        && let Some(last) = vals.last_mut()
                    {
                        last.push(' ');
                        last.push_str(line.trim());
                    }
                } else if let Some(colon_pos) = line.find(':') {
                    let name = normalize_header_name(&line[..colon_pos]);
                    let value = line[colon_pos + 1..].trim().to_string();
                    headers.entry(name.clone()).or_default().push(value);
                    last_key = Some(name);
                }
            } else {
                body_lines.push(line);
            }
        }

        Ok(SipMessage {
            method,
            request_uri,
            status_code,
            headers,
            body: body_lines.join("\r\n"),
            raw: raw.to_string(),
        })
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let key = normalize_header_name(name);
        self.headers
            .get(&key)
            .and_then(|v| v.first())
            .map(|s| s.as_str())
    }

    pub fn all_headers_vec(&self, name: &str) -> Vec<&str> {
        let key = normalize_header_name(name);
        self.headers
            .get(&key)
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    pub fn call_id(&self) -> Option<&str> {
        self.header("call-id")
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn from_header(&self) -> Option<&str> {
        self.header("from")
    }

    pub fn to_header(&self) -> Option<&str> {
        self.header("to")
    }

    pub fn contact(&self) -> Option<&str> {
        self.header("contact")
    }

    pub fn via_headers(&self) -> Vec<&str> {
        self.all_headers_vec("via")
    }

    pub fn cseq(&self) -> Option<&str> {
        self.header("cseq")
    }

    pub fn expires(&self) -> Option<u32> {
        self.header("expires").and_then(|s| s.parse().ok())
    }

    pub fn user_agent(&self) -> Option<&str> {
        self.header("user-agent")
    }

    pub fn authorization(&self) -> Option<&str> {
        self.header("authorization")
    }
}

pub fn normalize_header_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    match lower.as_str() {
        "f" => "from".to_string(),
        "t" => "to".to_string(),
        "v" => "via".to_string(),
        "c" => "content-type".to_string(),
        "l" => "content-length".to_string(),
        "i" => "call-id".to_string(),
        "m" => "contact".to_string(),
        "e" => "content-encoding".to_string(),
        other => other.to_string(),
    }
}

pub fn extract_uri(addr: &str) -> Option<String> {
    if let Some(start) = addr.find('<')
        && let Some(end_rel) = addr[start..].find('>')
    {
        return Some(addr[start + 1..start + end_rel].trim().to_string());
    }
    let uri = addr.split(';').next().unwrap_or(addr).trim();
    if uri.starts_with("sip:") || uri.starts_with("sips:") || uri.starts_with("tel:") {
        Some(uri.to_string())
    } else {
        None
    }
}

pub fn uri_username(uri: &str) -> Option<String> {
    let without_scheme = uri.trim_start_matches("sip:").trim_start_matches("sips:");
    if without_scheme.contains('@') {
        Some(without_scheme.split('@').next()?.to_string())
    } else {
        None
    }
}

pub fn uri_host(uri: &str) -> Option<String> {
    let without_scheme = uri.trim_start_matches("sip:").trim_start_matches("sips:");
    let part = if without_scheme.contains('@') {
        without_scheme.split('@').nth(1).unwrap_or(without_scheme)
    } else {
        without_scheme
    };
    let host = part.split(';').next().unwrap_or(part);
    let host = host.split('?').next().unwrap_or(host);
    Some(host.to_string())
}

pub fn parse_auth_params(header: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let rest = match header.find(' ') {
        Some(pos) => header[pos + 1..].trim(),
        None => return params,
    };

    let mut pos = 0;
    let bytes = rest.as_bytes();
    let len = bytes.len();

    while pos < len {
        while pos < len && (bytes[pos] == b',' || bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= len {
            break;
        }

        let key_start = pos;
        while pos < len && bytes[pos] != b'=' {
            pos += 1;
        }
        if pos >= len {
            break;
        }
        let key = rest[key_start..pos].trim().to_lowercase();
        pos += 1;

        if pos < len && bytes[pos] == b'"' {
            pos += 1;
            let val_start = pos;
            while pos < len && bytes[pos] != b'"' {
                if bytes[pos] == b'\\' {
                    pos += 1;
                }
                pos += 1;
            }
            let val = rest[val_start..pos].to_string();
            if pos < len {
                pos += 1;
            }
            params.insert(key, val);
        } else {
            let val_start = pos;
            while pos < len && bytes[pos] != b',' {
                pos += 1;
            }
            let val = rest[val_start..pos].trim().to_string();
            params.insert(key, val);
        }
    }

    params
}

pub fn make_www_authenticate(realm: &str, nonce: &str) -> String {
    format!(
        r#"Digest realm="{}", nonce="{}", algorithm=MD5, qop="auth""#,
        realm, nonce
    )
}

pub fn md5_hex(input: &str) -> String {
    format!("{:x}", md5::compute(input.as_bytes()))
}

pub fn strip_proxy_via(raw: &str, sip_domain: &str) -> String {
    let mut removed = false;
    let mut lines: Vec<&str> = Vec::new();
    for line in raw.split("\r\n") {
        if !removed {
            let lower = line.trim_start().to_lowercase();
            if (lower.starts_with("via:") || lower.starts_with("v:")) && line.contains(sip_domain) {
                removed = true;
                continue;
            }
        }
        lines.push(line);
    }
    lines.join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_register_request() {
        let raw = "REGISTER sip:sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK776asdhds\r\n\
                   From: Alice <sip:alice@sip.example.com>;tag=1928301774\r\n\
                   To: Alice <sip:alice@sip.example.com>\r\n\
                   Call-ID: a84b4c76e66710@192.168.1.100\r\n\
                   CSeq: 314159 REGISTER\r\n\
                   Contact: <sip:alice@192.168.1.100:5060>\r\n\
                   Expires: 3600\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("REGISTER"));
        assert!(msg.header("via").is_some());
        assert!(msg.from_header().is_some());
        assert!(msg.to_header().is_some());
        assert_eq!(msg.call_id(), Some("a84b4c76e66710@192.168.1.100"));
        assert_eq!(msg.expires(), Some(3600));
    }

    #[test]
    fn test_parse_invite_request() {
        let raw = "INVITE sip:bob@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK74bf9\r\n\
                   From: Alice <sip:alice@sip.example.com>;tag=9fxced76sl\r\n\
                   To: Bob <sip:bob@sip.example.com>\r\n\
                   Call-ID: 3848276298220188511@192.168.1.100\r\n\
                   CSeq: 314159 INVITE\r\n\
                   Contact: <sip:alice@192.168.1.100:5060>\r\n\
                   Content-Type: application/sdp\r\n\
                   Content-Length: 4\r\n\
                   \r\n\
                   Test";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("INVITE"));
        assert_eq!(msg.request_uri.as_deref(), Some("sip:bob@sip.example.com"));
        assert!(msg.header("content-type").is_some());
        assert_eq!(msg.body, "Test");
    }

    #[test]
    fn test_parse_sip_response() {
        let raw = "SIP/2.0 180 Ringing\r\n\
                   Via: SIP/2.0/UDP sip.example.com;branch=z9hG4bKproxy123\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKorig\r\n\
                   From: Alice <sip:alice@sip.example.com>;tag=abc\r\n\
                   To: Bob <sip:bob@sip.example.com>;tag=xyz\r\n\
                   Call-ID: test-call@192.168.1.100\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert!(msg.method.is_none());
        assert_eq!(msg.status_code, Some(180));
        assert_eq!(msg.via_headers().len(), 2);
    }

    #[test]
    fn test_parse_message_request() {
        let raw = "MESSAGE sip:1002@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKmsg\r\n\
                   From: <sip:1001@sip.example.com>;tag=msg-1\r\n\
                   To: <sip:1002@sip.example.com>\r\n\
                   Call-ID: msg-call-id@192.168.1.100\r\n\
                   CSeq: 1 MESSAGE\r\n\
                   Content-Type: text/plain\r\n\
                   Content-Length: 5\r\n\
                   \r\n\
                   hello";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("MESSAGE"));
        assert_eq!(msg.request_uri.as_deref(), Some("sip:1002@sip.example.com"));
        assert_eq!(msg.header("content-type"), Some("text/plain"));
        assert_eq!(msg.body, "hello");
    }

    #[test]
    fn test_parse_subscribe_request() {
        let raw = "SUBSCRIBE sip:alice@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKsub\r\n\
                   From: <sip:bob@sip.example.com>;tag=blf-tag\r\n\
                   To: <sip:alice@sip.example.com>\r\n\
                   Call-ID: sub-call-id@192.168.1.100\r\n\
                   CSeq: 1 SUBSCRIBE\r\n\
                   Event: presence\r\n\
                   Expires: 300\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("SUBSCRIBE"));
        assert_eq!(msg.header("event"), Some("presence"));
        assert_eq!(msg.expires(), Some(300));
        assert_eq!(msg.call_id(), Some("sub-call-id@192.168.1.100"));
    }

    #[test]
    fn test_parse_empty_message() {
        let result = SipMessage::parse("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_first_line() {
        let result = SipMessage::parse("INVALID LINE\r\n\r\n");
        assert!(result.is_ok());
        let msg = result.unwrap();
        assert_eq!(msg.method.as_deref(), Some("INVALID"));
        assert_eq!(msg.request_uri.as_deref(), Some("LINE"));
    }

    #[test]
    fn test_parse_header_folding() {
        let raw = "INVITE sip:test@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.1\r\n\
                   From: <sip:alice@sip.example.com>\r\n\
                   Contact: <sip:alice@192.168.1.1>;expires=3600\r\n\
                   \x20;q=0.5\r\n\
                   Call-ID: test-fold\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        let contact = msg.contact().expect("contact should exist");
        assert!(contact.contains("expires=3600"));
        assert!(contact.contains("q=0.5"));
    }

    #[test]
    fn test_parse_bye_request() {
        let raw = "BYE sip:1001@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060\r\n\
                   From: <sip:1000@sip.example.com>;tag=abc\r\n\
                   To: <sip:1001@sip.example.com>;tag=xyz\r\n\
                   Call-ID: bye-call\r\n\
                   CSeq: 2 BYE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("BYE"));
        assert_eq!(msg.cseq(), Some("2 BYE"));
    }

    #[test]
    fn test_parse_cancel_request() {
        let raw = "CANCEL sip:1001@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060\r\n\
                   From: <sip:1000@sip.example.com>;tag=abc\r\n\
                   To: <sip:1001@sip.example.com>\r\n\
                   Call-ID: cancel-call\r\n\
                   CSeq: 1 CANCEL\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("CANCEL"));
    }

    #[test]
    fn test_parse_options_request() {
        let raw = "OPTIONS sip:proxy@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060\r\n\
                   From: <sip:1000@sip.example.com>;tag=opt\r\n\
                   To: <sip:proxy@sip.example.com>\r\n\
                   Call-ID: options-call\r\n\
                   CSeq: 1 OPTIONS\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("OPTIONS"));
    }

    #[test]
    fn test_parse_refer_request() {
        let raw = "REFER sip:1001@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060\r\n\
                   From: <sip:1000@sip.example.com>;tag=ref\r\n\
                   To: <sip:1001@sip.example.com>\r\n\
                   Call-ID: refer-call\r\n\
                   CSeq: 1 REFER\r\n\
                   Refer-To: <sip:1002@sip.example.com>\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("REFER"));
        assert_eq!(msg.header("refer-to").is_some(), true);
    }

    #[test]
    fn test_parse_notify_request() {
        let raw = "NOTIFY sip:1000@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060\r\n\
                   From: <sip:1001@sip.example.com>;tag=nfy\r\n\
                   To: <sip:1000@sip.example.com>\r\n\
                   Call-ID: notify-call\r\n\
                   CSeq: 1 NOTIFY\r\n\
                   Event: refer\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("NOTIFY"));
        assert_eq!(msg.header("event"), Some("refer"));
    }

    #[test]
    fn test_parse_ack_request() {
        let raw = "ACK sip:1001@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060\r\n\
                   From: <sip:1000@sip.example.com>;tag=ack\r\n\
                   To: <sip:1001@sip.example.com>\r\n\
                   Call-ID: ack-call\r\n\
                   CSeq: 1 ACK\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert_eq!(msg.method.as_deref(), Some("ACK"));
    }

    #[test]
    fn test_parse_response_with_sdp() {
        let raw = "SIP/2.0 200 OK\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060\r\n\
                   From: <sip:1000@sip.example.com>;tag=abc\r\n\
                   To: <sip:1001@sip.example.com>;tag=xyz\r\n\
                   Call-ID: 200ok-call\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Type: application/sdp\r\n\
                   Content-Length: 26\r\n\
                   \r\n\
                   v=0\r\ns=-\r\nc=IN IP4 10.0.0.1\r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert!(msg.method.is_none());
        assert_eq!(msg.status_code, Some(200));
        assert!(msg.body.contains("v=0"));
    }

    #[test]
    fn test_normalize_header_name() {
        assert_eq!(normalize_header_name("f"), "from");
        assert_eq!(normalize_header_name("v"), "via");
        assert_eq!(normalize_header_name("i"), "call-id");
        assert_eq!(normalize_header_name("Via"), "via");
        assert_eq!(normalize_header_name("Content-Length"), "content-length");
        assert_eq!(normalize_header_name("F"), "from");
        assert_eq!(normalize_header_name("C"), "content-type");
        assert_eq!(normalize_header_name("L"), "content-length");
        assert_eq!(normalize_header_name("M"), "contact");
        assert_eq!(normalize_header_name("E"), "content-encoding");
    }

    #[test]
    fn test_extract_uri_from_address_with_angle_brackets() {
        assert_eq!(
            extract_uri("Alice <sip:alice@sip.example.com>"),
            Some("sip:alice@sip.example.com".to_string())
        );
        assert_eq!(
            extract_uri("<sip:bob@192.168.1.1:5060>"),
            Some("sip:bob@192.168.1.1:5060".to_string())
        );
    }

    #[test]
    fn test_extract_uri_from_plain_uri() {
        assert_eq!(
            extract_uri("sip:charlie@sip.example.com"),
            Some("sip:charlie@sip.example.com".to_string())
        );
    }

    #[test]
    fn test_extract_uri_with_parameters() {
        assert_eq!(
            extract_uri("<sip:alice@proxy.com;transport=tls>"),
            Some("sip:alice@proxy.com;transport=tls".to_string())
        );
    }

    #[test]
    fn test_extract_uri_rejects_non_sip() {
        assert_eq!(extract_uri("tel:+1234567890"), Some("tel:+1234567890".to_string()));
        assert_eq!(extract_uri("invalid"), None);
    }

    #[test]
    fn test_uri_username_extraction() {
        assert_eq!(uri_username("sip:alice@sip.example.com"), Some("alice".to_string()));
        assert_eq!(uri_username("sip:bob@192.168.1.1:5060"), Some("bob".to_string()));
        assert_eq!(
            uri_username("sip:charlie@domain.com;transport=udp"),
            Some("charlie".to_string())
        );
        assert_eq!(uri_username("sip:sip.example.com"), None);
        assert_eq!(uri_username("sip:@example.com"), Some("".to_string()));
    }

    #[test]
    fn test_uri_host_extraction() {
        assert_eq!(uri_host("sip:alice@sip.example.com"), Some("sip.example.com".to_string()));
        assert_eq!(uri_host("sip:bob@192.168.1.1:5060"), Some("192.168.1.1:5060".to_string()));
        assert_eq!(
            uri_host("sip:charlie@proxy.com;transport=udp"),
            Some("proxy.com".to_string())
        );
    }

    #[test]
    fn test_parse_auth_params_basic() {
        let auth_header = r#"Digest username="alice", realm="sip.example.com", nonce="abcdef123456", uri="sip:sip.example.com", response="abc123def456""#;
        let params = parse_auth_params(auth_header);

        assert_eq!(params.get("username").map(|s| s.as_str()), Some("alice"));
        assert_eq!(params.get("realm").map(|s| s.as_str()), Some("sip.example.com"));
        assert_eq!(params.get("nonce").map(|s| s.as_str()), Some("abcdef123456"));
        assert_eq!(params.get("uri").map(|s| s.as_str()), Some("sip:sip.example.com"));
        assert_eq!(params.get("response").map(|s| s.as_str()), Some("abc123def456"));
    }

    #[test]
    fn test_parse_auth_params_with_quoted_values() {
        let auth_header = r#"Digest realm="test realm", nonce="abc\"def", algorithm=MD5"#;
        let params = parse_auth_params(auth_header);

        assert_eq!(params.get("realm").map(|s| s.as_str()), Some("test realm"));
        assert_eq!(params.get("nonce").map(|s| s.as_str()), Some(r#"abc\"def"#));
        assert_eq!(params.get("algorithm").map(|s| s.as_str()), Some("MD5"));
    }

    #[test]
    fn test_parse_auth_params_empty_header() {
        let params = parse_auth_params("");
        assert!(params.is_empty());

        let params = parse_auth_params("Digest");
        assert!(params.is_empty());
    }

    #[test]
    fn test_parse_auth_params_unquoted_values() {
        let header = "Digest username=alice, realm=sip.example.com";
        let params = parse_auth_params(header);

        assert_eq!(params.get("username").map(|s| s.as_str()), Some("alice"));
        assert_eq!(params.get("realm").map(|s| s.as_str()), Some("sip.example.com"));
    }

    #[test]
    fn test_md5_hex_deterministic() {
        let input = "test input";
        let result1 = md5_hex(input);
        let result2 = md5_hex(input);
        assert_eq!(result1, result2);
        assert_eq!(result1.len(), 32);
    }

    #[test]
    fn test_md5_hex_digest_auth() {
        let username = "alice";
        let realm = "sip.example.com";
        let password = "secret";
        let method = "REGISTER";
        let uri = "sip:sip.example.com";

        let ha1 = md5_hex(&format!("{}:{}:{}", username, realm, password));
        let ha2 = md5_hex(&format!("{}:{}", method, uri));

        assert_eq!(ha1.len(), 32);
        assert_eq!(ha2.len(), 32);
        assert!(ha1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_make_www_authenticate() {
        let challenge = make_www_authenticate("sip.example.com", "testnonce123");
        assert!(challenge.contains(r#"Digest realm="sip.example.com""#));
        assert!(challenge.contains(r#"nonce="testnonce123""#));
        assert!(challenge.contains("algorithm=MD5"));
        assert!(challenge.contains(r#"qop="auth""#));
    }

    #[test]
    fn test_strip_proxy_via_removes_domain_via() {
        let raw = "SIP/2.0 180 Ringing\r\n\
                   Via: SIP/2.0/UDP sip.example.com;branch=z9hG4bKproxy42\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKorig\r\n\
                   From: Alice\r\n\
                   To: Bob\r\n\
                   Call-ID: test@192.168.1.100\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let stripped = strip_proxy_via(raw, "sip.example.com");

        assert!(!stripped.contains("z9hG4bKproxy42"));
        assert!(stripped.contains("z9hG4bKorig"));
        assert!(stripped.contains("From: Alice"));
    }

    #[test]
    fn test_strip_proxy_via_preserves_multiple_vias() {
        let raw = "SIP/2.0 200 OK\r\n\
                   Via: SIP/2.0/UDP proxy1.com;branch=z9hG4bK1\r\n\
                   Via: SIP/2.0/UDP proxy2.com;branch=z9hG4bK2\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKorig\r\n\
                   Call-ID: test\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let stripped = strip_proxy_via(raw, "proxy1.com");

        assert!(!stripped.contains("z9hG4bK1"));
        assert!(stripped.contains("z9hG4bK2"));
        assert!(stripped.contains("z9hG4bKorig"));
    }

    #[test]
    fn test_strip_proxy_via_no_match_leaves_intact() {
        let raw = "SIP/2.0 200 OK\r\n\
                   Via: SIP/2.0/UDP other.domain.com;branch=z9hG4bK123\r\n\
                   Call-ID: test\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let stripped = strip_proxy_via(raw, "sip.example.com");

        assert_eq!(stripped, raw);
    }

    #[test]
    fn test_parse_response_with_multiple_via_headers() {
        let raw = "SIP/2.0 200 OK\r\n\
                   Via: SIP/2.0/UDP proxy1.com;branch=z9hG4bK1\r\n\
                   Via: SIP/2.0/UDP proxy2.com;branch=z9hG4bK2\r\n\
                   From: <sip:alice@example.com>;tag=abc\r\n\
                   To: <sip:bob@example.com>;tag=xyz\r\n\
                   Call-ID: multi-via-test\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        let vias = msg.via_headers();
        assert_eq!(vias.len(), 2);
        assert!(vias[0].contains("proxy1.com"));
        assert!(vias[1].contains("proxy2.com"));
    }

    #[test]
    fn test_parse_message_with_sdp_body() {
        let raw = "INVITE sip:1001@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060\r\n\
                   From: <sip:1000@sip.example.com>;tag=abc\r\n\
                   To: <sip:1001@sip.example.com>\r\n\
                   Call-ID: sdp-test\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Type: application/sdp\r\n\
                   Content-Length: 30\r\n\
                   \r\n\
                   v=0\r\n\
                   o=user 1 1 IN IP4 0.0.0.0\r\n\
                   s=-\r\n                   c=IN IP4 0.0.0.0\r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert!(msg.body.contains("v=0"));
        assert!(msg.body.contains("o=user"));
        assert!(msg.body.contains("c=IN IP4 0.0.0.0"));
    }

    #[test]
    fn test_normalize_header_name_short_forms() {
        assert_eq!(normalize_header_name("k"), "k".to_string());
        assert_eq!(normalize_header_name("t"), "to".to_string());
        assert_eq!(normalize_header_name("s"), "s".to_string());
    }

    #[test]
    fn test_md5_hex_empty_string() {
        let result = md5_hex("");
        assert_eq!(result.len(), 32);
        assert_eq!(result, "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn test_parse_auth_params_with_escaped_quotes() {
        let auth_header = r#"Digest realm="test\"realm", nonce="abc\"def\"ghi", algorithm=MD5"#;
        let params = parse_auth_params(auth_header);

        assert_eq!(params.get("realm").map(|s| s.as_str()), Some(r#"test\"realm"#));
        assert_eq!(params.get("nonce").map(|s| s.as_str()), Some(r#"abc\"def\"ghi"#));
        assert_eq!(params.get("algorithm").map(|s| s.as_str()), Some("MD5"));
    }

    #[test]
    fn test_parse_auth_params_with_comma_in_quoted_value() {
        let auth_header = r#"Digest realm="test, realm", nonce="abc,def", uri="sip:test,test.com""#;
        let params = parse_auth_params(auth_header);

        assert_eq!(params.get("realm").map(|s| s.as_str()), Some("test, realm"));
        assert_eq!(params.get("nonce").map(|s| s.as_str()), Some("abc,def"));
        assert_eq!(params.get("uri").map(|s| s.as_str()), Some("sip:test,test.com"));
    }

    #[test]
    fn test_uri_username_with_sips() {
        assert_eq!(uri_username("sips:alice@sip.example.com"), Some("alice".to_string()));
        assert_eq!(uri_username("sips:bob@sip.example.com"), Some("bob".to_string()));
    }

    #[test]
    fn test_uri_host_with_port_and_params() {
        assert_eq!(uri_host("sip:alice@192.168.1.1:5060;transport=tls"), Some("192.168.1.1:5060".to_string()));
    }

    #[test]
    fn test_extract_uri_from_sip_uri_with_port() {
        assert_eq!(
            extract_uri("Alice <sip:alice@192.168.1.1:5060>"),
            Some("sip:alice@192.168.1.1:5060".to_string())
        );
    }

    #[test]
    fn test_make_www_authenticate_with_special_chars() {
        let www = make_www_authenticate("test realm", "abc123");
        assert!(www.contains(r#"realm="test realm""#));
        assert!(www.contains(r#"nonce="abc123""#));
        assert!(www.contains("algorithm=MD5"));
        assert!(www.contains(r#"qop="auth""#));
    }

    #[test]
    fn test_strip_proxy_via_removes_first_matching() {
        let raw = "INVITE sip:test@sip.example.com SIP/2.0\r\n\
                   Via: SIP/2.0/UDP sip.example.com;branch=z9hG4bKfirst\r\n\
                   Via: SIP/2.0/UDP other.com;branch=z9hG4bKsecond\r\n\
                   Call-ID: strip-first\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let stripped = strip_proxy_via(raw, "sip.example.com");

        assert!(!stripped.contains("z9hG4bKfirst"));
        assert!(stripped.contains("z9hG4bKsecond"));
    }
}
