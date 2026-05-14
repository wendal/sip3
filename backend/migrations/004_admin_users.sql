-- Admin users table for web console authentication
-- Default admin: username=admin, password=admin123
-- IMPORTANT: Change the default password after first login!

CREATE TABLE IF NOT EXISTS admin_users (
    id            BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    username      VARCHAR(64)  NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    created_at    DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at    DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
);

INSERT INTO admin_users (username, password_hash) VALUES
('admin', '$2b$12$w2.c8L8Loe/gSdFjvgdgJ.T1Ehw7kQyf2DRFuBhysMYVaiAZJqzty')
ON DUPLICATE KEY UPDATE username = username;
