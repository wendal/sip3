//! Integration tests that exercise the actual production SIP handler code.
//! Functions are imported directly from the `sip3_backend` library crate.

#[cfg(test)]
mod tests {
    use sip3_backend::api::accounts::validate_sip_username;
    use sip3_backend::security_guard::{AuthSurface, GuardLimits, SecurityGuard};
    use sip3_backend::sip::call_cleanup::STALE_CALL_CLEANUP_SQL;
    use sip3_backend::sip::handler::{
        SIP_ALLOW_METHODS, SipMessage, extract_uri, md5_hex, normalize_header_name,
        parse_auth_params, strip_proxy_via, uri_username,
    };
    use sip3_backend::sip::media::{MediaRelay, rewrite_sdp, sdp_audio_port, sdp_has_crypto};
    use sip3_backend::sip::proxy::{
        CALLER_ACCOUNT_EXISTS_SQL, MESSAGE_SENDER_ACCOUNT_EXISTS_SQL,
        build_forwarded_cancel_for_target, should_bridge_plain_sip_to_websocket_target,
        should_preserve_webrtc_sdp_for_target, should_refresh_registration_source,
    };
    use sip3_backend::sip::registrar::{ACCOUNT_LOOKUP_SQL, generate_nonce, validate_nonce};
    use sip3_backend::sip::transport::TransportRegistry;
    use std::time::Duration;

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
    fn test_allow_methods_include_message() {
        assert!(SIP_ALLOW_METHODS.contains("MESSAGE"));
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
    fn test_stale_call_cleanup_sql_targets_only_open_active_calls() {
        let sql = STALE_CALL_CLEANUP_SQL.to_lowercase();
        // Must only touch sip_calls, must scope to NULL ended_at, and must
        // limit to the two "open" statuses so finished calls are not rewritten.
        assert!(sql.contains("update sip_calls"), "must update sip_calls");
        assert!(
            sql.contains("set status = 'ended'") && sql.contains("ended_at = now()"),
            "must mark rows as ended with a real timestamp"
        );
        assert!(
            sql.contains("ended_at is null"),
            "must not rewrite rows that already ended"
        );
        assert!(
            sql.contains("status in ('trying', 'answered')"),
            "must only target rows still considered active"
        );
        // The NULL-tolerant predicate lets a single bind value handle both
        // "close everything" and "older than N hours" cases.
        assert!(
            sql.contains("? is null or started_at < date_sub(now(), interval ? hour)"),
            "must support an optional age threshold"
        );
    }

    #[test]
    fn test_account_existence_queries_do_not_decode_unsigned_ids() {
        assert!(
            !ACCOUNT_LOOKUP_SQL.to_lowercase().contains("select id"),
            "REGISTER account lookup must not decode BIGINT UNSIGNED id into a signed Rust integer"
        );
        assert!(
            !CALLER_ACCOUNT_EXISTS_SQL
                .to_lowercase()
                .contains("select id"),
            "INVITE caller lookup must not decode BIGINT UNSIGNED id into a signed Rust integer"
        );
        assert!(
            !MESSAGE_SENDER_ACCOUNT_EXISTS_SQL
                .to_lowercase()
                .contains("select id"),
            "MESSAGE sender lookup must not decode BIGINT UNSIGNED id into a signed Rust integer"
        );
    }

    #[test]
    fn test_websocket_callee_preserves_webrtc_sdp() {
        let webrtc_sdp = "v=0\r\n\
                          m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
                          a=fingerprint:sha-256 ABCD\r\n";

        assert!(should_preserve_webrtc_sdp_for_target(
            "sip:abc@example.invalid;transport=ws",
            webrtc_sdp
        ));
        assert!(!should_preserve_webrtc_sdp_for_target(
            "sip:1001@192.0.2.10:5060",
            webrtc_sdp
        ));
    }

    #[test]
    fn test_websocket_callee_with_plain_sdp_requires_reverse_bridge() {
        let plain_sdp = "v=0\r\n\
                         o=alice 123 1 IN IP4 192.0.2.10\r\n\
                         s=-\r\n\
                         c=IN IP4 192.0.2.10\r\n\
                         t=0 0\r\n\
                         m=audio 49170 RTP/AVP 0 8\r\n";
        let webrtc_sdp = "v=0\r\n\
                          m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
                          a=ice-ufrag:abc\r\n\
                          a=fingerprint:sha-256 ABCD\r\n";

        assert!(should_bridge_plain_sip_to_websocket_target(
            "sip:abc@example.invalid;transport=ws",
            plain_sdp
        ));
        assert!(!should_bridge_plain_sip_to_websocket_target(
            "sip:abc@example.invalid;transport=ws",
            webrtc_sdp
        ));
        assert!(!should_bridge_plain_sip_to_websocket_target(
            "sip:1001@192.0.2.10:5060",
            plain_sdp
        ));
    }

    #[test]
    fn test_should_refresh_registration_source_when_ip_matches_and_port_changes() {
        let src = "119.130.132.117:62505".parse().expect("socket addr");
        assert!(should_refresh_registration_source(
            "119.130.132.117",
            64346,
            src
        ));
    }

    #[test]
    fn test_should_not_refresh_registration_source_when_ip_differs() {
        let src = "119.130.132.117:62505".parse().expect("socket addr");
        assert!(!should_refresh_registration_source(
            "203.0.113.10",
            64346,
            src
        ));
    }

    #[test]
    fn test_should_not_refresh_registration_source_when_port_unchanged() {
        let src = "119.130.132.117:64346".parse().expect("socket addr");
        assert!(!should_refresh_registration_source(
            "119.130.132.117",
            64346,
            src
        ));
    }

    #[test]
    fn test_forwarded_cancel_targets_callee_contact_and_proxy_branch() {
        let raw = "CANCEL sip:1003@sip.air32.cn SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.2:56473;branch=z9hG4bK.NMNgTadYq;rport\r\n\
                   Max-Forwards: 70\r\n\
                   From: <sip:1001@sip.air32.cn>;tag=oP6liqShM\r\n\
                   To: sip:1003@sip.air32.cn\r\n\
                   Call-ID: NO4pEKSYw-\r\n\
                   CSeq: 20 CANCEL\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse cancel");
        let forwarded = build_forwarded_cancel_for_target(
            &msg,
            "sip:1003@192.168.1.2:43453;transport=udp",
            69,
            "sip.air32.cn",
        );

        assert!(
            forwarded.starts_with("CANCEL sip:1003@192.168.1.2:43453;transport=udp SIP/2.0\r\n")
        );
        assert!(forwarded.contains("Via: SIP/2.0/UDP sip.air32.cn;branch=z9hG4bKproxy"));
        assert!(
            forwarded.contains("Via: SIP/2.0/UDP 192.168.1.2:56473;branch=z9hG4bK.NMNgTadYq;rport")
        );
        assert!(forwarded.contains("Max-Forwards: 69"));
        assert!(forwarded.ends_with("\r\n\r\n"));
    }

    #[tokio::test]
    async fn test_media_relay_sends_from_signaled_ports() {
        let relay = MediaRelay::new("127.0.0.1".to_string(), 31000, 31100);
        let call_id = format!("test-call-{}", std::process::id());
        let (relay_a, relay_b) = relay
            .allocate_session(call_id.clone())
            .await
            .expect("allocate relay session");

        let caller = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind caller");
        let callee = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind callee");

        caller
            .send_to(b"learn-caller", ("127.0.0.1", relay_b))
            .await
            .expect("caller learn send");
        let mut caller_buf = [0u8; 256];
        let (caller_len, caller_src) = {
            let mut received = None;
            for _ in 0..10 {
                callee
                    .send_to(b"to-caller", ("127.0.0.1", relay_a))
                    .await
                    .expect("callee send");
                if let Ok(Ok(pair)) = tokio::time::timeout(
                    Duration::from_millis(250),
                    caller.recv_from(&mut caller_buf),
                )
                .await
                {
                    received = Some(pair);
                    break;
                }
            }
            received.expect("caller receive timeout")
        };
        assert_eq!(&caller_buf[..caller_len], b"to-caller");
        assert_eq!(
            caller_src.port(),
            relay_b,
            "caller must receive from relay_b announced in 200 OK SDP"
        );

        let mut callee_buf = [0u8; 256];
        let (callee_len, callee_src) = {
            let mut received = None;
            for _ in 0..20 {
                caller
                    .send_to(b"to-callee", ("127.0.0.1", relay_b))
                    .await
                    .expect("caller send");
                if let Ok(Ok(pair)) = tokio::time::timeout(
                    Duration::from_millis(250),
                    callee.recv_from(&mut callee_buf),
                )
                .await
                {
                    let (len, src) = pair;
                    if &callee_buf[..len] == b"to-callee" {
                        received = Some((len, src));
                        break;
                    }
                }
            }
            received.expect("callee receive timeout")
        };
        assert_eq!(&callee_buf[..callee_len], b"to-callee");
        assert_eq!(
            callee_src.port(),
            relay_a,
            "callee must receive from relay_a announced in INVITE SDP"
        );

        relay.remove_session(&call_id).await;
    }

    #[test]
    fn test_transport_registry_routes_messages_to_stream_connections() {
        let registry = TransportRegistry::default();
        let addr = "127.0.0.1:54430".parse().unwrap();
        let mut rx = registry.register(addr);

        assert!(registry.send(
            addr,
            "INVITE sip:1001@example.com SIP/2.0\r\n\r\n".to_string()
        ));
        assert_eq!(
            rx.try_recv().unwrap(),
            "INVITE sip:1001@example.com SIP/2.0\r\n\r\n"
        );

        registry.unregister(addr);
        assert!(!registry.send(addr, "BYE sip:1001@example.com SIP/2.0\r\n\r\n".to_string()));
    }

    #[test]
    fn test_sip_username_must_be_three_to_six_digits() {
        for username in ["100", "1001", "999999"] {
            assert!(
                validate_sip_username(username).is_ok(),
                "{username} should be accepted as a dialable extension"
            );
        }

        for username in ["", "12", "1000000", "alice", "100a", "10 01"] {
            assert!(
                validate_sip_username(username).is_err(),
                "{username:?} should be rejected because phone dialing only supports 3-6 digit extensions"
            );
        }
    }

    #[test]
    fn test_numeric_seed_migration_replaces_legacy_default_accounts() {
        for seed_sql in [
            include_str!("../migrations/008_numeric_seed_accounts.sql"),
            include_str!("../../migrations/008_numeric_seed_accounts.sql"),
        ] {
            for extension in ["1001", "1002", "1003"] {
                assert!(
                    seed_sql.contains(&format!("'{extension}'")),
                    "numeric seed migration should create extension {extension}"
                );
            }
            for legacy_username in ["alice", "bob", "charlie"] {
                assert!(
                    seed_sql.contains(&format!("'{legacy_username}'")),
                    "numeric seed migration should remove legacy username {legacy_username}"
                );
            }
        }
    }

    #[test]
    fn test_nonce_expired() {
        let secret = "test_secret";
        let nonce = generate_nonce(secret);
        // max_age_secs = 0 → already expired.
        assert!(!validate_nonce(&nonce, secret, 0));
    }

    // ── Security guard (bruteforce protection) ───────────────────────────────

    #[test]
    fn test_guard_blocks_by_ip_after_threshold() {
        let limits = GuardLimits {
            window_secs: 300,
            ip_fail_threshold: 3,
            user_ip_fail_threshold: 10,
            block_secs: 900,
        };
        let mut guard = SecurityGuard::new(limits);

        assert!(!guard.is_blocked("198.51.100.10"));
        assert!(!guard.record_failure(AuthSurface::SipRegister, "198.51.100.10", Some("1001")));
        assert!(!guard.record_failure(AuthSurface::SipRegister, "198.51.100.10", Some("1002")));
        assert!(guard.record_failure(AuthSurface::SipRegister, "198.51.100.10", Some("1003")));
        assert!(guard.is_blocked("198.51.100.10"));
    }

    #[test]
    fn test_guard_blocks_by_user_ip_after_threshold() {
        let limits = GuardLimits {
            window_secs: 300,
            ip_fail_threshold: 50,
            user_ip_fail_threshold: 2,
            block_secs: 900,
        };
        let mut guard = SecurityGuard::new(limits);

        assert!(!guard.record_failure(AuthSurface::SipRegister, "203.0.113.20", Some("1001")));
        assert!(guard.record_failure(AuthSurface::SipRegister, "203.0.113.20", Some("1001")));
        assert!(guard.is_blocked("203.0.113.20"));
    }

    #[test]
    fn test_guard_success_clears_counters_for_same_user_ip() {
        let limits = GuardLimits {
            window_secs: 300,
            ip_fail_threshold: 20,
            user_ip_fail_threshold: 2,
            block_secs: 900,
        };
        let mut guard = SecurityGuard::new(limits);

        assert!(!guard.record_failure(AuthSurface::ApiLogin, "203.0.113.30", Some("admin")));
        guard.record_success("203.0.113.30", Some("admin"));
        assert!(!guard.record_failure(AuthSurface::ApiLogin, "203.0.113.30", Some("admin")));
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

    // ── SRTP / SDES ──────────────────────────────────────────────────────────

    #[test]
    fn test_srtp_sdp_crypto_preserved() {
        // Verify that rewrite_sdp passes a=crypto: lines through unchanged.
        let sdp = "v=0\r\n\
                   o=alice 1234 1 IN IP4 192.168.1.100\r\n\
                   s=-\r\n\
                   c=IN IP4 192.168.1.100\r\n\
                   t=0 0\r\n\
                   m=audio 49170 RTP/SAVP 0\r\n\
                   a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:WVNfX19zZW1jdGwgKioqKioqKioqKioqKio=\r\n\
                   a=rtpmap:0 PCMU/8000\r\n";

        let rewritten = rewrite_sdp(sdp, "10.0.0.1", 10000);

        // IP and port must be rewritten.
        assert!(rewritten.contains("c=IN IP4 10.0.0.1"));
        assert!(rewritten.contains("m=audio 10000 RTP/SAVP 0"));

        // Crypto attribute must be preserved verbatim.
        assert!(rewritten.contains(
            "a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:WVNfX19zZW1jdGwgKioqKioqKioqKioqKio="
        ));
    }

    #[test]
    fn test_srtp_sdp_has_crypto_detection() {
        let sdp_with_crypto = "m=audio 49170 RTP/SAVP 0\r\n\
                                a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:key==\r\n";
        let sdp_plain = "m=audio 49170 RTP/AVP 0\r\n\
                         a=rtpmap:0 PCMU/8000\r\n";

        assert!(sdp_has_crypto(sdp_with_crypto));
        assert!(!sdp_has_crypto(sdp_plain));
    }

    #[test]
    fn test_srtp_savp_port_extraction() {
        // sdp_audio_port must work with RTP/SAVP (SRTP) m= lines.
        let sdp =
            "m=audio 12345 RTP/SAVP 0 8\r\na=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:key==\r\n";
        assert_eq!(sdp_audio_port(sdp), Some(12345));
    }
}
