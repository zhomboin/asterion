ALTER TABLE runtimes ADD COLUMN endpoint_fingerprint TEXT;
ALTER TABLE runtimes ADD COLUMN engine_kind TEXT CHECK (engine_kind IN ('desktop', 'native'));
