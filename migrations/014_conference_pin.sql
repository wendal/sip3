-- Add PIN protection to conference rooms
-- Optional PIN hash (bcrypt) for accessing conference rooms

ALTER TABLE sip_conference_rooms
    ADD COLUMN pin_hash VARCHAR(128) NULL AFTER max_participants;
