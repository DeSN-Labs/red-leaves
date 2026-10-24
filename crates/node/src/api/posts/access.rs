//! Current author-controlled access, independent of the immutable publication signature.
//! New content/media fetches and interactions share this rule; delivered copies stay readable.
//! Future comment admission and pulls must use this same authorization entry point.
use super::*;

// Shared by body/media authorization and atomic like reads/writes. Values remain bound parameters.
macro_rules! allowed_sql { () => { "SELECT p.id FROM social_posts p JOIN social_post_access a ON a.post_id=p.id JOIN (SELECT ? AS id, ? AS viewer) c ON p.id=c.id WHERE p.published=1 AND (p.author=c.viewer OR (
 NOT EXISTS(SELECT 1 FROM social_blocks b WHERE b.owner=p.author AND b.target=c.viewer) AND
 (a.level='public' OR
 (a.level='following' AND EXISTS(SELECT 1 FROM social_follows f WHERE f.owner=p.author AND f.target=c.viewer)) OR
 (a.level='followers' AND EXISTS(SELECT 1 FROM social_follows f WHERE f.owner=c.viewer AND f.target=p.author)) OR
 (a.level='directed' AND EXISTS(SELECT 1 FROM social_post_access_recipients r WHERE r.post_id=p.id AND r.viewer=c.viewer)))))" }; }
pub(super) use allowed_sql;

pub(super) async fn ensure(state: &AppState, id: &str, viewer: &str) -> Result<(), ApiError> {
    let allowed: bool = sqlx::query_scalar(concat!("SELECT EXISTS(", allowed_sql!(), ")"))
        .bind(id)
        .bind(viewer)
        .fetch_one(&state.pool)
        .await?;
    if !allowed {
        return Err(forbidden());
    }
    Ok(())
}
pub(super) async fn revision(state: &AppState, id: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar("SELECT revision FROM social_post_access WHERE post_id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(forbidden)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Level {
    None,
    Followers,
    Following,
    Directed,
    Public,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    level: Level,
    #[serde(default)]
    recipients: Vec<String>,
}
async fn author(state: &AppState, id: &str, viewer: &str) -> Result<(), ApiError> {
    let _ = delivery::cid(id)?;
    let actual: String =
        sqlx::query_scalar("SELECT author FROM social_posts WHERE id=? AND published=1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(forbidden)?;
    if actual != viewer || delivery::profile(state, viewer).await?.peer != state.config.peer_id {
        return Err(forbidden());
    }
    Ok(())
}
pub(super) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    author(&state, &id, &viewer).await?;
    let row = sqlx::query("SELECT level,revision FROM social_post_access WHERE post_id=?")
        .bind(&id)
        .fetch_one(&state.pool)
        .await?;
    let recipients: Vec<String> = sqlx::query_scalar(
        "SELECT viewer FROM social_post_access_recipients WHERE post_id=? ORDER BY viewer",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({"level":row.get::<String,_>("level"),"revision":row.get::<i64,_>("revision"),"recipients":recipients}),
    ))
}
pub(super) async fn set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Policy>,
) -> Result<Json<Value>, ApiError> {
    let viewer = owner(&state, headers).await?;
    author(&state, &id, &viewer).await?;
    let level = serde_json::to_value(&input.level)
        .map_err(|_| bad())?
        .as_str()
        .ok_or_else(bad)?
        .to_owned();
    if input.recipients.len() > 32
        || (matches!(input.level, Level::Directed) == input.recipients.is_empty())
        || (!matches!(input.level, Level::Directed) && !input.recipients.is_empty())
    {
        return Err(bad());
    }
    let mut recipients = std::collections::BTreeSet::new();
    for recipient in input.recipients {
        let recipient = wallet(address(&recipient)?);
        if recipient == viewer || address(&recipient)?.0 == [0; 20] || !recipients.insert(recipient)
        {
            return Err(bad());
        }
    }
    let mut tx = state.pool.begin().await?;
    let next:i64=sqlx::query_scalar("UPDATE social_post_access SET level=?,revision=revision+1 WHERE post_id=? RETURNING revision").bind(&level).bind(&id).fetch_one(&mut *tx).await?;
    sqlx::query("DELETE FROM social_post_access_recipients WHERE post_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    for recipient in &recipients {
        sqlx::query("INSERT INTO social_post_access_recipients(post_id,viewer) VALUES(?,?)")
            .bind(&id)
            .bind(recipient)
            .execute(&mut *tx)
            .await?;
    }
    // A running old delivery must not acknowledge or revive tasks created for a newer policy.
    sqlx::query("UPDATE social_outbox SET state='filtered',access_revision=?,last_error='Access policy changed' WHERE post_id=?").bind(next).bind(&id).execute(&mut *tx).await?;
    let targets: Vec<String> = match input.level {
        Level::None => vec![],
        Level::Public => vec!["broadcast".into()],
        Level::Directed => recipients.into_iter().collect(),
        Level::Following => {
            sqlx::query_scalar(
                "SELECT target FROM social_follows WHERE owner=? ORDER BY target LIMIT 32",
            )
            .bind(&viewer)
            .fetch_all(&mut *tx)
            .await?
        }
        Level::Followers => {
            sqlx::query_scalar(
                "SELECT owner FROM social_follows WHERE target=? ORDER BY owner LIMIT 32",
            )
            .bind(&viewer)
            .fetch_all(&mut *tx)
            .await?
        }
    };
    for target in targets {
        sqlx::query("INSERT INTO social_outbox(post_id,target,access_revision) VALUES(?,?,?) ON CONFLICT(post_id,target) DO UPDATE SET state='pending_network',attempts=0,next_attempt=0,last_error=NULL,access_revision=excluded.access_revision").bind(&id).bind(target).bind(next).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    service(&state)?.runtime.changed(&viewer);
    service(&state)?.runtime.wake.notify_one();
    Ok(Json(json!({"status":"saved","revision":next})))
}
