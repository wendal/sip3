-- Add email notification field to voicemail boxes
ALTER TABLE sip_voicemail_boxes
    ADD COLUMN email VARCHAR(255) DEFAULT NULL AFTER pin_hash;
