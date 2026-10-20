//! Administration: the report queue, bans, titles and site settings. Only accounts with is_admin set
//! (in the database) see these pages.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use maud::Markup;
use serde::Deserialize;

use crate::auth::{self, MaybeViewer, Viewer};
use crate::error::AppError;
use crate::posts::remove_post;
use crate::threads::{checked, reports_for_admin};
use crate::{views, views_community, views_forms, AppState};

/// The longest ban in days. Longer bans are permanent.
const MAX_BAN_DAYS: i32 = 3650;

pub(crate) async fn require_admin(app: &AppState, viewer: Option<Viewer>) -> Result<Viewer, AppError> {
    let viewer = viewer.ok_or(AppError::Forbidden)?;
    let is_admin: Option<(bool,)> = sqlx::query_as("SELECT is_admin FROM users WHERE id = $1")
        .bind(viewer.user_id)
        .fetch_optional(&app.pool)
        .await?;
    match is_admin {
        Some((true,)) => Ok(viewer),
        _ => Err(AppError::Forbidden),
    }
}

pub async fn reports(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Result<Markup, AppError> {
    let viewer = require_admin(&app, viewer).await?;
    let rows = reports_for_admin(&app).await?;
    Ok(views::page("Reports", Some(&viewer), views_forms::reports(&rows, &viewer.csrf)))
}

#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

pub async fn remove(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(post_id): Path<i64>,
    axum::Form(form): axum::Form<CsrfForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    require_admin(&app, Some(viewer)).await?;
    remove_post(&app, post_id).await?;
    Ok(Redirect::to("/admin/reports").into_response())
}

#[derive(Deserialize)]
pub struct BanForm {
    csrf: String,
    /// A number of days, or `permanent`.
    duration: String,
    reason: String,
}

/// Bans a member: they are signed out at once and cannot log in until the ban ends.
pub async fn ban(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(username): Path<String>,
    axum::Form(form): axum::Form<BanForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    let viewer = require_admin(&app, Some(viewer)).await?;
    let target = find_member(&app, &username).await?;
    if target.0 == viewer.user_id || target.1 {
        return Err(AppError::Forbidden);
    }
    let days = match form.duration.as_str() {
        "permanent" => 0,
        other => match other.parse::<i32>() {
            Ok(days) if (1..=MAX_BAN_DAYS).contains(&days) => days,
            _ => return Ok(plain_error("Choose a number of days, or permanent.")),
        },
    };
    let reason: String = form.reason.trim().chars().take(300).collect();
    sqlx::query(
        "UPDATE users SET ban_reason = $2,
                banned_until = CASE WHEN $3 = 0 THEN '9999-12-31T00:00:00Z'::timestamptz
                                    ELSE now() + make_interval(days => $3) END
          WHERE id = $1",
    )
    .bind(target.0)
    .bind(reason)
    .bind(days)
    .execute(&app.pool)
    .await?;
    auth::delete_all_sessions(&app.pool, target.0).await?;
    Ok(Redirect::to(&format!("/users/{username}")).into_response())
}

pub async fn unban(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(username): Path<String>,
    axum::Form(form): axum::Form<CsrfForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    require_admin(&app, Some(viewer)).await?;
    let target = find_member(&app, &username).await?;
    sqlx::query("UPDATE users SET banned_until = NULL, ban_reason = '' WHERE id = $1")
        .bind(target.0)
        .execute(&app.pool)
        .await?;
    Ok(Redirect::to(&format!("/users/{username}")).into_response())
}

#[derive(Deserialize)]
pub struct TitleForm {
    csrf: String,
    title: String,
}

/// Sets the title shown under a member's name. An empty title removes it.
pub async fn set_title(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(username): Path<String>,
    axum::Form(form): axum::Form<TitleForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    require_admin(&app, Some(viewer)).await?;
    let target = find_member(&app, &username).await?;
    let title = form.title.trim();
    if title.chars().count() > 40 {
        return Ok(plain_error("A title has at most 40 characters."));
    }
    let title = (!title.is_empty()).then_some(title);
    sqlx::query("UPDATE users SET title = $2 WHERE id = $1")
        .bind(target.0)
        .bind(title)
        .execute(&app.pool)
        .await?;
    Ok(Redirect::to(&format!("/users/{username}")).into_response())
}

/// The id of a member and whether they are an admin.
async fn find_member(app: &AppState, username: &str) -> Result<(i64, bool), AppError> {
    sqlx::query_as::<_, (i64, bool)>("SELECT id, is_admin FROM users WHERE username = $1")
        .bind(username)
        .fetch_optional(&app.pool)
        .await?
        .ok_or(AppError::NotFound)
}

pub async fn settings(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Result<Markup, AppError> {
    let viewer = require_admin(&app, viewer).await?;
    let messages = messages_switched_on(&app).await?;
    Ok(views::page("Site settings", Some(&viewer), views_community::admin_settings(&viewer, messages)))
}

#[derive(Deserialize)]
pub struct SettingsForm {
    csrf: String,
    /// Present (as `on`) when the box is ticked.
    private_messages: Option<String>,
}

pub async fn save_settings(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    axum::Form(form): axum::Form<SettingsForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    require_admin(&app, Some(viewer)).await?;
    let value = if form.private_messages.as_deref() == Some("on") { "on" } else { "off" };
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES ('private_messages', $1)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
    )
    .bind(value)
    .execute(&app.pool)
    .await?;
    Ok(Redirect::to("/admin/settings").into_response())
}

/// Whether private messages are switched on. A missing setting counts as off.
pub async fn messages_switched_on(app: &AppState) -> Result<bool, AppError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = 'private_messages'")
        .fetch_optional(&app.pool)
        .await?;
    Ok(matches!(row, Some((value,)) if value == "on"))
}

fn plain_error(message: &str) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, views::page("Error", None, maud::html! { p.error role="alert" { (message) } })).into_response()
}
