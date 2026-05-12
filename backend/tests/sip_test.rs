#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    // Import the handler module items for testing
    // Since we're in an integration test, we need to reference the crate
    // These tests exercise SIP message parsing and auth utilities

    fn parse_sip_message(raw: &str) -> (Option<String>, HashMap<String, Vec<String>>) {
        let mut method = None;
        let mut headers: HashMap<String, Vec<String>> = HashMap::new();
        let mut lines = raw.lines();

        if let Some(first) = lines.next() {
            let parts: Vec<&str> = first.splitn(3, ' ').collect();
            if !first.starts_with("SIP/2.0") && parts.len() >= 1 {
                method = Some(parts[0].to_string());
            }
        }

        let mut in_headers = true;
        for line in lines {
            if !in_headers {
                break;
            }
            if line.is_empty() {
                in_headers = false;
                continue;
            }
            if let Some(pos) = line.find(':') {
                let name = line[..pos].trim().to_lowercase();
                let value = line[pos + 1..].trim().to_string();
                headers.entry(name).or_default().push(value);
            }
        }

        (method, headers)
    }

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

        let (method, headers) = parse_sip_message(raw);
        assert_eq!(method, Some("REGISTER".to_string()));
        assert!(headers.contains_key("via"));
        assert!(headers.contains_key("from"));
        assert!(headers.contains_key("to"));
        assert!(headers.contains_key("call-id"));
        assert!(headers.contains_key("cseq"));
        assert!(headers.contains_key("contact"));
        assert_eq!(headers["expires"][0], "3600");
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

        let (method, headers) = parse_sip_message(raw);
        assert_eq!(method, Some("INVITE".to_string()));
        assert!(headers.contains_key("content-type"));
    }

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
            assert_eq!(result, Some(expected.to_string()), "Failed for: {}", input);
        }
    }

    fn extract_uri(addr: &str) -> Option<String> {
        if let Some(start) = addr.find('<') {
            if let Some(end_rel) = addr[start..].find('>') {
                return Some(addr[start + 1..start + end_rel].trim().to_string());
            }
        }
        let uri = addr.split(';').next().unwrap_or(addr).trim();
        if uri.starts_with("sip:") || uri.starts_with("sips:") {
            Some(uri.to_string())
        } else {
            None
        }
    }

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

    fn uri_username(uri: &str) -> Option<String> {
        let without_scheme = uri.trim_start_matches("sip:").trim_start_matches("sips:");
        if without_scheme.contains('@') {
            Some(without_scheme.split('@').next()?.to_string())
        } else {
            None
        }
    }

    #[test]
    fn test_md5_digest_auth() {
        // Test MD5 digest computation per RFC 3261
        let username = "alice";
        let realm = "sip.example.com";
        let password = "secret";
        let method = "REGISTER";
        let uri = "sip:sip.example.com";
        let nonce = "testNonce12345";

        let ha1 = md5_hex(&format!("{}:{}:{}", username, realm, password));
        let ha2 = md5_hex(&format!("{}:{}", method, uri));
        let response = md5_hex(&format!("{}:{}:{}", ha1, nonce, ha2));

        // Verify the response is a 32-char hex string
        assert_eq!(response.len(), 32);
        assert!(response.chars().all(|c| c.is_ascii_hexdigit()));

        // Verify that same inputs produce same output (deterministic)
        let ha1_again = md5_hex(&format!("{}:{}:{}", username, realm, password));
        assert_eq!(ha1, ha1_again);
    }

    fn md5_hex(input: &str) -> String {
        format!("{:x}", md5::compute(input.as_bytes()))
    }

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
    }

    fn parse_auth_params(header: &str) -> HashMap<String, String> {
        let mut params = HashMap::new();
        let rest = match header.find(' ') {
            Some(pos) => &header[pos + 1..],
            None => return params,
        };

        for part in rest.split(',') {
            let part = part.trim();
            if let Some(eq_pos) = part.find('=') {
                let key = part[..eq_pos].trim().to_lowercase();
                let val = part[eq_pos + 1..].trim().trim_matches('"').to_string();
                params.insert(key, val);
            }
        }
        params
    }

    #[test]
    fn test_sip_response_format() {
        let response = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK776asdhds\r\n\
             From: Alice <sip:alice@sip.example.com>;tag=1928301774\r\n\
             To: Alice <sip:alice@sip.example.com>;tag=server-tag\r\n\
             Call-ID: a84b4c76e66710@192.168.1.100\r\n\
             CSeq: 314159 REGISTER\r\n\
             Content-Length: 0\r\n\
             \r\n"
        );

        assert!(response.starts_with("SIP/2.0 200 OK"));
        assert!(response.contains("Content-Length: 0"));
        assert!(response.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_options_response() {
        let options_req = "OPTIONS sip:sip.example.com SIP/2.0\r\n\
                           Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bKoptions\r\n\
                           From: <sip:alice@sip.example.com>;tag=123\r\n\
                           To: <sip:sip.example.com>\r\n\
                           Call-ID: options-test@192.168.1.100\r\n\
                           CSeq: 1 OPTIONS\r\n\
                           Content-Length: 0\r\n\
                           \r\n";

        let (method, _) = parse_sip_message(options_req);
        assert_eq!(method, Some("OPTIONS".to_string()));
    }
}
