-- Webhook subscriptions and an at-least-once delivery outbox.
-- Secret is used to compute X-Sip3-Signature = hex(HMAC-SHA256(secret, body)).
-- events_json is a JSON array of event type strings (e.g. ["call.ended","registration.changed"]).

CREATE TABLE IF NOT EXISTS sip_webhooks (
    id          BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    name        VARCHAR(128)    NOT NULL,
    url         VARCHAR(1024)   NOT NULL,
    secret      VARCHAR(255)    NOT NULL,
    events_json JSON            NOT NULL,
    active      TINYINT(1)      NOT NULL DEFAULT 1,
    created_at  DATETIME        NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  DATETIME        NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uniq_webhook_name (name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS sip_webhook_deliveries (
    id            BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    webhook_id    BIGINT UNSIGNED NOT NULL,
    event_type    VARCHAR(64)     NOT NULL,
    payload_json  JSON            NOT NULL,
    status        VARCHAR(16)     NOT NULL DEFAULT 'pending',
    attempts      INT UNSIGNED    NOT NULL DEFAULT 0,
    last_error    VARCHAR(1024)   NULL,
    next_retry_at DATETIME        NULL,
    created_at    DATETIME        NOT NULL DEFAULT CURRENT_TIMESTAMP,
    delivered_at  DATETIME        NULL,
    KEY idx_webhook_delivery_status (status, next_retry_at),
    KEY idx_webhook_delivery_webhook (webhook_id, created_at),
    CONSTRAINT fk_webhook_delivery_webhook
        FOREIGN KEY (webhook_id) REFERENCES sip_webhooks(id)
        ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
