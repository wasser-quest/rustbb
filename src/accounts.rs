//! Registration (with the anti-bot question and the e-mail confirmation), and log in and out.

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use maud::Markup;
use serde::Deserialize;

use crate::auth::{self, MaybeViewer};
use crate::error::AppError;
use crate::{captcha, confirm, net, validate, views, views_forms, AppState};

#[derive(Deserialize)]
pub struct RegisterForm {
    username: String,
    email: String,
    password: String,
    captcha_id: String,
    captcha_answer: String,
}

pub async fn register_form(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Result<Markup, AppError> {
    let challenge = captcha::issue(&app.pool).await?;
    Ok(views::page("Register", viewer.as_ref(), views_forms::register(None, "", "", &challenge)))
}

pub async fn register(
    State(app): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    MaybeViewer(_viewer): MaybeViewer,
    axum::Form(form): axum::Form<RegisterForm>,
) -> Result<Response, AppError> {
    let ip = net::client_ip(&headers, peer, app.trust_proxy);
    if !app.limiter.allow(&format!("register:{ip}"), 5, Duration::from_secs(3600)) {
        return error_page(&app, "Too many sign-ups from this address. Try again later.", &form.username, &form.email).await;
    }
    // The question is used up by this attempt, whatever the outcome.
    let human = captcha::verify(&app.pool, &form.captcha_id, &form.captcha_answer).await?;
    if !human {
        let error = "The anti-bot answer is wrong or expired. Answer the new question.";
        return error_page(&app, error, &form.username, &form.email).await;
    }
    let username = match validate::username(&form.username) {
        Ok(name) => name,
        Err(error) => return error_page(&app, error, &form.username, &form.email).await,
    };
    let email = match validate::email(&form.email) {
        Ok(address) => address,
        Err(error) => return error_page(&app, error, username, &form.email).await,
    };
    if let Err(error) = validate::password(&form.password) {
        return error_page(&app, error, username, email).await;
    }
    let hash = auth::hash_password(&form.password).map_err(|_| AppError::Forbidden)?;
    // The first account ever made is the admin. The lock makes sign-ups take turns, so two
    // sign-ups at the same moment cannot both count as the first.
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7001)").execute(&mut *tx).await?;
    let inserted: Result<(i64,), sqlx::Error> = sqlx::query_as(
        "INSERT INTO users (username, password_hash, email, is_admin)
         VALUES ($1, $2, $3, NOT EXISTS (SELECT 1 FROM users)) RETURNING id",
    )
    .bind(username)
    .bind(hash)
    .bind(email)
    .fetch_one(&mut *tx)
    .await;
    let user_id = match inserted {
        Ok((id,)) => {
            tx.commit().await?;
            id
        }
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return error_page(&app, "That username is taken.", username, email).await;
        }
        Err(error) => return Err(error.into()),
    };
    // The account can log in only after the link in the mail has been opened.
    if !confirm::send_link(&app, user_id, username, email).await? {
        sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&app.pool).await?;
        let error = "We could not send the confirmation e-mail. Try again later.";
        return error_page(&app, error, username, email).await;
    }
    Ok(views::page("Check your e-mail", None, views_forms::check_mail(email)).into_response())
}

async fn error_page(app: &AppState, error: &str, username: &str, email: &str) -> Result<Response, AppError> {
    let challenge = captcha::issue(&app.pool).await?;
    let markup = views_forms::register(Some(error), username, email, &challenge);
    Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, views::page("Register", None, markup)).into_response())
}

const BANNED: &str = "This account is banned. Ask an admin if you think this is a mistake.";
const UNCONFIRMED: &str ="Confirm your e-mail address first: open the link in the mail, or request a new one at /verify.";

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

pub async fn login_form(MaybeViewer(viewer): MaybeViewer) -> Markup {
    views::page("Log in", viewer.as_ref(), views_forms::login(None, ""))
}

pub async fn login(
    State(app): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    MaybeViewer(_viewer): MaybeViewer,
    axum::Form(form): axum::Form<LoginForm>,
) -> Result<Response, AppError> {
    let ip = net::client_ip(&headers, peer, app.trust_proxy);
    let wrong = || {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            views::page("Log in", None, views_forms::login(Some("Wrong username or password."), &form.username)),
        )
            .into_response()
    };
    if !app.limiter.allow(&format!("login:{ip}"), 20, Duration::from_secs(900)) {
        return Ok(wrong());
    }
    let found: Option<(i64, String, bool, bool)> = sqlx::query_as(
        "SELECT id, password_hash, email_confirmed_at IS NOT NULL,
                COALESCE(banned_until > now(), false)
           FROM users WHERE username = $1",
    )
    .bind(&form.username)
    .fetch_optional(&app.pool)
    .await?;
    // Unknown users are checked against a dummy hash, so the response time does not reveal them.
    let (user_id, stored, confirmed, banned) = match found {
        Some((id, hash, confirmed, banned)) => (Some(id), hash, confirmed, banned),
        None => (None, app.dummy_hash.clone(), false, false),
    };
    let ok = auth::verify_password(&form.password, &stored) && user_id.is_some();
    // The password is checked first, so only the owner learns about an unconfirmed address or a ban.
    let refusal = if !ok {
        None
    } else if banned {
        Some(BANNED)
    } else if !confirmed {
        Some(UNCONFIRMED)
    } else {
        None
    };
    match (user_id, refusal) {
        (Some(_), Some(message)) => Ok((
            axum::http::StatusCode::FORBIDDEN,
            views::page("Log in", None, views_forms::login(Some(message), &form.username)),
        )
            .into_response()),
        (Some(id), None) if ok => start_session(&app, id).await,
        _ => Ok(wrong()),
    }
}

#[derive(Deserialize)]
pub struct LogoutForm {
    csrf: String,
}

pub async fn logout(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    parts: axum::http::request::Parts,
    axum::Form(form): axum::Form<LogoutForm>,
) -> Result<Response, AppError> {
    crate::threads::checked(viewer, &form.csrf)?;
    if let Some(token) = auth::session_token(&parts) {
        auth::delete_session(&app.pool, &token).await?;
    }
    let mut response = Redirect::to("/").into_response();
    set_cookie(&mut response, &auth::cleared_cookie(app.secure_cookies));
    Ok(response)
}

async fn start_session(app: &AppState, user_id: i64) -> Result<Response, AppError> {
    let token = auth::create_session(&app.pool, user_id).await?;
    let mut response = Redirect::to("/").into_response();
    set_cookie(&mut response, &auth::session_cookie(&token, app.secure_cookies));
    Ok(response)
}

fn set_cookie(response: &mut Response, cookie: &str) {
    if let Ok(value) = axum::http::HeaderValue::from_str(cookie) {
        response.headers_mut().append(axum::http::header::SET_COOKIE, value);
    }
}
