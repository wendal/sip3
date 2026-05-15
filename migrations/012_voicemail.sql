CREATE TABLE IF NOT EXISTS sip_voicemail_boxes (
    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    username VARCHAR(64) NOT NULL,
    domain VARCHAR(128) NOT NULL,
    enabled TINYINT(1) NOT NULL DEFAULT 1,
    no_answer_secs INT UNSIGNED NOT NULL DEFAULT 25,
    max_message_secs INT UNSIGNED NOT NULL DEFAULT 120,
    max_messages INT UNSIGNED NOT NULL DEFAULT 100,
    greeting_storage_key VARCHAR(512) NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uniq_voicemail_box (username, domain),
    CONSTRAINT fk_voicemail_box_account
      FOREIGN KEY (username, domain)
      REFERENCES sip_accounts(username, domain)
      ON DELETE CASCADE
      ON UPDATE CASCADE
);

CREATE TABLE IF NOT EXISTS sip_voicemail_messages (
    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    box_id BIGINT UNSIGNED NOT NULL,
    caller VARCHAR(128) NOT NULL,
    callee VARCHAR(128) NOT NULL,
    call_id VARCHAR(255) NOT NULL,
    duration_secs INT UNSIGNED NOT NULL DEFAULT 0,
    storage_key VARCHAR(512) NOT NULL,
    content_type VARCHAR(128) NOT NULL DEFAULT 'audio/wav',
    status VARCHAR(32) NOT NULL DEFAULT 'new',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    heard_at DATETIME NULL,
    UNIQUE KEY uniq_voicemail_storage_key (storage_key),
    KEY idx_voicemail_box_status_created (box_id, status, created_at),
    KEY idx_voicemail_call_id (call_id),
    CONSTRAINT fk_voicemail_message_box
      FOREIGN KEY (box_id)
      REFERENCES sip_voicemail_boxes(id)
      ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS sip_voicemail_mwi_subscriptions (
    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    subscriber VARCHAR(64) NOT NULL,
    domain VARCHAR(128) NOT NULL,
    call_id VARCHAR(255) NOT NULL,
    subscriber_tag VARCHAR(128) NOT NULL,
    subscriber_ip VARCHAR(45) NOT NULL,
    subscriber_port SMALLINT UNSIGNED NOT NULL,
    expires_at DATETIME NOT NULL,
    cseq INT UNSIGNED NOT NULL DEFAULT 1,
    UNIQUE KEY uniq_voicemail_mwi_subscription (subscriber, domain, call_id),
    KEY idx_voicemail_mwi_expires (expires_at)
);
