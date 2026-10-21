CREATE TABLE social_accounts(owner TEXT PRIMARY KEY);
INSERT OR IGNORE INTO social_accounts SELECT DISTINCT address FROM auth_sessions;
INSERT OR IGNORE INTO social_accounts SELECT DISTINCT author FROM social_posts;
INSERT OR IGNORE INTO social_accounts SELECT DISTINCT owner FROM social_preferences;
CREATE TABLE social_inbox (
 post_id TEXT NOT NULL, recipient TEXT NOT NULL, author TEXT NOT NULL, source_peer TEXT NOT NULL,
 notice TEXT NOT NULL, received_at INTEGER NOT NULL, created_at INTEGER NOT NULL,
 object_path TEXT, PRIMARY KEY(post_id,recipient)
);
CREATE INDEX social_inbox_timeline ON social_inbox(recipient,created_at DESC,post_id DESC);
CREATE TABLE social_blocks(owner TEXT NOT NULL,target TEXT NOT NULL,PRIMARY KEY(owner,target));
ALTER TABLE social_outbox ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE social_outbox ADD COLUMN next_attempt INTEGER NOT NULL DEFAULT 0;
ALTER TABLE social_outbox ADD COLUMN last_error TEXT;
CREATE TABLE social_follow_sync (
 owner TEXT NOT NULL,target TEXT NOT NULL,payload TEXT NOT NULL,signature TEXT NOT NULL,
 issued_at INTEGER NOT NULL,nonce TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'pending',
 attempts INTEGER NOT NULL DEFAULT 0,next_attempt INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(owner,target)
);
CREATE TABLE social_follow_versions(owner TEXT NOT NULL,target TEXT NOT NULL,issued_at INTEGER NOT NULL,nonce TEXT NOT NULL,PRIMARY KEY(owner,target));
PRAGMA user_version=4;
