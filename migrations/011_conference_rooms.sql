-- Conference rooms (SIP-only, MVP) and live participant tracking.
-- Room extensions are 9-digit numeric to avoid colliding with 3-6 digit SIP user accounts.
-- PIN gating is intentionally deferred until an IVR exists; no pin columns in MVP.

CREATE TABLE IF NOT EXISTS sip_conference_rooms (
    id               BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    extension        VARCHAR(64)  NOT NULL,
    domain           VARCHAR(128) NOT NULL,
    name             VARCHAR(128) NOT NULL,
    enabled          TINYINT(1)   NOT NULL DEFAULT 1,
    max_participants INT UNSIGNED NOT NULL DEFAULT 20,
    created_at       DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at       DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uniq_conference_extension_domain (extension, domain)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS sip_conference_participants (
    id           BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    room_id      BIGINT UNSIGNED  NOT NULL,
    call_id      VARCHAR(255)     NOT NULL,
    account      VARCHAR(128)     NOT NULL,
    source_ip    VARCHAR(45)      NOT NULL,
    source_port  SMALLINT UNSIGNED NOT NULL,
    rtp_ip       VARCHAR(45)      NULL,
    rtp_port     SMALLINT UNSIGNED NULL,
    relay_port   SMALLINT UNSIGNED NOT NULL,
    codec        VARCHAR(16)      NOT NULL,
    muted        TINYINT(1)       NOT NULL DEFAULT 0,
    joined_at    DATETIME         NOT NULL DEFAULT CURRENT_TIMESTAMP,
    left_at      DATETIME         NULL,
    UNIQUE KEY uniq_conference_call_id (call_id),
    KEY idx_conference_room_active (room_id, left_at),
    CONSTRAINT fk_conference_participant_room
        FOREIGN KEY (room_id) REFERENCES sip_conference_rooms(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Seed one default conference room for smoke testing.
INSERT INTO sip_conference_rooms (extension, domain, name, enabled, max_participants)
VALUES ('900000000', 'sip.air32.cn', 'Default Conference', 1, 20)
ON DUPLICATE KEY UPDATE name = VALUES(name);
