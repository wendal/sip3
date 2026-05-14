-- SIP3 Database Schema
-- MySQL 8.0+

CREATE TABLE IF NOT EXISTS sip_accounts (
    id           BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    username     VARCHAR(64)  NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    ha1_hash     VARCHAR(255) COMMENT 'MD5(username:realm:password) for SIP Digest auth',
    display_name VARCHAR(128),
    domain       VARCHAR(128) NOT NULL DEFAULT 'localhost',
    enabled      TINYINT(1)  NOT NULL DEFAULT 1,
    created_at   DATETIME    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at   DATETIME    NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    -- SIP identities are user@domain; the same username can exist in multiple domains.
    UNIQUE KEY uniq_username_domain (username, domain)
);

CREATE TABLE IF NOT EXISTS sip_registrations (
    id            BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    username      VARCHAR(64)  NOT NULL,
    domain        VARCHAR(128) NOT NULL,
    contact_uri   VARCHAR(255) NOT NULL,
    user_agent    VARCHAR(255),
    expires_at    DATETIME     NOT NULL,
    registered_at DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    source_ip     VARCHAR(45)  NOT NULL,
    source_port   SMALLINT UNSIGNED NOT NULL,
    UNIQUE KEY uniq_user_domain (username, domain)
);

CREATE TABLE IF NOT EXISTS sip_calls (
    id          BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    call_id     VARCHAR(255) NOT NULL,
    caller      VARCHAR(128) NOT NULL,
    callee      VARCHAR(128) NOT NULL,
    status      VARCHAR(32)  NOT NULL DEFAULT 'trying',
    started_at  DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    answered_at DATETIME,
    ended_at    DATETIME,
    UNIQUE KEY uniq_call_id (call_id)
);
