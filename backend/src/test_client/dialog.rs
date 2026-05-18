 
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answered_flow_requires_200_before_ack() {
        let mut trace = DialogTrace::new("call-42");
        trace.on_invite_sent();
        trace.on_ringing();
        trace.on_ack_sent();

        let err = trace.require_established().expect_err("ack before 200 must fail");
        assert!(err.contains("200 OK"));
    }
}
