use super::*;
use sqlx::Row;

#[tokio::test]
async fn current_schema_initializes_and_reopens_without_migration() {
    let root = std::env::temp_dir().join(format!(
        "red-leaves-schema-{}",
        crate::identity::secret().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let url = format!("sqlite://{}", root.join("db.sqlite").display());
    let pool = open(&url).await.unwrap();
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('social_posts')")
            .fetch_all(&pool)
            .await
            .unwrap();
    for removed in ["price_wei", "listing_key", "market", "archived"] {
        assert!(!columns.iter().any(|column| column == removed));
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='legacy_post_access'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    sqlx::query("INSERT INTO social_posts(id,author,title,distribution,audience_scope,created_at,published,object_path) VALUES('post','author','Title','broadcast','broadcast',1,1,'object')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO social_outbox(post_id,target,attempts,next_attempt) VALUES('post','broadcast',2,123)")
        .execute(&pool).await.unwrap();
    pool.close().await;
    let pool = open(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap(),
        8
    );
    let row =
        sqlx::query("SELECT state,attempts,next_attempt FROM social_outbox WHERE post_id='post'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.get::<String, _>("state"), "pending_network");
    assert_eq!(row.get::<i64, _>("attempts"), 2);
    assert_eq!(row.get::<i64, _>("next_attempt"), 123);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unsupported_schema_versions_are_rejected_without_being_reinterpreted() {
    let root = std::env::temp_dir().join(format!(
        "red-leaves-schema-versions-{}",
        crate::identity::secret().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    for (version, pragma) in [
        (1, "PRAGMA user_version=1"),
        (4, "PRAGMA user_version=4"),
        (5, "PRAGMA user_version=5"),
        (9, "PRAGMA user_version=9"),
    ] {
        let url = format!(
            "sqlite://{}?mode=rwc",
            root.join(format!("v{version}.sqlite")).display()
        );
        let pool = SqlitePool::connect(&url).await.unwrap();
        sqlx::query(pragma).execute(&pool).await.unwrap();
        let error = open(&url).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unsupported database schema version")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("PRAGMA user_version")
                .fetch_one(&pool)
                .await
                .unwrap(),
            version
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type='table'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        pool.close().await;
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn version_six_adds_likes_without_losing_existing_posts() {
    let root = std::env::temp_dir().join(format!(
        "red-leaves-likes-upgrade-{}",
        crate::identity::secret().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let url = format!("sqlite://{}?mode=rwc", root.join("db.sqlite").display());
    let pool = SqlitePool::connect(&url).await.unwrap();
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO social_posts(id,author,title,distribution,audience_scope,created_at,published,object_path) VALUES('post','author','Title','broadcast','broadcast',1,1,'object')").execute(&pool).await.unwrap();
    pool.close().await;
    let pool = open(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap(),
        8
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM social_posts")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM social_likes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn version_seven_seeds_access_and_preserves_likes_and_delivery() {
    let root = std::env::temp_dir().join(format!(
        "red-leaves-access-upgrade-{}",
        crate::identity::secret().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let url = format!("sqlite://{}?mode=rwc", root.join("db.sqlite").display());
    let pool = SqlitePool::connect(&url).await.unwrap();
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("likes.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO social_posts(id,author,title,distribution,audience_scope,created_at,published,object_path) VALUES('post','author','','directed','friends',1,1,'object')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO social_recipients(post_id,recipient) VALUES('post','viewer')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO social_outbox(post_id,target) VALUES('post','viewer')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let pool = open(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT level FROM social_post_access WHERE post_id='post'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "directed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT viewer FROM social_post_access_recipients WHERE post_id='post'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "viewer"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT access_revision FROM social_outbox WHERE post_id='post'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    pool.close().await;
    let pool = open(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap(),
        8
    );
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
