-- SMTP outbox for voicemail email notifications.
-- Populated by voicemail storage save; drained by the email_worker background task.

CREATE TABLE IF NOT EXISTS sip_email_outbox (
    id              BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    mailbox_id      BIGINT UNSIGNED NULL,
    to_addr         VARCHAR(255)    NOT NULL,
    subject         VARCHAR(512)    NOT NULL,
    body            MEDIUMTEXT      NOT NULL,
    attachment_key  VARCHAR(512)    NULL,
    status          VARCHAR(16)     NOT NULL DEFAULT 'pending',
    attempts        INT UNSIGNED    NOT NULL DEFAULT 0,
    last_error      VARCHAR(1024)   NULL,
    scheduled_at    DATETIME        NOT NULL DEFAULT CURRENT_TIMESTAMP,
    sent_at         DATETIME        NULL,
    KEY idx_email_outbox_status (status, scheduled_at),
    KEY idx_email_outbox_mailbox (mailbox_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
