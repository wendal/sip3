-- SIP3 RTP media relay sessions
-- Tracks active media relay sessions for in-progress calls.
-- Rows are inserted on INVITE and cleaned up on BYE/CANCEL/non-2xx.

CREATE TABLE IF NOT EXISTS sip_media_sessions (
    id              BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    call_id         VARCHAR(255) NOT NULL,
    relay_port_a    SMALLINT UNSIGNED NOT NULL COMMENT 'Callee sends RTP here; server forwards to caller',
    relay_port_b    SMALLINT UNSIGNED NOT NULL COMMENT 'Caller sends RTP here; server forwards to callee',
    created_at      DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uniq_call_id (call_id)
);
