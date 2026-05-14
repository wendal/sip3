-- Indexes on sip_calls for statistics queries
-- These dramatically speed up the /api/stats and /api/accounts list endpoints

ALTER TABLE sip_calls ADD INDEX idx_calls_started_at (started_at);
ALTER TABLE sip_calls ADD INDEX idx_calls_caller (caller);
ALTER TABLE sip_calls ADD INDEX idx_calls_callee (callee);
