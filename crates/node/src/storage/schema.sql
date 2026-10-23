CREATE TABLE auth_challenges (
 id TEXT PRIMARY KEY, address TEXT NOT NULL, message TEXT NOT NULL,
 verifier_hash TEXT NOT NULL, expires_at INTEGER NOT NULL
);
CREATE INDEX challenges_expiry ON auth_challenges(expires_at);
CREATE INDEX challenges_address ON auth_challenges(address);
CREATE TABLE auth_sessions (
 token_hash TEXT PRIMARY KEY, address TEXT NOT NULL, chain_id INTEGER NOT NULL,
 created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_expiry ON auth_sessions(expires_at);

CREATE TABLE social_challenges (
 id TEXT PRIMARY KEY, owner TEXT NOT NULL, payload_hash TEXT NOT NULL, expires_at INTEGER NOT NULL
);
CREATE INDEX social_challenges_owner ON social_challenges(owner, expires_at);
CREATE TABLE social_posts (
 id TEXT PRIMARY KEY, author TEXT NOT NULL, title TEXT NOT NULL,
 distribution TEXT NOT NULL CHECK(distribution IN ('broadcast','directed')),
 audience_scope TEXT NOT NULL, created_at INTEGER NOT NULL, published INTEGER NOT NULL,
 object_path TEXT NOT NULL
);
CREATE INDEX social_posts_timeline ON social_posts(published,created_at DESC,id DESC);
CREATE TABLE social_recipients (
 post_id TEXT NOT NULL REFERENCES social_posts(id), recipient TEXT NOT NULL, PRIMARY KEY(post_id,recipient)
);
CREATE TABLE social_follows (
 owner TEXT NOT NULL, target TEXT NOT NULL, PRIMARY KEY(owner,target), CHECK(owner != target)
);
CREATE TABLE social_preferences (
 owner TEXT PRIMARY KEY, broadcasts INTEGER NOT NULL DEFAULT 0,
 following INTEGER NOT NULL DEFAULT 1, followers INTEGER NOT NULL DEFAULT 0,
 direct INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE social_outbox (
 post_id TEXT NOT NULL REFERENCES social_posts(id), target TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending_network', attempts INTEGER NOT NULL DEFAULT 0,
 next_attempt INTEGER NOT NULL DEFAULT 0, last_error TEXT, PRIMARY KEY(post_id,target)
);
CREATE TABLE social_media (
 id TEXT PRIMARY KEY, owner TEXT NOT NULL, mime TEXT NOT NULL, size INTEGER NOT NULL,
 digest TEXT NOT NULL, object_path TEXT NOT NULL, envelope TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE INDEX social_media_owner ON social_media(owner,created_at);
CREATE TABLE social_post_media (
 post_id TEXT NOT NULL REFERENCES social_posts(id), media_id TEXT NOT NULL REFERENCES social_media(id),
 position INTEGER NOT NULL, PRIMARY KEY(post_id,media_id), UNIQUE(post_id,position)
);
CREATE TABLE social_accounts(owner TEXT PRIMARY KEY);
CREATE TABLE social_inbox (
 post_id TEXT NOT NULL, recipient TEXT NOT NULL, author TEXT NOT NULL, source_peer TEXT NOT NULL,
 notice TEXT NOT NULL, received_at INTEGER NOT NULL, created_at INTEGER NOT NULL,
 object_path TEXT, PRIMARY KEY(post_id,recipient)
);
CREATE INDEX social_inbox_timeline ON social_inbox(recipient,created_at DESC,post_id DESC);
CREATE TABLE social_blocks(owner TEXT NOT NULL,target TEXT NOT NULL,PRIMARY KEY(owner,target));
CREATE TABLE social_follow_sync (
 owner TEXT NOT NULL,target TEXT NOT NULL,payload TEXT NOT NULL,signature TEXT NOT NULL,
 issued_at INTEGER NOT NULL,nonce TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'pending',
 attempts INTEGER NOT NULL DEFAULT 0,next_attempt INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(owner,target)
);
CREATE TABLE social_follow_versions (
 owner TEXT NOT NULL,target TEXT NOT NULL,issued_at INTEGER NOT NULL,nonce TEXT NOT NULL,
 PRIMARY KEY(owner,target)
);
PRAGMA user_version=6;
