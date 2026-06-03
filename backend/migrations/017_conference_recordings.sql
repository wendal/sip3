-- Conference room-level WAV recording.
-- When sip_conference_rooms.record_enabled=1, the conference mixer writes a
-- single mixed-down stream to a per-session WAV file under voicemail_storage_dir.

ALTER TABLE sip_conference_rooms
    ADD COLUMN record_enabled TINYINT(1) NOT NULL DEFAULT 0 AFTER max_participants;

CREATE TABLE IF NOT EXISTS sip_conference_recordings (
    id           BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    room_id      BIGINT UNSIGNED NOT NULL,
    started_at   DATETIME        NOT NULL DEFAULT CURRENT_TIMESTAMP,
    ended_at     DATETIME        NULL,
    storage_key  VARCHAR(512)    NOT NULL,
    size_bytes   BIGINT UNSIGNED NOT NULL DEFAULT 0,
    duration_secs INT UNSIGNED   NOT NULL DEFAULT 0,
    UNIQUE KEY uniq_conference_recording_storage_key (storage_key),
    KEY idx_conference_recording_room (room_id, started_at),
    CONSTRAINT fk_conference_recording_room
        FOREIGN KEY (room_id) REFERENCES sip_conference_rooms(id)
        ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
