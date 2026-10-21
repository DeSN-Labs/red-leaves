CREATE TABLE IF NOT EXISTS social_challenges (
 id TEXT PRIMARY KEY, owner TEXT NOT NULL, payload_hash TEXT NOT NULL, expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS social_challenges_owner ON social_challenges(owner, expires_at);
CREATE TABLE IF NOT EXISTS social_posts (
 id TEXT PRIMARY KEY, author TEXT NOT NULL, title TEXT NOT NULL, price_wei TEXT NOT NULL,
 distribution TEXT NOT NULL CHECK(distribution IN ('broadcast','directed')),
 created_at INTEGER NOT NULL, published INTEGER NOT NULL DEFAULT 0,
 object_path TEXT NOT NULL, listing_key TEXT, market TEXT
);
CREATE INDEX IF NOT EXISTS social_posts_timeline ON social_posts(published,created_at DESC,id DESC);
CREATE TABLE IF NOT EXISTS social_recipients (
 post_id TEXT NOT NULL REFERENCES social_posts(id), recipient TEXT NOT NULL, PRIMARY KEY(post_id,recipient)
);
CREATE TABLE IF NOT EXISTS social_follows (
 owner TEXT NOT NULL, target TEXT NOT NULL, PRIMARY KEY(owner,target), CHECK(owner != target)
);
CREATE TABLE IF NOT EXISTS social_preferences (
 owner TEXT PRIMARY KEY, broadcasts INTEGER NOT NULL DEFAULT 0,
 following INTEGER NOT NULL DEFAULT 1, followers INTEGER NOT NULL DEFAULT 0,
 direct INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS social_outbox (
 post_id TEXT NOT NULL REFERENCES social_posts(id), target TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending_network', PRIMARY KEY(post_id,target)
);
PRAGMA user_version = 2;
