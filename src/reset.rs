//! Forgotten passwords. A link by mail leads to a form for a new password. The link works once and
//! for an hour, and a new password signs the account out everywhere. The forms give the same answer
//! for every address, so they do not reveal which accounts exist.

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use maud::Markup;
use serde::Deserialize;

use crate::auth::{self, MaybeViewer};
use crate::error::AppError;
use crate::{net, validate, views, views_forms, AppState};

const LINK_MINUTES: i64 = 60;
const ANSWER: &str = "If an account uses that address and has a confirmed e-mail address, a link to set a new password is on its way.";

pub async fn forgot_form(MaybeViewer(viewer): MaybeViewer) -> Markup {
    views::page("Forgot your password", viewer.as_ref(), views_forms::forgot(None))
}

#[derive(Deserialize)]
pub struct ForgotForm {
    email: String,
}

pub async fn forgot(
    State(app): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    MaybeViewer(viewer): MaybeViewer,
    axum::Form(form): axum::Form<ForgotForm>,
) -> Result<Markup, AppError> {
    let ip = net::client_ip(&headers, peer, app.trust_proxy);
    if !app.limiter.allow(&format!("reset:{ip}"), 5, Duration::from_secs(3600)) {
        return Ok(views::page("Forgot your password", viewer.as_ref(), views_forms::forgot(Some("Too many requests. Try again later."))));
    }
    let email = form.email.trim();
    let account: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, username FROM users
          WHERE lower(email) = lower($1) AND email_confirmed_at IS NOT NULL
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(email)
    .fetch_optional(&app.pool)
    .await?;
    if let Some((user_id, username)) = account {
        let token = auth::random_token();
        let expires: DateTime<Utc> = Utc::now() + ChronoDuration::minutes(LINK_MINUTES);
        let mut tx = app.pool.begin().await?;
        sqlx::query("DELETE FROM password_resets WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO password_resets (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
            .bind(auth::token_hash(&token))
            .bind(user_id)
            .bind(expires)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let link = format!("{}/password/reset?token={token}", app.public_url);
        let text = format!(
            "Hello {username},\n\nopen this link to set a new password for rustbb. It works once, for {LINK_MINUTES} minutes:\n\n{link}\n\n\
             If you did not ask for this, ignore this message. Your password stays as it is."
        );
        if let Err(error) = app.mailer.send(email, "Set a new rustbb password", &text).await {
            eprintln!("password mail failed: {error}");
        }
    }
    Ok(views::page("Forgot your password", viewer.as_ref(), views_forms::forgot(Some(ANSWER))))
}

#[derive(Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

pub async fn reset_form(MaybeViewer(viewer): MaybeViewer, Query(query): Query<TokenQuery>) -> Markup {
    let markup = match query.token {
        Some(token) if !token.is_empty() => views_forms::reset(&token, None),
        _ => views_forms::forgot(None),
    };
    views::page("Set a new password", viewer.as_ref(), markup)
}

#[derive(Deserialize)]
pub struct ResetForm {
    token: String,
    password: String,
}

pub async fn reset(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer, axum::Form(form): axum::Form<ResetForm>) -> Result<Response, AppError> {
    if let Err(error) = validate::password(&form.password) {
        let markup = views_forms::reset(&form.token, Some(error));
        return Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, views::page("Set a new password", viewer.as_ref(), markup)).into_response());
    }
    let row: Option<(i64, DateTime<Utc>)> =
        sqlx::query_as("DELETE FROM password_resets WHERE token_hash = $1 RETURNING user_id, expires_at")
            .bind(auth::token_hash(&form.token))
            .fetch_optional(&app.pool)
            .await?;
    let user_id = match row {
        Some((user_id, expires)) if expires > Utc::now() => user_id,
        _ => {
            let markup = views_forms::reset_unusable();
            return Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, views::page("Set a new password", viewer.as_ref(), markup)).into_response());
        }
    };
    let hash = auth::hash_password(&form.password).map_err(|_| AppError::Forbidden)?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(user_id)
        .bind(hash)
        .execute(&app.pool)
        .await?;
    sqlx::query("DELETE FROM password_resets WHERE user_id = $1").bind(user_id).execute(&app.pool).await?;
    auth::delete_all_sessions(&app.pool, user_id).await?;
    Ok(views::page("Password changed", None, views_forms::reset_done()).into_response())
}
