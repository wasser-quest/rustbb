//! Following threads, mentions and notifications. A member who follows a thread hears about its
//! replies. A mention of `@name` tells that member, once, even if they follow the thread too.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::{PgPool, Postgres, Transaction};

use crate::auth::{MaybeViewer, Viewer};
use crate::error::AppError;
use crate::threads::checked;
use crate::{views, views_community, AppState};

const MAX_MENTIONS: usize = 10;
const LIST_LIMIT: i64 = 100;

/// The usernames mentioned as `@name` in a text: 3 to 32 letters, digits, `_` or `-` after an `@`
/// that starts a word. Each name once, and at most MAX_MENTIONS of them.
pub fn mentions(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let Some(name) = word.strip_prefix('@') else {
            continue;
        };
        let name = name.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
        let plausible = (3..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if plausible && !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
        if names.len() == MAX_MENTIONS {
            break;
        }
    }
    names
}

/// Records a new reply: the author follows the thread, mentioned members are told, and the other
/// followers are told about the reply.
pub async fn record_reply(
    tx: &mut Transaction<'_, Postgres>,
    thread_id: i64,
    post_id: i64,
    author_id: i64,
    body: &str,
) -> Result<(), sqlx::Error> {
    follow_in(&mut **tx, author_id, thread_id).await?;
    let names = mentions(body);
    let mentioned: Vec<(i64,)> = sqlx::query_as("SELECT id FROM users WHERE username = ANY($1) AND id <> $2")
        .bind(&names)
        .bind(author_id)
        .fetch_all(&mut **tx)
        .await?;
    let mentioned: Vec<i64> = mentioned.into_iter().map(|(id,)| id).collect();
    sqlx::query(
        "INSERT INTO notifications (user_id, kind, actor_id, thread_id, post_id)
         SELECT id, 'mention', $2, $3, $4 FROM unnest($1::bigint[]) AS id",
    )
    .bind(&mentioned)
    .bind(author_id)
    .bind(thread_id)
    .bind(post_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO notifications (user_id, kind, actor_id, thread_id, post_id)
         SELECT s.user_id, 'reply', $2, $1, $3 FROM thread_subscriptions s
          WHERE s.thread_id = $1 AND s.user_id <> $2 AND s.user_id <> ALL($4)",
    )
    .bind(thread_id)
    .bind(author_id)
    .bind(post_id)
    .bind(&mentioned)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn follow_in<'c>(db: impl sqlx::PgExecutor<'c>, user_id: i64, thread_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO thread_subscriptions (user_id, thread_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(thread_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn is_following(pool: &PgPool, user_id: i64, thread_id: i64) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT 1::bigint FROM thread_subscriptions WHERE user_id = $1 AND thread_id = $2")
        .bind(user_id)
        .bind(thread_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

#[derive(Deserialize)]
pub struct FollowForm {
    csrf: String,
    action: String,
}

pub async fn toggle_follow(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(thread_id): Path<i64>,
    axum::Form(form): axum::Form<FollowForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM threads WHERE id = $1")
        .bind(thread_id)
        .fetch_optional(&app.pool)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }
    if form.action == "unfollow" {
        sqlx::query("DELETE FROM thread_subscriptions WHERE user_id = $1 AND thread_id = $2")
            .bind(viewer.user_id)
            .bind(thread_id)
            .execute(&app.pool)
            .await?;
    } else {
        follow_in(&app.pool, viewer.user_id, thread_id).await?;
    }
    Ok(Redirect::to(&format!("/threads/{thread_id}")).into_response())
}

#[derive(Debug, sqlx::FromRow)]
pub struct NotificationRow {
    pub kind: String,
    pub actor: Option<String>,
    pub thread_id: Option<i64>,
    pub thread_title: Option<String>,
    pub post_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub unread: bool,
}

pub async fn notifications(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Result<Response, AppError> {
    let Some(viewer) = viewer else {
        return Ok(Redirect::to("/login").into_response());
    };
    let rows = sqlx::query_as::<_, NotificationRow>(
        "SELECT n.kind, a.username AS actor, n.thread_id, t.title AS thread_title, n.post_id,
                n.conversation_id, n.created_at, n.read_at IS NULL AS unread
           FROM notifications n
           LEFT JOIN users a ON a.id = n.actor_id
           LEFT JOIN threads t ON t.id = n.thread_id
          WHERE n.user_id = $1
          ORDER BY n.id DESC
          LIMIT $2",
    )
    .bind(viewer.user_id)
    .bind(LIST_LIMIT)
    .fetch_all(&app.pool)
    .await?;
    Ok(views::page("Notifications", Some(&viewer), views_community::notifications(&rows, &viewer.csrf)).into_response())
}

#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

pub async fn read_all(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    axum::Form(form): axum::Form<CsrfForm>,
) -> Result<Response, AppError> {
    let viewer: Viewer = checked(viewer, &form.csrf)?;
    sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND read_at IS NULL")
        .bind(viewer.user_id)
        .execute(&app.pool)
        .await?;
    Ok(Redirect::to("/notifications").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_names_after_an_at_sign() {
        assert_eq!(mentions("hi @alice, and @bob_2!"), vec!["alice", "bob_2"]);
        assert_eq!(mentions("mail me at a@example.org"), Vec::<String>::new());
        assert_eq!(mentions("@ab @alice @alice"), vec!["alice"]);
        assert_eq!(mentions(&"@xyz ".repeat(20)).len(), 1);
    }

    #[test]
    fn at_most_ten_names_count() {
        let text: String = (0..15).map(|i| format!("@user{i:02} ")).collect();
        assert_eq!(mentions(&text).len(), MAX_MENTIONS);
    }
}
