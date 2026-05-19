use anyhow::Result;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;

use super::assertions::ScenarioOutcome;
use super::dialog::DialogTrace;
use super::endpoint::{SipEndpoint, SipEndpointConfig, SipEvent, run_scoped_id};
use super::rtp_probe::RtpProbe;
use crate::sip::handler::{extract_uri, uri_username};
use crate::sip::media::sdp_rtp_addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioName {
    TlsRegisterDual,
    TlsMessageDual,
    TlsBasicCall,
}

impl ScenarioName {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "tls_register_dual" => Ok(Self::TlsRegisterDual),
            "tls_message_dual" => Ok(Self::TlsMessageDual),
            "tls_basic_call" => Ok(Self::TlsBasicCall),
            _ => anyhow::bail!("unknown scenario: {raw}"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TlsRegisterDual => "tls_register_dual",
            Self::TlsMessageDual => "tls_message_dual",
            Self::TlsBasicCall => "tls_basic_call",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TesterConfig {
    pub target_host: String,
    pub tls_port: u16,
    pub domain: String,
    pub realm: String,
    pub caller: SipEndpointConfig,
    pub callee: SipEndpointConfig,
    pub rtp_threshold: usize,
    pub scenario: ScenarioName,
}

pub async fn run_scenario(cfg: &TesterConfig) -> Result<ScenarioOutcome> {
    let mut caller = SipEndpoint::connect(cfg.caller.clone()).await?;
    let mut callee = SipEndpoint::connect(cfg.callee.clone()).await?;

    match cfg.scenario {
        ScenarioName::TlsRegisterDual => {
            caller.register().await?;
            callee.register().await?;
            Ok(ScenarioOutcome::pass(
                "tls_register_dual",
                "both endpoints registered over TLS",
            ))
        }
        ScenarioName::TlsMessageDual => {
            caller.register().await?;
            callee.register().await?;
            caller
                .send_message(&cfg.callee.username, "hello-from-caller")
                .await?;
            expect_event(
                &mut callee,
                "MESSAGE receipt",
                Duration::from_secs(5),
                |event| message_event_matches(event, &cfg.caller.username, "hello-from-caller"),
            )
            .await?;
            callee
                .send_message(&cfg.caller.username, "hello-from-callee")
                .await?;
            expect_event(
                &mut caller,
                "MESSAGE receipt",
                Duration::from_secs(5),
                |event| message_event_matches(event, &cfg.callee.username, "hello-from-callee"),
            )
            .await?;
            Ok(ScenarioOutcome::pass(
                "tls_message_dual",
                "two-way MESSAGE completed over TLS",
            ))
        }
        ScenarioName::TlsBasicCall => {
            caller.register().await?;
            callee.register().await?;
            run_tls_basic_call(&mut caller, &mut callee, cfg.rtp_threshold).await
        }
    }
}

async fn run_tls_basic_call(
    caller: &mut SipEndpoint,
    callee: &mut SipEndpoint,
    rtp_threshold: usize,
) -> Result<ScenarioOutcome> {
    let caller_probe = RtpProbe::bind("127.0.0.1:0").await?;
    let callee_probe = RtpProbe::bind("127.0.0.1:0").await?;
    let _receivers = ReceiverTasks::new(vec![
        caller_probe.spawn_receiver(),
        callee_probe.spawn_receiver(),
    ]);

    let call_id = run_scoped_id(
        &["call", &caller.cfg.username, &callee.cfg.username],
        &caller.cfg.run_token,
    );
    let mut trace = DialogTrace::new(&call_id);
    let offer = build_audio_sdp(caller_probe.local_addr());
    send_raw(
        caller,
        &build_invite_request(&caller.cfg, &callee.cfg.username, &call_id, &offer),
    )
    .await?;
    trace.on_invite_sent();

    let invite = expect_event(callee, "INVITE", Duration::from_secs(5), |event| {
        matches!(event, SipEvent::InviteReceived { .. })
    })
    .await?;
    let invite_sdp = match invite {
        SipEvent::InviteReceived { call_id, sdp, .. } => {
            anyhow::ensure!(
                call_id == trace.call_id,
                "unexpected INVITE call-id: {call_id}"
            );
            sdp
        }
        other => anyhow::bail!("unexpected INVITE event: {:?}", other),
    };
    callee_probe
        .set_peer(
            sdp_rtp_addr(&invite_sdp)
                .ok_or_else(|| anyhow::anyhow!("caller offer missing RTP address"))?,
        )
        .await;

    send_raw(
        callee,
        &build_invite_response(180, "Ringing", &trace.call_id, None),
    )
    .await?;
    expect_event(
        caller,
        "180 Ringing",
        Duration::from_secs(5),
        |event| matches!(event, SipEvent::Ringing { call_id } if call_id == &trace.call_id),
    )
    .await?;
    trace.on_ringing();

    let answer = build_audio_sdp(callee_probe.local_addr());
    send_raw(
        callee,
        &build_invite_response(200, "OK", &trace.call_id, Some(&answer)),
    )
    .await?;
    let answered = expect_event(
        caller,
        "200 OK",
        Duration::from_secs(5),
        |event| matches!(event, SipEvent::Answered { call_id, .. } if call_id == &trace.call_id),
    )
    .await?;
    let answer_sdp = match answered {
        SipEvent::Answered { sdp, .. } => sdp,
        other => anyhow::bail!("unexpected INVITE answer event: {:?}", other),
    };
    let answer_target = sdp_rtp_addr(&answer_sdp)
        .ok_or_else(|| anyhow::anyhow!("callee answer missing RTP address"))?;
    caller_probe
        .set_peer(require_relay_target(
            answer_target,
            callee_probe.local_addr(),
        )?)
        .await;
    trace.on_answered();

    send_raw(
        caller,
        &build_in_dialog_request("ACK", &caller.cfg, &callee.cfg.username, &trace.call_id, 1),
    )
    .await?;
    trace.on_ack_sent();
    expect_event(
        callee,
        "ACK",
        Duration::from_secs(5),
        |event| matches!(event, SipEvent::AckReceived { call_id } if call_id == &trace.call_id),
    )
    .await?;
    trace.require_established()?;

    let packet_count = rtp_threshold.max(8);
    caller_probe
        .send_packets(packet_count, Duration::from_millis(20))
        .await?;
    callee_probe
        .send_packets(packet_count, Duration::from_millis(20))
        .await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    send_raw(
        caller,
        &build_in_dialog_request("BYE", &caller.cfg, &callee.cfg.username, &trace.call_id, 2),
    )
    .await?;
    expect_event(
        callee,
        "BYE",
        Duration::from_secs(5),
        |event| matches!(event, SipEvent::ByeReceived { call_id } if call_id == &trace.call_id),
    )
    .await?;
    send_raw(callee, &build_bye_ok_response(&trace.call_id)).await?;
    expect_event(
        caller,
        "BYE OK",
        Duration::from_secs(5),
        |event| matches!(event, SipEvent::Ok { cseq_method } if cseq_method == "BYE"),
    )
    .await?;
    trace.on_bye();

    let caller_rx = caller_probe.rx_count();
    let callee_rx = callee_probe.rx_count();
    let outcome = if caller_probe.meets_threshold(rtp_threshold)
        && callee_probe.meets_threshold(rtp_threshold)
    {
        ScenarioOutcome::pass(
            "tls_basic_call",
            "TLS call established with bidirectional RTP",
        )
    } else {
        ScenarioOutcome::fail(
            "tls_basic_call",
            &format!(
                "RTP threshold not met, caller_rx={caller_rx} callee_rx={callee_rx} threshold={rtp_threshold}"
            ),
        )
    };

    Ok(outcome.with_rtp_counts(caller_rx, callee_rx))
}

async fn send_raw(endpoint: &mut SipEndpoint, raw: &str) -> Result<()> {
    endpoint.writer.write_all(raw.as_bytes()).await?;
    Ok(())
}

async fn expect_event<F>(
    endpoint: &mut SipEndpoint,
    label: &str,
    timeout: Duration,
    predicate: F,
) -> Result<SipEvent>
where
    F: Fn(&SipEvent) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let now = tokio::time::Instant::now();
        anyhow::ensure!(now < deadline, "{label} timed out");
        let remaining = deadline.saturating_duration_since(now);
        let event = tokio::time::timeout(remaining, endpoint.events.recv())
            .await
            .map_err(|_| anyhow::anyhow!("{label} timed out"))?
            .ok_or_else(|| anyhow::anyhow!("{label} channel closed"))?;
        if predicate(&event) {
            return Ok(event);
        }
    }
}

fn message_event_matches(
    event: &SipEvent,
    expected_from_username: &str,
    expected_body: &str,
) -> bool {
    match event {
        SipEvent::MessageReceived { from, body } => {
            body == expected_body
                && extract_uri(from)
                    .and_then(|uri| uri_username(&uri))
                    .is_some_and(|username| username == expected_from_username)
        }
        _ => false,
    }
}

fn require_relay_target(
    answer_target: SocketAddr,
    direct_target: SocketAddr,
) -> Result<SocketAddr> {
    if answer_target == direct_target {
        anyhow::bail!(
            "expected relay target in 200 OK SDP, got direct callee target {}",
            direct_target
        );
    }

    Ok(answer_target)
}

fn build_audio_sdp(addr: SocketAddr) -> String {
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 {ip}\r\n\
         s=headless-call-tester\r\n\
         c=IN IP4 {ip}\r\n\
         t=0 0\r\n\
         m=audio {port} RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=sendrecv\r\n",
        ip = addr.ip(),
        port = addr.port(),
    )
}

fn build_invite_request(
    from: &SipEndpointConfig,
    to_username: &str,
    call_id: &str,
    sdp: &str,
) -> String {
    format!(
        "INVITE sip:{to_username}@{domain} SIP/2.0\r\n\
         Via: SIP/2.0/TLS tester.invalid;branch=z9hG4bK-{call_id};rport\r\n\
         From: <sip:{from_username}@{domain}>;tag=caller\r\n\
         To: <sip:{to_username}@{domain}>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:{from_username}@{domain};transport=tls>\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: {len}\r\n\
         \r\n\
         {sdp}",
        domain = from.domain,
        from_username = from.username,
        len = sdp.len(),
    )
}

fn build_invite_response(
    status_code: u16,
    reason: &str,
    call_id: &str,
    sdp: Option<&str>,
) -> String {
    let body = sdp.unwrap_or("");
    let content_type = if sdp.is_some() {
        "Content-Type: application/sdp\r\n"
    } else {
        ""
    };

    format!(
        "SIP/2.0 {status_code} {reason}\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         {content_type}Content-Length: {len}\r\n\
         \r\n\
         {body}",
        len = body.len(),
    )
}

fn build_in_dialog_request(
    method: &str,
    from: &SipEndpointConfig,
    to_username: &str,
    call_id: &str,
    cseq: u32,
) -> String {
    format!(
        "{method} sip:{to_username}@{domain} SIP/2.0\r\n\
         Via: SIP/2.0/TLS tester.invalid;branch=z9hG4bK-{call_id}-{method};rport\r\n\
         From: <sip:{from_username}@{domain}>;tag=caller\r\n\
         To: <sip:{to_username}@{domain}>;tag=callee\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} {method}\r\n\
         Contact: <sip:{from_username}@{domain};transport=tls>\r\n\
         Content-Length: 0\r\n\
         \r\n",
        domain = from.domain,
        from_username = from.username,
    )
}

fn build_bye_ok_response(call_id: &str) -> String {
    format!(
        "SIP/2.0 200 OK\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 2 BYE\r\n\
         Content-Length: 0\r\n\
         \r\n"
    )
}

struct ReceiverTasks {
    handles: Vec<JoinHandle<()>>,
}

impl ReceiverTasks {
    fn new(handles: Vec<JoinHandle<()>>) -> Self {
        Self { handles }
    }
}

impl Drop for ReceiverTasks {
    fn drop(&mut self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::assertions::ScenarioStatus;

    fn test_endpoint(label: &str, username: &str) -> SipEndpointConfig {
        SipEndpointConfig {
            label: label.to_string(),
            host: "127.0.0.1".to_string(),
            tls_port: 5061,
            domain: "sip.air32.cn".to_string(),
            realm: "sip.air32.cn".to_string(),
            username: username.to_string(),
            password: "secret".to_string(),
            run_token: "test-run".to_string(),
            insecure_tls: true,
        }
    }

    fn test_config(scenario: ScenarioName) -> TesterConfig {
        TesterConfig {
            target_host: "127.0.0.1".to_string(),
            tls_port: 5061,
            domain: "sip.air32.cn".to_string(),
            realm: "sip.air32.cn".to_string(),
            caller: test_endpoint("caller", "1001"),
            callee: test_endpoint("callee", "1002"),
            rtp_threshold: 1,
            scenario,
        }
    }

    #[test]
    fn scenario_name_rejects_unknown_values() {
        let err = ScenarioName::parse("not-real").expect_err("unknown scenario");
        assert!(err.to_string().contains("unknown scenario"));
    }

    #[test]
    fn scenario_name_parses_all_supported_tls_names() {
        assert_eq!(
            ScenarioName::parse("tls_register_dual").expect("parse register scenario"),
            ScenarioName::TlsRegisterDual
        );
        assert_eq!(
            ScenarioName::parse("tls_message_dual").expect("parse message scenario"),
            ScenarioName::TlsMessageDual
        );
        assert_eq!(
            ScenarioName::parse("tls_basic_call").expect("parse basic call scenario"),
            ScenarioName::TlsBasicCall
        );
    }

    #[test]
    fn scenario_name_formats_all_supported_tls_names() {
        assert_eq!(ScenarioName::TlsRegisterDual.as_str(), "tls_register_dual");
        assert_eq!(ScenarioName::TlsMessageDual.as_str(), "tls_message_dual");
        assert_eq!(ScenarioName::TlsBasicCall.as_str(), "tls_basic_call");
    }

    #[test]
    fn test_config_preserves_expected_tls_defaults() {
        let cfg = test_config(ScenarioName::TlsBasicCall);

        assert_eq!(cfg.target_host, "127.0.0.1");
        assert_eq!(cfg.tls_port, 5061);
        assert_eq!(cfg.domain, "sip.air32.cn");
        assert_eq!(cfg.realm, "sip.air32.cn");
        assert_eq!(cfg.rtp_threshold, 1);
        assert_eq!(cfg.scenario, ScenarioName::TlsBasicCall);
        assert_eq!(cfg.caller.username, "1001");
        assert_eq!(cfg.callee.username, "1002");
        assert_eq!(
            ScenarioOutcome::pass(cfg.scenario.as_str(), "ok").status,
            ScenarioStatus::Passed
        );
    }

    #[test]
    fn message_event_matches_requires_expected_sender_and_body() {
        let event = SipEvent::MessageReceived {
            from: "<sip:1001@sip.air32.cn>;tag=abc".into(),
            body: "hello-from-caller".into(),
        };

        assert!(message_event_matches(&event, "1001", "hello-from-caller"));
        assert!(!message_event_matches(&event, "1002", "hello-from-caller"));
        assert!(!message_event_matches(&event, "1001", "wrong-body"));
    }

    #[test]
    fn build_invite_request_includes_run_token() {
        let cfg = test_endpoint("caller", "1001");

        let invite = build_invite_request(
            &cfg,
            "1002",
            "call-1001-1002-test-run",
            "v=0\r\nm=audio 1234 RTP/AVP 0\r\n",
        );

        assert!(invite.contains("Call-ID: call-1001-1002-test-run\r\n"));
        assert!(invite.contains("branch=z9hG4bK-call-1001-1002-test-run"));
    }

    #[test]
    fn require_relay_target_rejects_direct_answer_target() {
        let direct = "127.0.0.1:40000".parse().expect("direct addr");

        let err = require_relay_target(direct, direct).expect_err("direct target must fail");

        assert!(err.to_string().contains("relay target"));
    }
}
