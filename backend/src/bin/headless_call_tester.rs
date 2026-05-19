use anyhow::Result;
use sip3_backend::test_client::assertions::ScenarioStatus;
use sip3_backend::test_client::endpoint::SipEndpointConfig;
use sip3_backend::test_client::scenario::{ScenarioName, TesterConfig, run_scenario};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct CliArgs {
    target: String,
    tls_port: u16,
    domain: String,
    realm: String,
    scenario: ScenarioName,
    caller: String,
    caller_password: String,
    callee: String,
    callee_password: String,
    rtp_threshold: usize,
    insecure_tls: bool,
}

impl CliArgs {
    fn parse(args: &[String]) -> Result<Self> {
        fn value(args: &[String], key: &str) -> Result<String> {
            let idx = args
                .iter()
                .position(|arg| arg == key)
                .ok_or_else(|| anyhow::anyhow!("missing argument: {key}"))?;
            args.get(idx + 1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing value for {key}"))
        }

        Ok(Self {
            target: value(args, "--target")?,
            tls_port: value(args, "--tls-port")?.parse()?,
            domain: value(args, "--domain")?,
            realm: value(args, "--realm")?,
            scenario: ScenarioName::parse(&value(args, "--scenario")?)?,
            caller: value(args, "--caller")?,
            caller_password: value(args, "--caller-password")?,
            callee: value(args, "--callee")?,
            callee_password: value(args, "--callee-password")?,
            rtp_threshold: value(args, "--rtp-threshold")
                .unwrap_or_else(|_| "8".to_string())
                .parse()?,
            insecure_tls: args.iter().any(|arg| arg == "--insecure-tls"),
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cli = CliArgs::parse(&args)?;
    let run_token = Uuid::new_v4().simple().to_string();

    let caller = SipEndpointConfig {
        label: "caller".into(),
        host: cli.target.clone(),
        tls_port: cli.tls_port,
        domain: cli.domain.clone(),
        realm: cli.realm.clone(),
        username: cli.caller.clone(),
        password: cli.caller_password.clone(),
        run_token: run_token.clone(),
        insecure_tls: cli.insecure_tls,
    };
    let callee = SipEndpointConfig {
        label: "callee".into(),
        host: cli.target.clone(),
        tls_port: cli.tls_port,
        domain: cli.domain.clone(),
        realm: cli.realm.clone(),
        username: cli.callee.clone(),
        password: cli.callee_password.clone(),
        run_token,
        insecure_tls: cli.insecure_tls,
    };

    let cfg = TesterConfig {
        target_host: cli.target,
        tls_port: cli.tls_port,
        domain: cli.domain,
        realm: cli.realm,
        caller,
        callee,
        rtp_threshold: cli.rtp_threshold,
        scenario: cli.scenario,
    };

    let outcome = run_scenario(&cfg).await?;
    println!("{}", outcome.render());

    if matches!(outcome.status, ScenarioStatus::Failed) {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_token_is_unique_across_calls() {
        let first = new_run_token();
        let second = new_run_token();

        assert!(!first.is_empty());
        assert!(!second.is_empty());
        assert_ne!(first, second);
    }

    #[test]
    fn parse_args_accepts_tls_basic_call() {
        let args = vec![
            "headless_call_tester".to_string(),
            "--target".to_string(),
            "sip.air32.cn".to_string(),
            "--tls-port".to_string(),
            "5061".to_string(),
            "--domain".to_string(),
            "sip.air32.cn".to_string(),
            "--realm".to_string(),
            "sip.air32.cn".to_string(),
            "--scenario".to_string(),
            "tls_basic_call".to_string(),
            "--caller".to_string(),
            "1001".to_string(),
            "--caller-password".to_string(),
            "secret1".to_string(),
            "--callee".to_string(),
            "1003".to_string(),
            "--callee-password".to_string(),
            "secret3".to_string(),
        ];

        let cli = CliArgs::parse(&args).expect("parse args");
        assert_eq!(cli.target, "sip.air32.cn");
        assert_eq!(cli.scenario.as_str(), "tls_basic_call");
        assert_eq!(cli.tls_port, 5061);
    }
}
