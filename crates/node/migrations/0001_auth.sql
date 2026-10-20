CREATE TABLE IF NOT EXISTS auth_challenges (
    id TEXT PRIMARY KEY,
    address TEXT NOT NULL,
    message TEXT NOT NULL,
    verifier_hash TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS challenges_expiry ON auth_challenges(expires_at);
CREATE INDEX IF NOT EXISTS challenges_address ON auth_challenges(address);
CREATE TABLE IF NOT EXISTS auth_sessions (
    token_hash TEXT PRIMARY KEY,
    address TEXT NOT NULL,
    chain_id INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_expiry ON auth_sessions(expires_at);
PRAGMA user_version = 1;
