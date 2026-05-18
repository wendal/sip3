#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ScenarioOutcome {
    pub name: String,
    pub status: ScenarioStatus,
    pub detail: String,
    pub caller_rtp_rx: usize,
    pub callee_rtp_rx: usize,
}

impl ScenarioOutcome {
    pub fn pass(name: &str, detail: &str) -> Self {
        Self {
            name: name.to_string(),
            status: ScenarioStatus::Passed,
            detail: detail.to_string(),
            caller_rtp_rx: 0,
            callee_rtp_rx: 0,
        }
    }

    pub fn fail(name: &str, detail: &str) -> Self {
        Self {
            name: name.to_string(),
            status: ScenarioStatus::Failed,
            detail: detail.to_string(),
            caller_rtp_rx: 0,
            callee_rtp_rx: 0,
        }
    }

    pub fn with_rtp_counts(mut self, caller_rtp_rx: usize, callee_rtp_rx: usize) -> Self {
        self.caller_rtp_rx = caller_rtp_rx;
        self.callee_rtp_rx = callee_rtp_rx;
        self
    }

    pub fn render(&self) -> String {
        let status = match self.status {
            ScenarioStatus::Passed => "PASSED",
            ScenarioStatus::Failed => "FAILED",
        };

        format!(
            "[{status}] {}: {} (caller_rx={}, callee_rx={})",
            self.name, self.detail, self.caller_rtp_rx, self.callee_rtp_rx
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_render_includes_name_detail_and_rtp_counts() {
        let rendered = ScenarioOutcome::fail(
            "tls_rtp_bidirectional",
            "RTP one-way, caller_rx=0 callee_rx=47",
        )
        .with_rtp_counts(0, 47)
        .render();

        assert!(rendered.contains("tls_rtp_bidirectional"));
        assert!(rendered.contains("FAILED"));
        assert!(rendered.contains("RTP one-way, caller_rx=0 callee_rx=47"));
        assert!(rendered.contains("caller_rx=0"));
        assert!(rendered.contains("callee_rx=47"));
    }
}
