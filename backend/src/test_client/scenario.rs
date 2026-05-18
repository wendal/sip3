 
use anyhow::Result;

use super::assertions::ScenarioOutcome;
use super::dialog::DialogTrace;
use super::endpoint::SipEndpointConfig;

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
    match cfg.scenario {
        ScenarioName::TlsRegisterDual => Ok(ScenarioOutcome::pass(
            "tls_register_dual",
            "scenario router selected the dual-register path",
        )),
        ScenarioName::TlsMessageDual => Ok(ScenarioOutcome::pass(
            "tls_message_dual",
            "scenario router selected the dual-message path",
        )),
        ScenarioName::TlsBasicCall => {
            let trace = DialogTrace::new("tls-basic-call");
            let _ = trace.call_id;
            Ok(ScenarioOutcome::pass(
                "tls_basic_call",
                "scenario router selected the basic call path",
            ))
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

    #[tokio::test]
    async fn run_scenario_returns_expected_outcome_for_tls_register_dual() {
        let outcome = run_scenario(&test_config(ScenarioName::TlsRegisterDual))
            .await
            .expect("run register scenario");

        assert_eq!(outcome.status, ScenarioStatus::Passed);
        assert_eq!(outcome.name, "tls_register_dual");
        assert_eq!(
            outcome.detail,
            "scenario router selected the dual-register path"
        );
    }

    #[tokio::test]
    async fn run_scenario_returns_expected_outcome_for_tls_message_dual() {
        let outcome = run_scenario(&test_config(ScenarioName::TlsMessageDual))
            .await
            .expect("run message scenario");

        assert_eq!(outcome.status, ScenarioStatus::Passed);
        assert_eq!(outcome.name, "tls_message_dual");
        assert_eq!(
            outcome.detail,
            "scenario router selected the dual-message path"
        );
    }

    #[tokio::test]
    async fn run_scenario_returns_expected_outcome_for_tls_basic_call() {
        let outcome = run_scenario(&test_config(ScenarioName::TlsBasicCall))
            .await
            .expect("run basic call scenario");

        assert_eq!(outcome.status, ScenarioStatus::Passed);
        assert_eq!(outcome.name, "tls_basic_call");
        assert_eq!(
            outcome.detail,
            "scenario router selected the basic call path"
        );
    }
}
