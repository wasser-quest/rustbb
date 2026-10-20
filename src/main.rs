//! rustbb: boards, threads, replies, image attachments and accounts.
//! Server-rendered HTML, no JavaScript, pure Rust.

mod accounts;
mod admin;
mod auth;
mod captcha;
mod confirm;
mod content;
mod error;
mod files;
mod forms;
mod help;
mod mail;
mod metadata;
mod messages;
mod models;
mod net;
mod notify;
mod polls;
mod posts;
mod profile;
mod quota;
mod ratelimit;
mod reactions;
mod reset;
mod seo;
mod storage;
mod threads;
mod upload;
mod validate;
mod views;
mod views_community;
mod views_forms;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{header, HeaderName, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Shared by every request. Cheap to clone: the pool and the other parts are reference counted.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub storage: storage::Storage,
    pub limiter: Arc<ratelimit::Limiter>,
    /// What one user and all users together may upload (see quota.rs).
    pub quota: quota::Limits,
    pub secure_cookies: bool,
    /// Set when a reverse proxy is in front and sends X-Forwarded-For.
    pub trust_proxy: bool,
    /// An Argon2 hash of a random password, checked when a username does not exist.
    pub dummy_hash: String,
    pub mailer: mail::Mailer,
    /// The address the forum is reached at, for the links in mail (FORUM_PUBLIC_URL).
    pub public_url: String,
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("FORUM_DATABASE_URL")
        .map_err(|_| "FORUM_DATABASE_URL is required (postgres://user:password@host/database)")?;
    let bind: SocketAddr = env_or("FORUM_BIND", "127.0.0.1:8080").parse()?;
    // Secure cookies need https. Set FORUM_SECURE_COOKIES=false only for local http testing.
    let secure_cookies = env_or("FORUM_SECURE_COOKIES", "true") != "false";
    let trust_proxy = env_or("FORUM_TRUST_PROXY", "false") == "true";

    let pool = PgPoolOptions::new().max_connections(8).connect(&database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    let dummy_hash = auth::hash_password(&auth::random_token())
        .map_err(|error| format!("Passwort-Hash: {error}"))?;
    let storage = storage::Storage::from_env()?;
    let mailer = mail::Mailer::from_env()?;
    let public_url = env_or("FORUM_PUBLIC_URL", "http://127.0.0.1:8080").trim_end_matches('/').to_owned();

    // Expired CAPTCHA questions and unconfirmed sign-ups are removed every ten minutes.
    let purge_pool = pool.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            let _ = captcha::purge(&purge_pool).await;
            let _ = confirm::purge(&purge_pool).await;
        }
    });

    let state = AppState {
        pool,
        storage,
        limiter: Arc::new(ratelimit::Limiter::from_env()),
        quota: quota::Limits::from_env(),
        secure_cookies,
        trust_proxy,
        dummy_hash,
        mailer,
        public_url,
    };
    let app = Router::new()
        .route("/", get(threads::index))
        .route("/boards/{slug}", get(threads::board))
        .route("/boards/{slug}/threads/new", get(threads::new_thread_form).post(posts::create_thread))
        .route("/threads/{id}", get(threads::thread))
        .route("/threads/{id}/{title}", get(threads::thread_titled))
        .route("/robots.txt", get(seo::robots))
        .route("/sitemap.xml", get(seo::sitemap))
        .route("/threads/{id}/reply", post(posts::reply))
        .route("/threads/{id}/follow", post(notify::toggle_follow))
        .route("/polls/{id}/vote", post(polls::vote))
        .route("/posts/{id}/delete", post(posts::delete_post))
        .route("/posts/{id}/report", post(posts::report_post))
        .route("/search", get(threads::search))
        .route("/users/{username}", get(profile::profile))
        .route("/avatars/{id}", get(profile::avatar))
        .route("/settings", get(profile::settings))
        .route("/settings/avatar", post(profile::set_avatar))
        .route("/settings/avatar/remove", post(profile::remove_avatar))
        .route("/settings/delete", get(accounts::delete_form).post(accounts::delete))
        .route("/notifications", get(notify::notifications))
        .route("/notifications/read", post(notify::read_all))
        .route("/messages", get(messages::inbox))
        .route("/messages/new/{username}", get(messages::compose_form).post(messages::send_new))
        .route("/messages/{id}", get(messages::conversation))
        .route("/messages/{id}/reply", post(messages::reply))
        .route("/register", get(accounts::register_form).post(accounts::register))
        .route("/login", get(accounts::login_form).post(accounts::login))
        .route("/logout", post(accounts::logout))
        .route("/verify", get(confirm::verify))
        .route("/verify/resend", post(confirm::resend))
        .route("/password/forgot", get(reset::forgot_form).post(reset::forgot))
        .route("/password/reset", get(reset::reset_form).post(reset::reset))
        .route("/help", get(help::help))
        .route("/files/{id}", get(files::serve))
        .route("/admin/reports", get(admin::reports))
        .route("/admin/posts/{id}/remove", post(admin::remove))
        .route("/admin/users/{username}/ban", post(admin::ban))
        .route("/admin/users/{username}/unban", post(admin::unban))
        .route("/admin/users/{username}/title", post(admin::set_title))
        .route("/admin/settings", get(admin::settings).post(admin::save_settings))
        .route("/admin/reactions", get(reactions::admin_page).post(reactions::add_kind))
        .route("/admin/reactions/{id}/delete", post(reactions::remove_kind))
        .route("/posts/{id}/react", post(reactions::react))
        .route("/style.css", get(stylesheet))
        .route("/healthz", get(|| async { "ok" }))
        // Four files of 5 MB plus the text fields.
        .layer(DefaultBodyLimit::max(upload::MAX_FILE_BYTES * upload::MAX_FILES_PER_POST + 64 * 1024))
        .layer(middleware::from_fn(security_headers))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("rustbb listening on {bind}");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn stylesheet() -> Response {
    use axum::response::IntoResponse;
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/style.css"),
    )
        .into_response()
}

/// Every response: a strict policy that forbids scripts, frames and external resources.
async fn security_headers(request: Request, next: Next) -> Response {
    let noindex = seo::noindex_path(request.uri().path());
    let mut response = next.run(request).await;
    // A route may set a stricter policy of its own (the files route does); only fill in the rest.
    let headers = response.headers_mut();
    if noindex {
        headers.insert(HeaderName::from_static("x-robots-tag"), HeaderValue::from_static("noindex, nofollow"));
    }
    let defaults = [
        (header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::CACHE_CONTROL, "no-store"),
    ];
    for (name, value) in defaults {
        if !headers.contains_key(&name) {
            headers.insert(name, HeaderValue::from_static(value));
        }
    }
    response
}
