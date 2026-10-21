ALTER TABLE social_posts ADD COLUMN audience_scope TEXT;
CREATE TABLE social_media (
 id TEXT PRIMARY KEY, owner TEXT NOT NULL, mime TEXT NOT NULL, size INTEGER NOT NULL,
 digest TEXT NOT NULL, object_path TEXT NOT NULL, envelope TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE INDEX social_media_owner ON social_media(owner,created_at);
CREATE TABLE social_post_media (
 post_id TEXT NOT NULL REFERENCES social_posts(id), media_id TEXT NOT NULL REFERENCES social_media(id),
 position INTEGER NOT NULL, PRIMARY KEY(post_id,media_id), UNIQUE(post_id,position)
);
PRAGMA user_version = 3;
