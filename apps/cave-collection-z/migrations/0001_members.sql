-- The whitelist (DECISIONS §74.13, §168): one row per approved person, from
-- approval to being on the sale's list on Zyn. Applications stay in the
-- CAVE_WHITELIST KV namespace; this holds only who was let in.
CREATE TABLE members (
  handle      TEXT PRIMARY KEY COLLATE NOCASE,  -- their X handle
  code_hash   TEXT NOT NULL UNIQUE,             -- SHA-256 of the invitation code; the code itself is kept nowhere
  approved_at TEXT NOT NULL,
  account     TEXT UNIQUE,                      -- the Zyn account the code was linked to, once
  linked_at   TEXT,
  listed_at   TEXT,                             -- when that account went on the sale's list on Zyn
  note        TEXT
);
CREATE INDEX members_waiting ON members (listed_at) WHERE account IS NOT NULL;
