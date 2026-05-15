CREATE TABLE IF NOT EXISTS sip_security_events (
    id          BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    surface     ENUM('sip_register', 'api_login') NOT NULL,
    event_type  ENUM('auth_failed', 'ip_blocked', 'auth_succeeded', 'ip_unblocked') NOT NULL,
    source_ip   VARCHAR(45) NOT NULL,
    username    VARCHAR(64),
    detail      VARCHAR(255) NOT NULL,
    created_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_security_events_created (created_at),
    INDEX idx_security_events_ip_created (source_ip, created_at),
    INDEX idx_security_events_type_created (event_type, created_at)
);
