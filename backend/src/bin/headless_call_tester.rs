fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

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
