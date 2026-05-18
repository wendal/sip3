 
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogState {
    Idle,
    Inviting,
    Ringing,
    Answered,
    Established,
    Terminated,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct DialogTrace {
    pub call_id: String,
    pub state: DialogState,
    saw_200_ok: bool,
    saw_ack: bool,
    saw_bye: bool,
}

impl DialogTrace {
    pub fn new(call_id: &str) -> Self {
        Self {
            call_id: call_id.to_string(),
            state: DialogState::Idle,
            saw_200_ok: false,
            saw_ack: false,
            saw_bye: false,
        }
    }

    pub fn on_invite_sent(&mut self) {
        self.state = DialogState::Inviting;
    }

    pub fn on_ringing(&mut self) {
        self.state = DialogState::Ringing;
    }

    pub fn on_answered(&mut self) {
        self.saw_200_ok = true;
        self.state = DialogState::Answered;
    }

    pub fn on_ack_sent(&mut self) {
        self.saw_ack = true;
        if self.saw_200_ok {
            self.state = DialogState::Established;
        }
    }

    pub fn on_bye(&mut self) {
        self.saw_bye = true;
        self.state = DialogState::Terminated;
    }

    pub fn require_established(&self) -> Result<()> {
        if !self.saw_200_ok {
            anyhow::bail!("dialog never received 200 OK");
        }
        if !self.saw_ack {
            anyhow::bail!("dialog never sent ACK");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answered_flow_requires_200_before_ack() {
        let mut trace = DialogTrace::new("call-42");
        trace.on_invite_sent();
        trace.on_ringing();
        trace.on_ack_sent();

        let err = trace
            .require_established()
            .expect_err("ack before 200 must fail");
        assert!(err.to_string().contains("200 OK"));
    }
}
