-- SIP3 IP ACL rules (CIDR allow/deny list)
-- Rules are matched in ascending priority order; first match wins.
-- Default behavior when no rule matches is controlled by config acl.default_policy.

CREATE TABLE IF NOT EXISTS sip_acl (
    id          INT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    action      ENUM('allow', 'deny') NOT NULL,
    cidr        VARCHAR(43) NOT NULL,             -- e.g. "192.168.1.0/24" or "::1/128"
    description VARCHAR(255),
    priority    INT NOT NULL DEFAULT 100,         -- lower value = matched first
    enabled     TINYINT(1) NOT NULL DEFAULT 1,
    created_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
