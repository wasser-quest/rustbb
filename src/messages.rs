//! Private messages between members. Off until an admin switches them on (see admin.rs). A
//! conversation has its members and their messages; only members can open it.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::admin::messages_switched_on;
use crate::auth::{MaybeViewer, Viewer};
use crate::error::AppError;
use crate::threads::checked;
use crate::{validate, views, views_community, AppState};

#[derive(Debug, sqlx::FromRow)]
pub struct InboxRow {
    pub id: i64,
    pub subject: String,
    pub others: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub unread: bool,
}

#[derive(Debug, sqlx::FromRow)]
pub struct MessageRow {
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

/// The signed-in member, if private messages are on. Otherwise a page that says so.
async fn member(app: &AppState, viewer: Option<Viewer>) -> Result<Result<Viewer, Response>, AppError> {
    let Some(viewer) = viewer else {
        return Ok(Err(crate::views::page("Log in", None, crate::views_forms::login(None, "")).into_response()));
    };
    if !messages_switched_on(app).await? {
        let markup = maud::html! { h1 { "Private messages" } p.meta { "Private messages are switched off on this site." } };
        return Ok(Err((StatusCode::FORBIDDEN, views::page("Private messages", Some(&viewer), markup)).into_response()));
    }
    Ok(Ok(viewer))
}

pub async fn inbox(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Result<Response, AppError> {
    let viewer = match member(&app, viewer).await? {
        Ok(viewer) => viewer,
        Err(response) => return Ok(response),
    };
    let rows = sqlx::query_as::<_, InboxRow>(
        "SELECT c.id, c.subject,
                (SELECT string_agg(u.username, ', ' ORDER BY u.username)
                   FROM conversation_members o JOIN users u ON u.id = o.user_id
                  WHERE o.conversation_id = c.id AND o.user_id <> $1) AS others,
                c.updated_at,
                EXISTS (SELECT 1 FROM messages m
                         WHERE m.conversation_id = c.id AND m.user_id <> $1 AND m.created_at > me.read_at) AS unread
           FROM conversation_members me JOIN conversations c ON c.id = me.conversation_id
          WHERE me.user_id = $1
          ORDER BY c.updated_at DESC
          LIMIT 100",
    )
    .bind(viewer.user_id)
    .fetch_all(&app.pool)
    .await?;
    Ok(views::page("Messages", Some(&viewer), views_community::inbox(&rows)).into_response())
}

pub async fn compose_form(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer, Path(username): Path<String>) -> Result<Response, AppError> {
    let viewer = match member(&app, viewer).await? {
        Ok(viewer) => viewer,
        Err(response) => return Ok(response),
    };
    find_recipient(&app, &viewer, &username).await?;
    let markup = views_community::compose(&viewer, &username, None, "", "");
    Ok(views::page("New message", Some(&viewer), markup).into_response())
}

#[derive(Deserialize)]
pub struct ComposeForm {
    csrf: String,
    subject: String,
    body: String,
}

pub async fn send_new(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(username): Path<String>,
    axum::Form(form): axum::Form<ComposeForm>,
) -> Result<Response, AppError> {
    let viewer = match member(&app, viewer).await? {
        Ok(viewer) => checked(Some(viewer), &form.csrf)?,
        Err(response) => return Ok(response),
    };
    let recipient = find_recipient(&app, &viewer, &username).await?;
    let subject = match validate::title(&form.subject) {
        Ok(subject) => subject,
        Err(error) => return Ok(compose_error(&viewer, &username, error, &form)),
    };
    let body = match validate::body(&form.body) {
        Ok(body) => body,
        Err(error) => return Ok(compose_error(&viewer, &username, error, &form)),
    };
    if !app.limiter.allow(&format!("message:{}", viewer.user_id), 20, Duration::from_secs(600)) {
        return Ok(compose_error(&viewer, &username, "You are sending very fast. Wait a moment.", &form));
    }
    let mut tx = app.pool.begin().await?;
    let (conversation_id,): (i64,) = sqlx::query_as("INSERT INTO conversations (subject) VALUES ($1) RETURNING id")
        .bind(subject)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO conversation_members (conversation_id, user_id) VALUES ($1, $2), ($1, $3)")
        .bind(conversation_id)
        .bind(viewer.user_id)
        .bind(recipient)
        .execute(&mut *tx)
        .await?;
    add_message(&mut tx, conversation_id, viewer.user_id, body).await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/messages/{conversation_id}")).into_response())
}

/// The member a message goes to: someone else, who exists.
async fn find_recipient(app: &AppState, viewer: &Viewer, username: &str) -> Result<i64, AppError> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM users WHERE username = $1 AND id <> $2")
        .bind(username)
        .bind(viewer.user_id)
        .fetch_optional(&app.pool)
        .await?;
    row.map(|(id,)| id).ok_or(AppError::NotFound)
}

fn compose_error(viewer: &Viewer, username: &str, error: &str, form: &ComposeForm) -> Response {
    let markup = views_community::compose(viewer, username, Some(error), &form.subject, &form.body);
    (StatusCode::UNPROCESSABLE_ENTITY, views::page("New message", Some(viewer), markup)).into_response()
}

/// Writes a message and tells the other members of the conversation about it.
async fn add_message(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    conversation_id: i64,
    user_id: i64,
    body: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO messages (conversation_id, user_id, body) VALUES ($1, $2, $3)")
        .bind(conversation_id)
        .bind(user_id)
        .bind(body)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE conversations SET updated_at = now() WHERE id = $1")
        .bind(conversation_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO notifications (user_id, kind, actor_id, conversation_id)
         SELECT m.user_id, 'message', $2, $1 FROM conversation_members m
          WHERE m.conversation_id = $1 AND m.user_id <> $2",
    )
    .bind(conversation_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The subject and the messages of a conversation the viewer is a member of. Opening it marks it read.
pub async fn conversation(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer, Path(id): Path<i64>) -> Result<Response, AppError> {
    let viewer = match member(&app, viewer).await? {
        Ok(viewer) => viewer,
        Err(response) => return Ok(response),
    };
    let subject = member_subject(&app, id, viewer.user_id).await?;
    let messages = sqlx::query_as::<_, MessageRow>(
        "SELECT u.username AS author, m.body, m.created_at
           FROM messages m JOIN users u ON u.id = m.user_id
          WHERE m.conversation_id = $1 ORDER BY m.id LIMIT 500",
    )
    .bind(id)
    .fetch_all(&app.pool)
    .await?;
    sqlx::query("UPDATE conversation_members SET read_at = now() WHERE conversation_id = $1 AND user_id = $2")
        .bind(id)
        .bind(viewer.user_id)
        .execute(&app.pool)
        .await?;
    sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $2 AND conversation_id = $1 AND read_at IS NULL")
        .bind(id)
        .bind(viewer.user_id)
        .execute(&app.pool)
        .await?;
    let markup = views_community::conversation(id, &subject, &messages, &viewer);
    Ok(views::page(&subject, Some(&viewer), markup).into_response())
}

async fn member_subject(app: &AppState, id: i64, user_id: i64) -> Result<String, AppError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT c.subject FROM conversations c JOIN conversation_members m ON m.conversation_id = c.id
          WHERE c.id = $1 AND m.user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&app.pool)
    .await?;
    row.map(|(subject,)| subject).ok_or(AppError::NotFound)
}

#[derive(Deserialize)]
pub struct ReplyForm {
    csrf: String,
    body: String,
}

pub async fn reply(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(id): Path<i64>,
    axum::Form(form): axum::Form<ReplyForm>,
) -> Result<Response, AppError> {
    let viewer = match member(&app, viewer).await? {
        Ok(viewer) => checked(Some(viewer), &form.csrf)?,
        Err(response) => return Ok(response),
    };
    member_subject(&app, id, viewer.user_id).await?;
    let body = match validate::body(&form.body) {
        Ok(body) => body,
        Err(error) => return Ok((StatusCode::UNPROCESSABLE_ENTITY, error).into_response()),
    };
    if !app.limiter.allow(&format!("message:{}", viewer.user_id), 20, Duration::from_secs(600)) {
        return Ok((StatusCode::TOO_MANY_REQUESTS, "You are sending very fast. Wait a moment.").into_response());
    }
    let mut tx = app.pool.begin().await?;
    add_message(&mut tx, id, viewer.user_id, body).await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/messages/{id}")).into_response())
}
