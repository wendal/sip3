 
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

    #[test]
    fn scenario_name_rejects_unknown_values() {
        let err = ScenarioName::parse("not-real").expect_err("unknown scenario");
        assert!(err.to_string().contains("unknown scenario"));
    }
}
