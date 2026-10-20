//! Writing to a thread: new threads and replies (with text, images and a poll), deleting and
//! reporting posts. Reading is in threads.rs.

use std::time::Duration;

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::auth::{MaybeViewer, Viewer};
use crate::error::AppError;
use crate::forms::{self, PostForm};
use crate::models::{Board, PostRow, ThreadHead};
use crate::threads::{attachments_for, board_by_slug, checked, thread_head, POSTS_PER_PAGE};
use crate::{notify, polls, reactions, upload, validate, views, views_forms, AppState};

/// Checks the files, stores them, and returns them with their new ids and content types.
/// Nothing is stored unless every file passes.
async fn store_post_files(app: &AppState, user_id: i64, files: &[bytes::Bytes]) -> Result<Vec<(String, &'static str, usize, bytes::Bytes)>, &'static str> {
    let mut prepared = Vec::new();
    for bytes in files {
        let content_type = upload::check(bytes)?;
        // Camera and location data go before anything is stored.
        let cleaned = bytes::Bytes::from(crate::metadata::strip(bytes, content_type)?);
        prepared.push((upload::new_id(), content_type, cleaned.len(), cleaned));
    }
    // The limits per user and for all files together come before anything is stored.
    if !prepared.is_empty() {
        let sizes: Vec<usize> = prepared.iter().map(|file| file.2).collect();
        crate::quota::check(&app.pool, app.quota, user_id, &sizes).await?;
    }
    let mut stored: Vec<String> = Vec::new();
    for (id, _, _, bytes) in &prepared {
        if let Err(error) = app.storage.put(id, bytes.clone()).await {
            eprintln!("attachment upload failed: {error}");
            for done in &stored {
                let _ = app.storage.delete(done).await;
            }
            return Err("The file could not be stored. Try again later.");
        }
        stored.push(id.clone());
    }
    Ok(prepared)
}

async fn remove_objects(app: &AppState, ids: &[String]) {
    for id in ids {
        let _ = app.storage.delete(id).await;
    }
}

pub async fn create_thread(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(slug): Path<String>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let board = board_by_slug(&app, &slug).await?;
    let form: PostForm = match forms::read_post(multipart).await {
        Ok(form) => form,
        Err(error) => return Ok(post_error(&board, viewer.as_ref(), error, &PostForm::default())),
    };
    let viewer = checked(viewer, &form.csrf)?;
    if !app.limiter.allow(&format!("post:{}", viewer.user_id), 20, Duration::from_secs(600)) {
        return Ok(post_error(&board, Some(&viewer), "You are posting very fast. Wait a moment.", &form));
    }
    let title = match validate::title(&form.title) {
        Ok(title) => title,
        Err(error) => return Ok(post_error(&board, Some(&viewer), error, &form)),
    };
    let body = match validate::body(&form.body) {
        Ok(body) => body,
        Err(error) => return Ok(post_error(&board, Some(&viewer), error, &form)),
    };
    let poll = match polls::parse(&form.poll_question, &form.poll_options) {
        Ok(poll) => poll,
        Err(error) => return Ok(post_error(&board, Some(&viewer), error, &form)),
    };
    let files = match store_post_files(&app, viewer.user_id, &form.files).await {
        Ok(files) => files,
        Err(error) => return Ok(post_error(&board, Some(&viewer), error, &form)),
    };
    let ids: Vec<String> = files.iter().map(|f| f.0.clone()).collect();
    let mut tx = app.pool.begin().await?;
    let inserted = async {
        let (thread_id,): (i64,) = sqlx::query_as(
            "INSERT INTO threads (board_id, user_id, title) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(board.id)
        .bind(viewer.user_id)
        .bind(title)
        .fetch_one(&mut *tx)
        .await?;
        let (post_id,): (i64,) = sqlx::query_as(
            "INSERT INTO posts (thread_id, user_id, body) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(thread_id)
        .bind(viewer.user_id)
        .bind(body)
        .fetch_one(&mut *tx)
        .await?;
        insert_attachments(&mut tx, post_id, viewer.user_id, &files).await?;
        if let Some((question, options)) = &poll {
            polls::create(&mut tx, thread_id, question, options).await?;
        }
        notify::follow_in(&mut *tx, viewer.user_id, thread_id).await?;
        Ok::<i64, sqlx::Error>(thread_id)
    }
    .await;
    match inserted {
        Ok(thread_id) => {
            tx.commit().await?;
            Ok(Redirect::to(&format!("/threads/{thread_id}")).into_response())
        }
        Err(error) => {
            remove_objects(&app, &ids).await;
            Err(error.into())
        }
    }
}

async fn insert_attachments(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    post_id: i64,
    user_id: i64,
    files: &[(String, &'static str, usize, bytes::Bytes)],
) -> Result<(), sqlx::Error> {
    for (id, content_type, size, _) in files {
        sqlx::query(
            "INSERT INTO attachments (id, post_id, user_id, content_type, size_bytes) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(post_id)
        .bind(user_id)
        .bind(content_type)
        .bind(*size as i64)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// The new-thread form again, with the message and what was typed, so nothing has to be retyped.
fn post_error(board: &Board, viewer: Option<&Viewer>, error: &str, form: &PostForm) -> Response {
    let markup = match viewer {
        Some(viewer) => views_forms::new_thread(board, viewer, Some(error), &form.title, &form.body, &form.poll_question, &form.poll_options),
        None => views::boards(std::slice::from_ref(board)),
    };
    (StatusCode::UNPROCESSABLE_ENTITY, views::page("New thread", viewer, markup)).into_response()
}

pub async fn reply(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let head = thread_head(&app, id).await?;
    let form = match forms::read_post(multipart).await {
        Ok(form) => form,
        Err(error) => return reply_error(&app, &head, viewer.as_ref(), error).await,
    };
    let viewer = checked(viewer, &form.csrf)?;
    if !app.limiter.allow(&format!("post:{}", viewer.user_id), 20, Duration::from_secs(600)) {
        return reply_error(&app, &head, Some(&viewer), "You are posting very fast. Wait a moment.").await;
    }
    let body = match validate::body(&form.body) {
        Ok(body) => body,
        Err(error) => return reply_error(&app, &head, Some(&viewer), error).await,
    };
    let files = match store_post_files(&app, viewer.user_id, &form.files).await {
        Ok(files) => files,
        Err(error) => return reply_error(&app, &head, Some(&viewer), error).await,
    };
    let ids: Vec<String> = files.iter().map(|f| f.0.clone()).collect();
    let mut tx = app.pool.begin().await?;
    let inserted = async {
        let (post_id,): (i64,) = sqlx::query_as(
            "INSERT INTO posts (thread_id, user_id, body) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(id)
        .bind(viewer.user_id)
        .bind(body)
        .fetch_one(&mut *tx)
        .await?;
        insert_attachments(&mut tx, post_id, viewer.user_id, &files).await?;
        sqlx::query("UPDATE threads SET last_post_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        notify::record_reply(&mut tx, id, post_id, viewer.user_id, body).await?;
        Ok::<(), sqlx::Error>(())
    }
    .await;
    match inserted {
        Ok(()) => {
            tx.commit().await?;
            Ok(Redirect::to(&format!("/threads/{id}")).into_response())
        }
        Err(error) => {
            remove_objects(&app, &ids).await;
            Err(error.into())
        }
    }
}

async fn reply_error(app: &AppState, head: &ThreadHead, viewer: Option<&Viewer>, error: &str) -> Result<Response, AppError> {
    let posts = sqlx::query_as::<_, PostRow>(
        "SELECT p.id, u.username AS author, u.id AS author_id, u.title AS author_title,
                u.avatar_id,
                (SELECT count(*) FROM posts pp WHERE pp.user_id = u.id AND pp.deleted_at IS NULL) AS author_posts,
                p.body, p.created_at, false AS deleted
           FROM posts p JOIN users u ON u.id = p.user_id WHERE p.thread_id = $1 ORDER BY p.id LIMIT $2",
    )
    .bind(head.id)
    .bind(i64::from(POSTS_PER_PAGE))
    .fetch_all(&app.pool)
    .await?;
    let attachments = attachments_for(app, &posts).await?;
    let viewer_id = viewer.map(|viewer| viewer.user_id);
    let poll = polls::load(&app.pool, head.id, viewer_id).await?;
    let following = match viewer_id {
        Some(user_id) => notify::is_following(&app.pool, user_id, head.id).await?,
        None => false,
    };
    let post_ids: Vec<i64> = posts.iter().map(|post| post.id).collect();
    let kinds = reactions::kinds(&app.pool).await?;
    let tallies = reactions::tallies(&app.pool, &post_ids, viewer_id).await?;
    let extras = views::ThreadExtras { poll: poll.as_ref(), following, kinds: &kinds, tallies: &tallies };
    let mut markup = views::thread(head, &posts, &attachments, viewer, 1, false, &extras);
    if let Some(viewer) = viewer {
        markup = maud::html! { (markup) (views_forms::reply_box(head, viewer, Some(error))) };
    }
    Ok((StatusCode::UNPROCESSABLE_ENTITY, views::page(&head.title, viewer, markup)).into_response())
}

#[derive(Deserialize)]
pub struct ActionForm {
    csrf: String,
    thread: Option<i64>,
    reason: Option<String>,
}

/// Where to go back to after a post action: its thread, if the form said which.
fn back_to(form: &ActionForm, post_id: i64) -> String {
    match form.thread {
        Some(thread) => format!("/threads/{thread}#post-{post_id}"),
        None => "/".to_owned(),
    }
}

pub async fn delete_post(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(post_id): Path<i64>,
    axum::Form(form): axum::Form<ActionForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    let owner: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM posts WHERE id = $1 AND deleted_at IS NULL")
        .bind(post_id)
        .fetch_optional(&app.pool)
        .await?;
    let Some((owner,)) = owner else {
        return Err(AppError::NotFound);
    };
    if owner != viewer.user_id {
        return Err(AppError::Forbidden);
    }
    remove_post(&app, post_id).await?;
    Ok(Redirect::to(&back_to(&form, post_id)).into_response())
}

/// Marks a post as removed and deletes its files. The row stays, so thread numbering holds.
pub async fn remove_post(app: &AppState, post_id: i64) -> Result<(), AppError> {
    let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM attachments WHERE post_id = $1")
        .bind(post_id)
        .fetch_all(&app.pool)
        .await?;
    sqlx::query("UPDATE posts SET body = '', deleted_at = now() WHERE id = $1")
        .bind(post_id)
        .execute(&app.pool)
        .await?;
    sqlx::query("DELETE FROM attachments WHERE post_id = $1")
        .bind(post_id)
        .execute(&app.pool)
        .await?;
    for (id,) in ids {
        let _ = app.storage.delete(&id).await;
    }
    Ok(())
}

pub async fn report_post(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(post_id): Path<i64>,
    axum::Form(form): axum::Form<ActionForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    if !app.limiter.allow(&format!("report:{}", viewer.user_id), 10, Duration::from_secs(3600)) {
        return Err(AppError::Forbidden);
    }
    let reason: String = form.reason.clone().unwrap_or_default().trim().chars().take(300).collect();
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM posts WHERE id = $1 AND deleted_at IS NULL")
        .bind(post_id)
        .fetch_optional(&app.pool)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }
    sqlx::query(
        "INSERT INTO reports (post_id, user_id, reason) VALUES ($1, $2, $3)
         ON CONFLICT (post_id, user_id) DO NOTHING",
    )
    .bind(post_id)
    .bind(viewer.user_id)
    .bind(reason)
    .execute(&app.pool)
    .await?;
    Ok(Redirect::to(&back_to(&form, post_id)).into_response())
}
