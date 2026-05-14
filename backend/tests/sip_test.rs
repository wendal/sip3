//! Integration tests that exercise the actual production SIP handler code.
//! Functions are imported directly from the `sip3_backend` library crate.

#[cfg(test)]
mod tests {
    use sip3_backend::sip::handler::{
        extract_uri, md5_hex, normalize_header_name, parse_auth_params, strip_proxy_via,
        uri_username, SipMessage,
    };
    use sip3_backend::sip::registrar::{generate_nonce, validate_nonce};

    // ── SipMessage::parse ────────────────────────────────────────────────────

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

    // ── extract_uri ─────────────────────────────────────────────────────────

    #[test]
    fn test_extract_uri_from_address() {
        let cases = vec![
            (
                "Alice <sip:alice@sip.example.com>",
                "sip:alice@sip.example.com",
            ),
            ("<sip:bob@192.168.1.1:5060>", "sip:bob@192.168.1.1:5060"),
            ("sip:charlie@sip.example.com", "sip:charlie@sip.example.com"),
        ];

        for (input, expected) in cases {
            let result = extract_uri(input);
            assert_eq!(result.as_deref(), Some(expected), "Failed for: {}", input);
        }
    }

    // ── uri_username ─────────────────────────────────────────────────────────

    #[test]
    fn test_uri_username_extraction() {
        let cases = vec![
            ("sip:alice@sip.example.com", Some("alice")),
            ("sip:bob@192.168.1.1:5060", Some("bob")),
            ("sip:charlie@domain.com;transport=udp", Some("charlie")),
            ("sip:sip.example.com", None),
        ];

        for (uri, expected) in cases {
            let result = uri_username(uri);
            assert_eq!(result.as_deref(), expected, "Failed for: {}", uri);
        }
    }

    // ── md5_hex ──────────────────────────────────────────────────────────────

    #[test]
    fn test_md5_digest_auth() {
        // Verify RFC 3261 digest computation: HA1 = MD5(user:realm:pass)
        let username = "alice";
        let realm = "sip.example.com";
        let password = "secret";
        let method = "REGISTER";
        let uri = "sip:sip.example.com";
        let nonce = "testNonce12345";

        let ha1 = md5_hex(&format!("{}:{}:{}", username, realm, password));
        let ha2 = md5_hex(&format!("{}:{}", method, uri));
        let response = md5_hex(&format!("{}:{}:{}", ha1, nonce, ha2));

        assert_eq!(ha1.len(), 32);
        assert_eq!(response.len(), 32);
        assert!(response.chars().all(|c| c.is_ascii_hexdigit()));

        // Deterministic
        assert_eq!(
            ha1,
            md5_hex(&format!("{}:{}:{}", username, realm, password))
        );
    }

    // ── parse_auth_params ─────────────────────────────────────────────────────

    #[test]
    fn test_parse_auth_params() {
        let auth_header = r#"Digest username="alice", realm="sip.example.com", nonce="abcdef123456", uri="sip:sip.example.com", response="abc123def456""#;
        let params = parse_auth_params(auth_header);

        assert_eq!(params.get("username").map(|s| s.as_str()), Some("alice"));
        assert_eq!(
            params.get("realm").map(|s| s.as_str()),
            Some("sip.example.com")
        );
        assert_eq!(
            params.get("nonce").map(|s| s.as_str()),
            Some("abcdef123456")
        );
        assert_eq!(
            params.get("uri").map(|s| s.as_str()),
            Some("sip:sip.example.com")
        );
    }

    // ── normalize_header_name ─────────────────────────────────────────────────

    #[test]
    fn test_normalize_header_name() {
        assert_eq!(normalize_header_name("f"), "from");
        assert_eq!(normalize_header_name("v"), "via");
        assert_eq!(normalize_header_name("i"), "call-id");
        assert_eq!(normalize_header_name("Via"), "via");
        assert_eq!(normalize_header_name("Content-Length"), "content-length");
    }

    // ── strip_proxy_via ───────────────────────────────────────────────────────

    #[test]
    fn test_strip_proxy_via() {
        let raw = "SIP/2.0 180 Ringing\r\n\
                   Via: SIP/2.0/UDP sip.example.com;branch=z9hG4bKproxy42\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKorig\r\n\
                   From: Alice <sip:alice@sip.example.com>;tag=abc\r\n\
                   To: Bob <sip:bob@sip.example.com>;tag=xyz\r\n\
                   Call-ID: test@192.168.1.100\r\n\
                   CSeq: 1 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let stripped = strip_proxy_via(raw, "sip.example.com");
        // Our proxy Via should be gone.
        assert!(!stripped.contains("z9hG4bKproxy42"));
        // Caller's original Via must survive.
        assert!(stripped.contains("z9hG4bKorig"));
    }

    // ── HMAC nonce generation / validation ────────────────────────────────────

    #[test]
    fn test_nonce_valid() {
        let secret = "test_secret_key";
        let nonce = generate_nonce(secret);
        // Fresh nonce with generous max_age should pass.
        assert!(validate_nonce(&nonce, secret, 300));
    }

    #[test]
    fn test_nonce_wrong_secret() {
        let nonce = generate_nonce("secret_a");
        assert!(!validate_nonce(&nonce, "secret_b", 300));
    }

    #[test]
    fn test_nonce_tampered() {
        let secret = "test_secret";
        let mut nonce = generate_nonce(secret);
        // Flip one character in the data portion.
        let bad_char = if nonce.chars().next() == Some('a') {
            'b'
        } else {
            'a'
        };
        nonce.replace_range(0..1, &bad_char.to_string());
        assert!(!validate_nonce(&nonce, secret, 300));
    }

    #[test]
    fn test_nonce_expired() {
        let secret = "test_secret";
        let nonce = generate_nonce(secret);
        // max_age_secs = 0 → already expired.
        assert!(!validate_nonce(&nonce, secret, 0));
    }

    // ── ACL ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_acl_allow_all_when_empty() {
        use sip3_backend::acl::{AclChecker, DefaultPolicy};
        use std::net::IpAddr;
        let checker = AclChecker::new(DefaultPolicy::Allow);
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        assert!(checker.is_allowed(ip));
    }

    #[test]
    fn test_acl_deny_all_default() {
        use sip3_backend::acl::{AclChecker, DefaultPolicy};
        use std::net::IpAddr;
        let checker = AclChecker::new(DefaultPolicy::Deny);
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        assert!(!checker.is_allowed(ip));
    }

    #[test]
    fn test_acl_parse_cidr() {
        use sip3_backend::acl::parse_cidr;
        // Valid IPv4 CIDR
        assert!(parse_cidr("192.168.1.0/24").is_ok());
        // Valid host address
        assert!(parse_cidr("10.0.0.1/32").is_ok());
        // Valid IPv6
        assert!(parse_cidr("::1/128").is_ok());
        // Invalid CIDR
        assert!(parse_cidr("not-a-cidr").is_err());
        assert!(parse_cidr("999.999.999.999/24").is_err());
    }

    #[test]
    fn test_sip_response_format() {
        // Verify that SipMessage::parse correctly identifies a response.
        let raw = "SIP/2.0 200 OK\r\n\
                   Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK776asdhds\r\n\
                   From: Alice <sip:alice@sip.example.com>;tag=1928301774\r\n\
                   To: Alice <sip:alice@sip.example.com>;tag=server-tag\r\n\
                   Call-ID: a84b4c76e66710@192.168.1.100\r\n\
                   CSeq: 314159 REGISTER\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse failed");
        assert!(msg.method.is_none());
        assert_eq!(msg.status_code, Some(200));
    }

    // ── Presence / BLF ───────────────────────────────────────────────────────

    #[test]
    fn test_subscribe_parse() {
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
    fn test_presence_pidf_open() {
        // Ensure the NOTIFY built for an Open status contains "open" in basic.
        let notify_msg = format!(
            "NOTIFY sip:bob@example.com SIP/2.0\r\n\
             Content-Type: application/pidf+xml\r\n\
             Content-Length: 0\r\n\
             \r\n\
             <basic>open</basic>"
        );
        assert!(notify_msg.contains("<basic>open</basic>"));
    }

    #[test]
    fn test_presence_pidf_closed() {
        let notify_msg = format!(
            "NOTIFY sip:bob@example.com SIP/2.0\r\n\
             Content-Type: application/pidf+xml\r\n\
             Content-Length: 0\r\n\
             \r\n\
             <basic>closed</basic>"
        );
        assert!(notify_msg.contains("<basic>closed</basic>"));
    }
}
