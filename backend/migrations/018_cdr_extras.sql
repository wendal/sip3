-- CDR enrichment columns needed for export, billing and Webhook payloads.
-- All new columns are NULL-tolerant so existing rows stay valid.

ALTER TABLE sip_calls
    ADD COLUMN hangup_cause     VARCHAR(64)  NULL AFTER status,
    ADD COLUMN sip_response_code SMALLINT UNSIGNED NULL AFTER hangup_cause,
    ADD COLUMN recording_key    VARCHAR(512) NULL AFTER sip_response_code;

-- Index to support CDR export filtering by status and time range.
CREATE INDEX idx_calls_status_ended ON sip_calls (status, ended_at);
