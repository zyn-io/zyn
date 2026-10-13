-- X sign-in (DECISIONS §187.8): the X account a member signed in with, by its
-- numeric id, which survives a change of handle. Pinned at the first sign-in,
-- so a handle given up and taken by someone else cannot link a wallet.
ALTER TABLE members ADD COLUMN x_id TEXT;
CREATE UNIQUE INDEX members_x_id ON members (x_id) WHERE x_id IS NOT NULL;
