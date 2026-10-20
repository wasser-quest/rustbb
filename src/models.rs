use chrono::{DateTime, Utc};

#[derive(Debug, sqlx::FromRow)]
pub struct Board {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ThreadRow {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub last_post_at: DateTime<Utc>,
    pub replies: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ThreadHead {
    pub id: i64,
    pub title: String,
    pub board_slug: String,
    pub board_title: String,
}

/// A post as shown: `body` is empty and `deleted` is true when its author removed it.
/// The author's title, picture and post count go with it, for the rank and the picture beside it.
#[derive(Debug, sqlx::FromRow)]
pub struct PostRow {
    pub id: i64,
    pub author: String,
    pub author_id: i64,
    pub author_title: Option<String>,
    pub avatar_id: Option<String>,
    pub author_posts: i64,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub deleted: bool,
}

#[derive(Debug, sqlx::FromRow)]
pub struct AttachmentRow {
    pub id: String,
    pub post_id: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ReportRow {
    pub post_id: i64,
    pub thread_id: i64,
    pub reporter: String,
    pub reason: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ProfileRow {
    pub id: i64,
    pub username: String,
    pub created_at: DateTime<Utc>,
    pub posts: i64,
    pub title: Option<String>,
    pub avatar_id: Option<String>,
    /// Set while the account is banned; 31 December 9999 for a permanent ban.
    pub banned_until: Option<DateTime<Utc>>,
    pub ban_reason: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SearchRow {
    pub id: i64,
    pub title: String,
    pub board_slug: String,
    pub author: String,
    pub last_post_at: DateTime<Utc>,
}
