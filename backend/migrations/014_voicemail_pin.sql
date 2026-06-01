-- Add PIN hash to voicemail boxes for optional PIN authentication
ALTER TABLE sip_voicemail_boxes
    ADD COLUMN pin_hash VARCHAR(255) DEFAULT NULL AFTER greeting_storage_key;
