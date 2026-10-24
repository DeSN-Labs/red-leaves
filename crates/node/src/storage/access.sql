CREATE TABLE social_post_access (
 post_id TEXT PRIMARY KEY REFERENCES social_posts(id),
 level TEXT NOT NULL CHECK(level IN ('none','followers','following','directed','public')),
 revision INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE social_post_access_recipients (
 post_id TEXT NOT NULL REFERENCES social_post_access(post_id),viewer TEXT NOT NULL,
 PRIMARY KEY(post_id,viewer)
);
INSERT INTO social_post_access(post_id,level)
 SELECT id,CASE audience_scope WHEN 'broadcast' THEN 'public' WHEN 'followers' THEN 'followers' WHEN 'following' THEN 'following' ELSE 'directed' END FROM social_posts;
INSERT INTO social_post_access_recipients(post_id,viewer)
 SELECT r.post_id,r.recipient FROM social_recipients r JOIN social_post_access a ON a.post_id=r.post_id WHERE a.level='directed';
ALTER TABLE social_outbox ADD COLUMN access_revision INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version=8;
