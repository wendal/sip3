CREATE TABLE IF NOT EXISTS sip_messages (
    id           BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    message_id   VARCHAR(255),
    call_id      VARCHAR(255),
    sender       VARCHAR(128) NOT NULL,
    receiver     VARCHAR(128) NOT NULL,
    content_type VARCHAR(128) NOT NULL DEFAULT 'text/plain',
    body         TEXT NOT NULL,
    status       VARCHAR(32) NOT NULL DEFAULT 'delivered',
    source_ip    VARCHAR(45) NOT NULL,
    created_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    delivered_at DATETIME,
    KEY idx_msg_sender_created (sender, created_at),
    KEY idx_msg_receiver_created (receiver, created_at),
    KEY idx_msg_call_id (call_id)
);
