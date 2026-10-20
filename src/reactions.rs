//! Reactions to posts. Admins add the kinds under Admin > Reactions, each with a symbol (an emoji
//! or a short text) and a name. A member gives a kind to a post, and gives it again to take it back.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use maud::Markup;
use serde::Deserialize;
use sqlx::PgPool;

use crate::admin::require_admin;
use crate::auth::MaybeViewer;
use crate::error::AppError;
use crate::threads::checked;
use crate::{views, views_community, AppState};

const SYMBOL_MAX: usize = 16;
const NAME_MAX: usize = 32;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Kind {
    pub id: i64,
    pub symbol: String,
    pub name: String,
}

/// How many members gave one kind to a post, and whether the viewer is one of them.
#[derive(Debug, Clone, Copy)]
pub struct Tally {
    pub kind_id: i64,
    pub count: i64,
    pub mine: bool,
}

pub async fn kinds(pool: &PgPool) -> Result<Vec<Kind>, sqlx::Error> {
    sqlx::query_as::<_, Kind>("SELECT id, symbol, name FROM reaction_kinds ORDER BY position, id")
        .fetch_all(pool)
        .await
}

/// The tallies of each post on a page, by post id.
pub async fn tallies(pool: &PgPool, post_ids: &[i64], viewer: Option<i64>) -> Result<HashMap<i64, Vec<Tally>>, sqlx::Error> {
    let mut by_post: HashMap<i64, Vec<Tally>> = HashMap::new();
    if post_ids.is_empty() {
        return Ok(by_post);
    }
    // Member ids start at 1, so 0 stands for a visitor who has no reactions of their own.
    let rows: Vec<(i64, i64, i64, bool)> = sqlx::query_as(
        "SELECT post_id, kind_id, count(*), bool_or(user_id = $2)
           FROM post_reactions WHERE post_id = ANY($1)
          GROUP BY post_id, kind_id ORDER BY post_id, kind_id",
    )
    .bind(post_ids)
    .bind(viewer.unwrap_or(0))
    .fetch_all(pool)
    .await?;
    for (post_id, kind_id, count, mine) in rows {
        by_post.entry(post_id).or_default().push(Tally { kind_id, count, mine });
    }
    Ok(by_post)
}

/// A symbol of 1 to 16 characters and a name of 1 to 32, both without line breaks.
pub fn validate(symbol: &str, name: &str) -> Result<(String, String), &'static str> {
    let symbol = symbol.trim();
    let name = name.trim();
    if symbol.is_empty() || symbol.chars().count() > SYMBOL_MAX || symbol.chars().any(char::is_control) {
        return Err("A symbol has 1 to 16 characters.");
    }
    if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
        return Err("A name has 1 to 32 characters.");
    }
    Ok((symbol.to_owned(), name.to_owned()))
}

#[derive(Deserialize)]
pub struct ReactForm {
    csrf: String,
    kind: i64,
}

/// Gives a kind to a post, or takes it back if the member gave it already.
pub async fn react(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(post_id): Path<i64>,
    axum::Form(form): axum::Form<ReactForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    if !app.limiter.allow(&format!("react:{}", viewer.user_id), 60, Duration::from_secs(600)) {
        return Err(AppError::Forbidden);
    }
    let post: Option<(i64,)> = sqlx::query_as("SELECT thread_id FROM posts WHERE id = $1 AND deleted_at IS NULL")
        .bind(post_id)
        .fetch_optional(&app.pool)
        .await?;
    let Some((thread_id,)) = post else {
        return Err(AppError::NotFound);
    };
    let kind_exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM reaction_kinds WHERE id = $1")
        .bind(form.kind)
        .fetch_optional(&app.pool)
        .await?;
    if kind_exists.is_none() {
        return Err(AppError::NotFound);
    }
    let removed = sqlx::query("DELETE FROM post_reactions WHERE post_id = $1 AND user_id = $2 AND kind_id = $3")
        .bind(post_id)
        .bind(viewer.user_id)
        .bind(form.kind)
        .execute(&app.pool)
        .await?
        .rows_affected();
    if removed == 0 {
        sqlx::query("INSERT INTO post_reactions (post_id, user_id, kind_id) VALUES ($1, $2, $3)")
            .bind(post_id)
            .bind(viewer.user_id)
            .bind(form.kind)
            .execute(&app.pool)
            .await?;
    }
    Ok(Redirect::to(&format!("/threads/{thread_id}#post-{post_id}")).into_response())
}

pub async fn admin_page(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Result<Markup, AppError> {
    let viewer = require_admin(&app, viewer).await?;
    let kinds = kinds(&app.pool).await?;
    Ok(views::page("Reactions", Some(&viewer), views_community::admin_reactions(&kinds, &viewer.csrf)))
}

#[derive(Deserialize)]
pub struct NewKindForm {
    csrf: String,
    symbol: String,
    name: String,
}

pub async fn add_kind(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    axum::Form(form): axum::Form<NewKindForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    require_admin(&app, Some(viewer)).await?;
    let (symbol, name) = match validate(&form.symbol, &form.name) {
        Ok(kind) => kind,
        Err(error) => return Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, error).into_response()),
    };
    sqlx::query(
        "INSERT INTO reaction_kinds (symbol, name, position)
         VALUES ($1, $2, (SELECT COALESCE(MAX(position), 0) + 1 FROM reaction_kinds))",
    )
    .bind(symbol)
    .bind(name)
    .execute(&app.pool)
    .await?;
    Ok(Redirect::to("/admin/reactions").into_response())
}

#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

/// Removes a kind; the reactions that gave it are removed with it.
pub async fn remove_kind(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(kind_id): Path<i64>,
    axum::Form(form): axum::Form<CsrfForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    require_admin(&app, Some(viewer)).await?;
    sqlx::query("DELETE FROM reaction_kinds WHERE id = $1").bind(kind_id).execute(&app.pool).await?;
    Ok(Redirect::to("/admin/reactions").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_has_a_short_symbol_and_a_name() {
        assert!(validate("👍", "Thumbs up").is_ok());
        assert_eq!(validate(" ok ", " Fine ").unwrap(), ("ok".to_owned(), "Fine".to_owned()));
        assert!(validate("", "x").is_err());
        assert!(validate(&"x".repeat(17), "x").is_err());
        assert!(validate("x", "").is_err());
        assert!(validate("x", &"n".repeat(33)).is_err());
        assert!(validate("a\nb", "name").is_err());
    }
}
