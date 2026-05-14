CREATE TABLE IF NOT EXISTS sip_presence_subscriptions (
    id              BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    subscriber      VARCHAR(128) NOT NULL,
    target          VARCHAR(128) NOT NULL,
    domain          VARCHAR(128) NOT NULL,
    call_id         VARCHAR(255) NOT NULL,
    cseq            INT UNSIGNED NOT NULL DEFAULT 1,
    subscriber_tag  VARCHAR(128) NOT NULL DEFAULT '',
    subscriber_ip   VARCHAR(45)  NOT NULL,
    subscriber_port SMALLINT UNSIGNED NOT NULL,
    expires_at      DATETIME NOT NULL,
    UNIQUE KEY uniq_sub (subscriber, target, domain)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
