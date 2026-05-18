 
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_name_rejects_unknown_values() {
        let err = ScenarioName::parse("not-real").expect_err("unknown scenario");
        assert!(err.contains("unknown scenario"));
    }
}
