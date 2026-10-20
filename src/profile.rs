//! Member profiles: a rank that follows from the number of posts, a title and a picture. The
//! member sets the picture in the settings; an admin may set the title (see admin.rs).

use std::time::Duration;

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use maud::Markup;
use serde::Deserialize;

use crate::auth::{MaybeViewer, Viewer};
use crate::error::AppError;
use crate::files;
use crate::models::{ProfileRow, ThreadRow};
use crate::threads::checked;
use crate::{upload, views, views_community, AppState};

/// A picture for the profile may be at most 1 MB.
pub const AVATAR_MAX_BYTES: usize = 1024 * 1024;

/// The rank a member has reached by their number of posts.
pub fn rank(posts: i64) -> &'static str {
    match posts {
        ..=9 => "Newcomer",
        10..=49 => "Member",
        50..=199 => "Regular",
        _ => "Veteran",
    }
}

pub async fn profile(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(username): Path<String>,
) -> Result<Markup, AppError> {
    let profile = sqlx::query_as::<_, ProfileRow>(
        "SELECT u.id, u.username, u.created_at,
                (SELECT count(*) FROM posts p WHERE p.user_id = u.id AND p.deleted_at IS NULL) AS posts,
                u.title, u.avatar_id, u.banned_until, u.ban_reason
           FROM users u WHERE u.username = $1",
    )
    .bind(&username)
    .fetch_optional(&app.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let recent = sqlx::query_as::<_, ThreadRow>(
        "SELECT t.id, t.title, u.username AS author, t.last_post_at, 0::bigint AS replies
           FROM threads t JOIN users u ON u.id = t.user_id
          WHERE u.username = $1 ORDER BY t.last_post_at DESC LIMIT 20",
    )
    .bind(&username)
    .fetch_all(&app.pool)
    .await?;
    let markup = views_community::profile(&profile, &recent, viewer.as_ref());
    Ok(views::page(&profile.username, viewer.as_ref(), markup))
}

/// The picture of a member. The id is random and changes with every new picture.
pub async fn avatar(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    if !files::is_id(&id) {
        return Err(AppError::NotFound);
    }
    let row: Option<(String,)> = sqlx::query_as("SELECT avatar_type FROM users WHERE avatar_id = $1")
        .bind(&id)
        .fetch_optional(&app.pool)
        .await?;
    let Some((content_type,)) = row else {
        return Err(AppError::NotFound);
    };
    files::stored_image(&app, &id, &content_type).await
}

pub async fn settings(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Result<Response, AppError> {
    let Some(viewer) = viewer else {
        return Ok(Redirect::to("/login").into_response());
    };
    let avatar = current_avatar(&app, viewer.user_id).await?;
    Ok(settings_page(&viewer, avatar.as_deref(), None, StatusCode::OK))
}

async fn current_avatar(app: &AppState, user_id: i64) -> Result<Option<String>, AppError> {
    let row: Option<(Option<String>,)> = sqlx::query_as("SELECT avatar_id FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&app.pool)
        .await?;
    Ok(row.and_then(|(id,)| id))
}

fn settings_page(viewer: &Viewer, avatar: Option<&str>, error: Option<&str>, status: StatusCode) -> Response {
    (status, views::page("Settings", Some(viewer), views_community::settings(viewer, avatar, error))).into_response()
}

/// The fields of the picture form: the CSRF token and the picture itself.
async fn read_avatar_form(mut multipart: Multipart) -> Result<(String, Option<bytes::Bytes>), &'static str> {
    let mut csrf = String::new();
    let mut picture = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| "The form is invalid.")? {
        match field.name().unwrap_or_default() {
            "csrf" => csrf = field.text().await.map_err(|_| "The form is invalid.")?,
            "avatar" => {
                let bytes = field.bytes().await.map_err(|_| "The picture could not be read.")?;
                if !bytes.is_empty() {
                    picture = Some(bytes);
                }
            }
            _ => {}
        }
    }
    Ok((csrf, picture))
}

pub async fn set_avatar(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer, multipart: Multipart) -> Result<Response, AppError> {
    let (csrf, picture) = match read_avatar_form(multipart).await {
        Ok(form) => form,
        Err(error) => return Ok(settings_error(&app, viewer, error).await?),
    };
    let viewer = checked(viewer, &csrf)?;
    if !app.limiter.allow(&format!("avatar:{}", viewer.user_id), 10, Duration::from_secs(3600)) {
        return Ok(settings_error(&app, Some(viewer), "You changed the picture very often. Try again later.").await?);
    }
    let Some(bytes) = picture else {
        return Ok(settings_error(&app, Some(viewer), "Choose a picture first.").await?);
    };
    if bytes.len() > AVATAR_MAX_BYTES {
        return Ok(settings_error(&app, Some(viewer), "The picture is larger than 1 MB.").await?);
    }
    let content_type = match upload::check(&bytes) {
        Ok(content_type) => content_type,
        Err(error) => return Ok(settings_error(&app, Some(viewer), error).await?),
    };
    let id = upload::new_id();
    if app.storage.put(&id, bytes).await.is_err() {
        return Ok(settings_error(&app, Some(viewer), "The picture could not be stored. Try again later.").await?);
    }
    let old = current_avatar(&app, viewer.user_id).await?;
    sqlx::query("UPDATE users SET avatar_id = $2, avatar_type = $3 WHERE id = $1")
        .bind(viewer.user_id)
        .bind(&id)
        .bind(content_type)
        .execute(&app.pool)
        .await?;
    if let Some(old) = old {
        let _ = app.storage.delete(&old).await;
    }
    Ok(Redirect::to("/settings").into_response())
}

async fn settings_error(app: &AppState, viewer: Option<Viewer>, error: &str) -> Result<Response, AppError> {
    let Some(viewer) = viewer else {
        return Ok(Redirect::to("/login").into_response());
    };
    let avatar = current_avatar(app, viewer.user_id).await?;
    Ok(settings_page(&viewer, avatar.as_deref(), Some(error), StatusCode::UNPROCESSABLE_ENTITY))
}

#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

pub async fn remove_avatar(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    axum::Form(form): axum::Form<CsrfForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    let old = current_avatar(&app, viewer.user_id).await?;
    sqlx::query("UPDATE users SET avatar_id = NULL, avatar_type = NULL WHERE id = $1")
        .bind(viewer.user_id)
        .execute(&app.pool)
        .await?;
    if let Some(old) = old {
        let _ = app.storage.delete(&old).await;
    }
    Ok(Redirect::to("/settings").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_follow_the_number_of_posts() {
        assert_eq!(rank(0), "Newcomer");
        assert_eq!(rank(9), "Newcomer");
        assert_eq!(rank(10), "Member");
        assert_eq!(rank(50), "Regular");
        assert_eq!(rank(200), "Veteran");
    }
}
