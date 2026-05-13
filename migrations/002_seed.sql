-- Seed data for SIP3
-- Default SIP domain: sip.air32.cn
-- Passwords are bcrypt hashed; ha1_hash is MD5(username:realm:password)
-- Default password for all seed users: "password123"
-- HA1 computed as: MD5("username:sip.air32.cn:password123")

INSERT INTO sip_accounts (username, password_hash, ha1_hash, display_name, domain, enabled) VALUES
('alice',
 '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeAFe1234567890ab',
 MD5('alice:sip.air32.cn:password123'),
 'Alice',
 'sip.air32.cn',
 1),
('bob',
 '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeAFe1234567890ab',
 MD5('bob:sip.air32.cn:password123'),
 'Bob',
 'sip.air32.cn',
 1),
('charlie',
 '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeAFe1234567890ab',
 MD5('charlie:sip.air32.cn:password123'),
 'Charlie',
 'sip.air32.cn',
 1)
ON DUPLICATE KEY UPDATE username = username;
