//! Serves attachments and avatars. The id is random; the response is the stored image and nothing
//! else, with headers that stop it from being treated as a page.

use axum::extract::{Path, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::error::AppError;
use crate::AppState;

pub async fn serve(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    // Only ids of this shape exist; anything else is not even looked up.
    if !is_id(&id) {
        return Err(AppError::NotFound);
    }
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT a.content_type FROM attachments a JOIN posts p ON p.id = a.post_id
          WHERE a.id = $1 AND p.deleted_at IS NULL",
    )
    .bind(&id)
    .fetch_optional(&app.pool)
    .await?;
    let Some((content_type,)) = row else {
        return Err(AppError::NotFound);
    };
    stored_image(&app, &id, &content_type).await
}

/// Ids are 32 hex characters, as `upload::new_id` makes them.
pub fn is_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The stored image with the given id, with the headers that keep it from running as a page.
pub async fn stored_image(app: &AppState, id: &str, content_type: &str) -> Result<Response, AppError> {
    let bytes = match app.storage.get(id).await {
        Ok(bytes) => bytes,
        Err(_) => return Err(AppError::NotFound),
    };
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; sandbox"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400"));
    headers.insert(HeaderName::from_static("cross-origin-resource-policy"), HeaderValue::from_static("same-origin"));
    Ok(response)
}
