ALTER TABLE nodes ADD COLUMN ssh_profile TEXT;
ALTER TABLE nodes ADD COLUMN ssh_online_until INTEGER;
ALTER TABLE nodes ADD COLUMN ssh_error TEXT;
ALTER TABLE discovered_resources ADD COLUMN source TEXT NOT NULL DEFAULT 'agent' CHECK (source IN ('agent', 'ssh'));
