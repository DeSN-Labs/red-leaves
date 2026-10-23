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
        6
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
        (7, "PRAGMA user_version=7"),
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
