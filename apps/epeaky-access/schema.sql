-- One row per buy that let a device in: a buy lets in one pass.
CREATE TABLE IF NOT EXISTS claims (
  signature TEXT PRIMARY KEY,
  pass TEXT NOT NULL,
  wallet TEXT NOT NULL,
  at INTEGER NOT NULL
);
