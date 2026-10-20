//! E-mail confirmation. A sign-up is only usable once its link has been opened. Only a hash of
//! the link's token is stored, and a new link replaces the older ones of the same account.

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use maud::Markup;
use serde::Deserialize;
use sqlx::PgPool;

use crate::auth::{self, MaybeViewer};
use crate::error::AppError;
use crate::{net, views, views_forms, AppState};

/// How long a confirmation link works.
const LINK_HOURS: i64 = 24;
/// Accounts whose address is never confirmed are removed after this many days.
const UNCONFIRMED_DAYS: i32 = 3;

pub enum Outcome {
    Confirmed,
    Expired,
    /// Another account confirmed the same address in the meantime.
    Taken,
}

/// Mails a new confirmation link to an account. Returns false when the mail could not be sent.
pub async fn send_link(app: &AppState, user_id: i64, username: &str, email: &str) -> Result<bool, AppError> {
    let token = auth::random_token();
    let expires: DateTime<Utc> = Utc::now() + ChronoDuration::hours(LINK_HOURS);
    let mut tx = app.pool.begin().await?;
    sqlx::query("DELETE FROM email_confirmations WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO email_confirmations (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(auth::token_hash(&token))
        .bind(user_id)
        .bind(expires)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let link = format!("{}/verify?token={token}", app.public_url);
    let text = format!(
        "Hello {username},\n\nopen this link to confirm your e-mail address for rustbb. \
         It works for {LINK_HOURS} hours:\n\n{link}\n\nIf you did not sign up, ignore this message."
    );
    match app.mailer.send(email, "Confirm your rustbb account", &text).await {
        Ok(()) => Ok(true),
        Err(error) => {
            eprintln!("confirmation mail to an account failed: {error}");
            Ok(false)
        }
    }
}

/// Confirms the address of the account that owns the token. A token works once.
pub async fn confirm(pool: &PgPool, token: &str) -> Result<Outcome, sqlx::Error> {
    let found: Option<(i64, DateTime<Utc>)> =
        sqlx::query_as("DELETE FROM email_confirmations WHERE token_hash = $1 RETURNING user_id, expires_at")
            .bind(auth::token_hash(token))
            .fetch_optional(pool)
            .await?;
    let Some((user_id, expires)) = found else {
        return Ok(Outcome::Expired);
    };
    if expires < Utc::now() {
        return Ok(Outcome::Expired);
    }
    let updated = sqlx::query("UPDATE users SET email_confirmed_at = now() WHERE id = $1 AND email_confirmed_at IS NULL")
        .bind(user_id)
        .execute(pool)
        .await;
    match updated {
        Ok(_) => Ok(Outcome::Confirmed),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => Ok(Outcome::Taken),
        Err(error) => Err(error),
    }
}

/// Removes the sign-ups whose address was never confirmed.
pub async fn purge(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM users WHERE email_confirmed_at IS NULL AND created_at < now() - make_interval(days => $1)",
    )
    .bind(UNCONFIRMED_DAYS)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

#[derive(Deserialize)]
pub struct VerifyQuery {
    token: Option<String>,
}

pub async fn verify(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Query(query): Query<VerifyQuery>,
) -> Result<Markup, AppError> {
    let message = match query.token.as_deref() {
        None => None,
        Some(token) => Some(match confirm(&app.pool, token).await? {
            Outcome::Confirmed => "Your address is confirmed. You can log in now.",
            Outcome::Expired => "This link has expired or was already used. Request a new one below.",
            Outcome::Taken => "Another account already uses this address.",
        }),
    };
    Ok(views::page("Confirm your e-mail", viewer.as_ref(), views_forms::verify(message)))
}

#[derive(Deserialize)]
pub struct ResendForm {
    email: String,
}

pub async fn resend(
    State(app): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    MaybeViewer(viewer): MaybeViewer,
    axum::Form(form): axum::Form<ResendForm>,
) -> Result<Markup, AppError> {
    let ip = net::client_ip(&headers, peer, app.trust_proxy);
    let message = if app.limiter.allow(&format!("resend:{ip}"), 5, Duration::from_secs(3600)) {
        // The same answer for every address, so the form does not reveal which ones have accounts.
        let pending: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, username FROM users WHERE lower(email) = lower($1) AND email_confirmed_at IS NULL \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(form.email.trim())
        .fetch_optional(&app.pool)
        .await?;
        if let Some((user_id, username)) = pending {
            send_link(&app, user_id, &username, form.email.trim()).await?;
        }
        "If that address is waiting for confirmation, a new link is on its way."
    } else {
        "Too many requests. Try again later."
    };
    Ok(views::page("Confirm your e-mail", viewer.as_ref(), views_forms::verify(Some(message))))
}
