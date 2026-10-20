//! Search engines: `robots.txt`, which keeps the private pages out of the crawl, and `sitemap.xml`
//! with the boards and the most recent threads. The pages that must not be indexed also send
//! `X-Robots-Tag: noindex` (see `noindex_path`).

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};

use crate::error::AppError;
use crate::views::thread_path;
use crate::AppState;

/// Pages for one member, for the moderation, or for a search: never listed in a search engine.
const PRIVATE: [&str; 9] = [
    "/messages", "/notifications", "/settings", "/admin", "/login", "/register", "/verify", "/password", "/search",
];
/// The threads listed in the sitemap: the most recent ones.
const SITEMAP_THREADS: i64 = 5000;

/// Whether a path is one of the private pages, which must not be indexed.
pub fn noindex_path(path: &str) -> bool {
    PRIVATE.iter().any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")) || path.starts_with(&format!("{prefix}?")))
}

pub async fn robots(State(app): State<AppState>) -> Response {
    let mut body = String::from("User-agent: *\n");
    for prefix in PRIVATE {
        body.push_str(&format!("Disallow: {prefix}\n"));
    }
    body.push_str(&format!("\nSitemap: {}/sitemap.xml\n", app.public_url));
    text(header::HeaderValue::from_static("text/plain; charset=utf-8"), body)
}

pub async fn sitemap(State(app): State<AppState>) -> Result<Response, AppError> {
    let boards: Vec<(String,)> = sqlx::query_as("SELECT slug FROM boards ORDER BY position, id")
        .fetch_all(&app.pool)
        .await?;
    let threads: Vec<(i64, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, title, last_post_at FROM threads ORDER BY last_post_at DESC, id DESC LIMIT $1",
    )
    .bind(SITEMAP_THREADS)
    .fetch_all(&app.pool)
    .await?;
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    let base = &app.public_url;
    xml.push_str(&format!("  <url><loc>{}/</loc></url>\n", escape(base)));
    for (slug,) in boards {
        xml.push_str(&format!("  <url><loc>{}/boards/{}</loc></url>\n", escape(base), escape(&slug)));
    }
    for (id, title, last_post_at) in threads {
        xml.push_str(&format!(
            "  <url><loc>{}{}</loc><lastmod>{}</lastmod></url>\n",
            escape(base),
            escape(&thread_path(id, &title)),
            last_post_at.format("%Y-%m-%d")
        ));
    }
    xml.push_str("</urlset>\n");
    Ok(text(header::HeaderValue::from_static("application/xml; charset=utf-8"), xml))
}

fn text(content_type: header::HeaderValue, body: String) -> Response {
    ([(header::CONTENT_TYPE, content_type)], (StatusCode::OK, body)).into_response()
}

fn escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_pages_are_not_indexed() {
        assert!(noindex_path("/messages/3"));
        assert!(noindex_path("/search?q=x"));
        assert!(noindex_path("/admin/reports"));
        assert!(!noindex_path("/boards/general"));
        assert!(!noindex_path("/threads/4/some-title"));
        assert!(!noindex_path("/messagesx"));
    }

    #[test]
    fn sitemap_values_are_escaped() {
        assert_eq!(escape("https://x/a&b<c>"), "https://x/a&amp;b&lt;c&gt;");
    }
}
