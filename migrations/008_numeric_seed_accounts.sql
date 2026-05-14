-- Replace non-dialable legacy seed users with numeric extensions for the browser dial pad.
DELETE FROM sip_registrations
WHERE domain = 'sip.air32.cn'
  AND username IN ('alice', 'bob', 'charlie');

DELETE FROM sip_presence_subscriptions
WHERE domain = 'sip.air32.cn'
  AND (subscriber IN ('alice', 'bob', 'charlie')
       OR target IN ('alice', 'bob', 'charlie'));

DELETE FROM sip_accounts
WHERE domain = 'sip.air32.cn'
  AND username IN ('alice', 'bob', 'charlie');

INSERT INTO sip_accounts (username, password_hash, ha1_hash, display_name, domain, enabled) VALUES
('1001',
 '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeAFe1234567890ab',
 MD5('1001:sip.air32.cn:password123'),
 'Alice',
 'sip.air32.cn',
 1),
('1002',
 '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeAFe1234567890ab',
 MD5('1002:sip.air32.cn:password123'),
 'Bob',
 'sip.air32.cn',
 1),
('1003',
 '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeAFe1234567890ab',
 MD5('1003:sip.air32.cn:password123'),
 'Charlie',
 'sip.air32.cn',
 1)
ON DUPLICATE KEY UPDATE
    ha1_hash = VALUES(ha1_hash),
    display_name = VALUES(display_name),
    enabled = VALUES(enabled);
