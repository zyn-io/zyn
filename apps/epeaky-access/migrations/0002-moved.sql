-- 15 Oct: a wallet moves its access to a new device once a day.
ALTER TABLE claims ADD COLUMN moved_at INTEGER;
CREATE INDEX IF NOT EXISTS claims_wallet ON claims (wallet, at);
