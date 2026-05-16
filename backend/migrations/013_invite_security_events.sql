ALTER TABLE sip_security_events
    MODIFY COLUMN surface ENUM('sip_register', 'api_login', 'sip_invite') NOT NULL;

ALTER TABLE sip_security_events
    MODIFY COLUMN event_type ENUM(
        'auth_failed',
        'invite_rejected',
        'ip_blocked',
        'auth_succeeded',
        'ip_unblocked'
    ) NOT NULL;
