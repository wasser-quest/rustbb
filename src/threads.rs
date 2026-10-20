//! Reading: boards, threads, search and the report queue. Writing to a thread is in posts.rs.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use maud::Markup;
use serde::Deserialize;

use crate::auth::{MaybeViewer, Viewer};
use crate::error::AppError;
use crate::models::{AttachmentRow, Board, PostRow, ReportRow, SearchRow, ThreadHead, ThreadRow};
use crate::{notify, polls, reactions, views, views_forms, AppState};

const THREADS_PER_PAGE: u32 = 30;
pub const POSTS_PER_PAGE: u32 = 50;

#[derive(Deserialize)]
pub struct PageQuery {
    page: Option<u32>,
}

fn page_no(query: &PageQuery) -> u32 {
    query.page.unwrap_or(1).clamp(1, 1000)
}

pub async fn board_by_slug(app: &AppState, slug: &str) -> Result<Board, AppError> {
    sqlx::query_as::<_, Board>("SELECT id, slug, title, description FROM boards WHERE slug = $1")
        .bind(slug)
        .fetch_optional(&app.pool)
        .await?
        .ok_or(AppError::NotFound)
}

/// The viewer for a state-changing form: signed in, and the CSRF token matches the session.
pub fn checked(viewer: Option<Viewer>, csrf: &str) -> Result<Viewer, AppError> {
    match viewer {
        Some(viewer) if viewer.csrf == csrf => Ok(viewer),
        _ => Err(AppError::Forbidden),
    }
}

pub async fn index(State(app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Result<Markup, AppError> {
    let boards = sqlx::query_as::<_, Board>("SELECT id, slug, title, description FROM boards ORDER BY position, id")
        .fetch_all(&app.pool)
        .await?;
    let seo = views::Seo {
        description: Some("Boards and threads: boards about the topics of the forum, with replies, images and polls.".to_owned()),
        canonical: Some(format!("{}/", app.public_url)),
    };
    Ok(views::page_seo("Boards", &seo, viewer.as_ref(), views::boards(&boards)))
}

pub async fn board(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(slug): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Markup, AppError> {
    let board = board_by_slug(&app, &slug).await?;
    let page = page_no(&query);
    // One extra row tells whether there is a next page.
    let mut threads = sqlx::query_as::<_, ThreadRow>(
        "SELECT t.id, t.title, u.username AS author, t.last_post_at,
                (SELECT count(*) FROM posts p WHERE p.thread_id = t.id AND p.deleted_at IS NULL) AS replies
           FROM threads t JOIN users u ON u.id = t.user_id
          WHERE t.board_id = $1
          ORDER BY t.last_post_at DESC, t.id DESC
          LIMIT $2 OFFSET $3",
    )
    .bind(board.id)
    .bind(i64::from(THREADS_PER_PAGE + 1))
    .bind(i64::from((page - 1) * THREADS_PER_PAGE))
    .fetch_all(&app.pool)
    .await?;
    let has_next = threads.len() > THREADS_PER_PAGE as usize;
    threads.truncate(THREADS_PER_PAGE as usize);
    let seo = views::Seo {
        description: Some(if board.description.is_empty() {
            format!("Threads in {}.", board.title)
        } else {
            board.description.clone()
        }),
        canonical: Some(format!("{}/boards/{}", app.public_url, board.slug)),
    };
    Ok(views::page_seo(&board.title, &seo, viewer.as_ref(), views::board(&board, &threads, viewer.as_ref(), page, has_next)))
}

pub async fn new_thread_form(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let board = board_by_slug(&app, &slug).await?;
    let Some(viewer) = viewer else {
        return Ok(Redirect::to("/login").into_response());
    };
    let markup = views_forms::new_thread(&board, &viewer, None, "", "", "", "");
    Ok(views::page("New thread", Some(&viewer), markup).into_response())
}

pub async fn thread(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> Result<Markup, AppError> {
    render_thread(&app, viewer, id, &query).await
}

/// The same thread under its address with the title words; the id alone reaches the same page.
pub async fn thread_titled(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path((id, _title)): Path<(i64, String)>,
    Query(query): Query<PageQuery>,
) -> Result<Markup, AppError> {
    render_thread(&app, viewer, id, &query).await
}

async fn render_thread(app: &AppState, viewer: Option<Viewer>, id: i64, query: &PageQuery) -> Result<Markup, AppError> {
    let head = thread_head(app, id).await?;
    let page = page_no(query);
    let mut posts = sqlx::query_as::<_, PostRow>(
        "SELECT p.id, u.username AS author, u.id AS author_id, u.title AS author_title,
                u.avatar_id,
                (SELECT count(*) FROM posts pp WHERE pp.user_id = u.id AND pp.deleted_at IS NULL) AS author_posts,
                CASE WHEN p.deleted_at IS NULL THEN p.body ELSE '' END AS body,
                p.created_at, p.deleted_at IS NOT NULL AS deleted
           FROM posts p JOIN users u ON u.id = p.user_id
          WHERE p.thread_id = $1
          ORDER BY p.id
          LIMIT $2 OFFSET $3",
    )
    .bind(id)
    .bind(i64::from(POSTS_PER_PAGE + 1))
    .bind(i64::from((page - 1) * POSTS_PER_PAGE))
    .fetch_all(&app.pool)
    .await?;
    let has_next = posts.len() > POSTS_PER_PAGE as usize;
    posts.truncate(POSTS_PER_PAGE as usize);
    let attachments = attachments_for(&app, &posts).await?;
    let viewer_id = viewer.as_ref().map(|viewer| viewer.user_id);
    let poll = polls::load(&app.pool, id, viewer_id).await?;
    let following = match viewer_id {
        Some(user_id) => notify::is_following(&app.pool, user_id, id).await?,
        None => false,
    };
    let post_ids: Vec<i64> = posts.iter().map(|post| post.id).collect();
    let kinds = reactions::kinds(&app.pool).await?;
    let tallies = reactions::tallies(&app.pool, &post_ids, viewer_id).await?;
    let extras = views::ThreadExtras { poll: poll.as_ref(), following, kinds: &kinds, tallies: &tallies };
    let markup = views::thread(&head, &posts, &attachments, viewer.as_ref(), page, has_next, &extras);
    let seo = views::Seo {
        description: posts.first().filter(|post| !post.deleted).map(|post| views::summary(&post.body)),
        canonical: Some(format!("{}{}", app.public_url, views::thread_path(id, &head.title))),
    };
    Ok(views::page_seo(&head.title, &seo, viewer.as_ref(), markup))
}

pub async fn thread_head(app: &AppState, id: i64) -> Result<ThreadHead, AppError> {
    sqlx::query_as::<_, ThreadHead>(
        "SELECT t.id, t.title, b.slug AS board_slug, b.title AS board_title
           FROM threads t JOIN boards b ON b.id = t.board_id
          WHERE t.id = $1",
    )
    .bind(id)
    .fetch_optional(&app.pool)
    .await?
    .ok_or(AppError::NotFound)
}

pub async fn attachments_for(app: &AppState, posts: &[PostRow]) -> Result<Vec<AttachmentRow>, AppError> {
    let ids: Vec<i64> = posts.iter().filter(|p| !p.deleted).map(|p| p.id).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as::<_, AttachmentRow>(
        "SELECT id, post_id FROM attachments WHERE post_id = ANY($1) ORDER BY created_at, id",
    )
    .bind(&ids)
    .fetch_all(&app.pool)
    .await?)
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
}

pub async fn search(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Query(query): Query<SearchQuery>,
) -> Result<Markup, AppError> {
    let text: String = query.q.unwrap_or_default().trim().chars().take(100).collect();
    let rows = if text.is_empty() {
        Vec::new()
    } else {
        // Escape the LIKE wildcards so the search text is matched literally.
        let escaped = text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        sqlx::query_as::<_, SearchRow>(
            "SELECT t.id, t.title, b.slug AS board_slug, u.username AS author, t.last_post_at
               FROM threads t JOIN boards b ON b.id = t.board_id JOIN users u ON u.id = t.user_id
              WHERE t.title ILIKE '%' || $1 || '%' ESCAPE '\\'
              ORDER BY t.last_post_at DESC LIMIT 50",
        )
        .bind(escaped)
        .fetch_all(&app.pool)
        .await?
    };
    Ok(views::page("Search", viewer.as_ref(), views::search(&text, &rows)))
}

pub async fn reports_for_admin(app: &AppState) -> Result<Vec<ReportRow>, AppError> {
    Ok(sqlx::query_as::<_, ReportRow>(
        "SELECT r.post_id, u.username AS reporter, r.reason,
                left(p.body, 500) AS body, r.created_at, p.thread_id
           FROM reports r JOIN posts p ON p.id = r.post_id JOIN users u ON u.id = r.user_id
          WHERE p.deleted_at IS NULL
          ORDER BY r.created_at DESC LIMIT 100",
    )
    .fetch_all(&app.pool)
    .await?)
}
