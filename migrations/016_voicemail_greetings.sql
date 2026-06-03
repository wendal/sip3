-- Custom voicemail greetings: tracks every uploaded greeting per mailbox.
-- The actual WAV bytes are stored on the filesystem under voicemail_storage_dir
-- (same LocalVoicemailStorage used for messages). One greeting per box; the
-- active key is mirrored onto sip_voicemail_boxes.greeting_storage_key.

CREATE TABLE IF NOT EXISTS sip_voicemail_greetings (
    id                 BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    box_id             BIGINT UNSIGNED NOT NULL,
    storage_key        VARCHAR(512)    NOT NULL,
    original_filename  VARCHAR(255)    NOT NULL,
    duration_secs      INT UNSIGNED    NOT NULL DEFAULT 0,
    created_at         DATETIME        NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uniq_voicemail_greeting_storage_key (storage_key),
    KEY idx_voicemail_greeting_box (box_id),
    CONSTRAINT fk_voicemail_greeting_box
        FOREIGN KEY (box_id) REFERENCES sip_voicemail_boxes(id)
        ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
