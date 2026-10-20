//! Accounts and sessions. Passwords are hashed with Argon2id; sessions use random tokens of
//! which only the SHA-256 is stored. Every form that changes state also checks a CSRF token.

use std::convert::Infallible;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use crate::AppState;

pub const SESSION_COOKIE: &str = "forum_session";
const SESSION_DAYS: i64 = 30;

/// The signed-in user of a request, with the CSRF token of their session. A banned account has no
/// viewer at all, so every page treats it as a visitor.
#[derive(Clone, Debug)]
pub struct Viewer {
    pub user_id: i64,
    pub username: String,
    pub csrf: String,
    pub is_admin: bool,
    /// Notifications nobody has read yet.
    pub unread: i64,
    /// Whether private messages are switched on for the site.
    pub messages: bool,
}

/// The viewer if the request carries a valid session. Never rejects: anonymous visitors are normal.
pub struct MaybeViewer(pub Option<Viewer>);

impl FromRequestParts<AppState> for MaybeViewer {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Infallible> {
        let Some(token) = cookie(parts, SESSION_COOKIE) else {
            return Ok(MaybeViewer(None));
        };
        let row = sqlx::query_as::<_, (i64, String, String, bool, i64, bool)>(
            "SELECT u.id, u.username, s.csrf, u.is_admin,
                    (SELECT count(*) FROM notifications n WHERE n.user_id = u.id AND n.read_at IS NULL),
                    EXISTS (SELECT 1 FROM settings st WHERE st.key = 'private_messages' AND st.value = 'on')
               FROM sessions s JOIN users u ON u.id = s.user_id
              WHERE s.token_hash = $1 AND s.expires_at > now()
                AND (u.banned_until IS NULL OR u.banned_until <= now())",
        )
        .bind(token_hash(&token))
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
        Ok(MaybeViewer(row.map(|(user_id, username, csrf, is_admin, unread, messages)| Viewer {
            user_id,
            username,
            csrf,
            is_admin,
            unread,
            messages,
        })))
    }
}

/// The session token from the request, if there is one.
pub fn session_token(parts: &Parts) -> Option<String> {
    cookie(parts, SESSION_COOKIE)
}

fn cookie(parts: &Parts, name: &str) -> Option<String> {
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
}

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Creates a session for a user and returns its token (only the hash is stored).
pub async fn create_session(pool: &sqlx::PgPool, user_id: i64) -> Result<String, sqlx::Error> {
    let token = random_token();
    let csrf = random_token();
    let expires = Utc::now() + Duration::days(SESSION_DAYS);
    sqlx::query("INSERT INTO sessions (token_hash, user_id, csrf, expires_at) VALUES ($1, $2, $3, $4)")
        .bind(token_hash(&token))
        .bind(user_id)
        .bind(csrf)
        .bind(expires)
        .execute(pool)
        .await?;
    Ok(token)
}

pub async fn delete_session(pool: &sqlx::PgPool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash(token))
        .execute(pool)
        .await?;
    Ok(())
}

/// Signs a user out everywhere, for example after a password reset.
pub async fn delete_all_sessions(pool: &sqlx::PgPool, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(user_id).execute(pool).await?;
    Ok(())
}

/// The cookie that keeps a session: HttpOnly, SameSite=Lax, and Secure unless disabled for local http.
pub fn session_cookie(token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        SESSION_DAYS * 24 * 60 * 60
    )
}

pub fn cleared_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
}
