CREATE TABLE social_likes (
 post_id TEXT NOT NULL,owner TEXT NOT NULL,enabled INTEGER NOT NULL,
 issued_at INTEGER NOT NULL,nonce TEXT NOT NULL,payload TEXT NOT NULL,signature TEXT NOT NULL,
 PRIMARY KEY(post_id,owner)
);
CREATE INDEX likes_count ON social_likes(post_id,enabled);
CREATE TABLE social_like_sync (
 post_id TEXT NOT NULL,owner TEXT NOT NULL,author TEXT NOT NULL,enabled INTEGER NOT NULL,
 issued_at INTEGER NOT NULL,payload TEXT NOT NULL,signature TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending',attempts INTEGER NOT NULL DEFAULT 0,next_attempt INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(post_id,owner)
);
CREATE TABLE social_like_denials (
 post_id TEXT NOT NULL REFERENCES social_posts(id),viewer TEXT NOT NULL,PRIMARY KEY(post_id,viewer)
);
PRAGMA user_version=7;
