//! Polls. A new thread may carry one poll: a question and two to ten options, one per line.
//! Each member votes once, and may change the vote. Results are shown to everyone.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use sqlx::{PgPool, Postgres, Transaction};

use crate::auth::MaybeViewer;
use crate::error::AppError;
use crate::threads::checked;
use crate::AppState;

const QUESTION_MAX: usize = 200;
const OPTION_MAX: usize = 100;
const OPTIONS_MIN: usize = 2;
const OPTIONS_MAX: usize = 10;

pub struct PollView {
    pub id: i64,
    pub question: String,
    pub options: Vec<PollOption>,
    pub total: i64,
    /// The option this viewer chose, if they voted.
    pub mine: Option<i64>,
}

pub struct PollOption {
    pub id: i64,
    pub text: String,
    pub votes: i64,
}

/// Checks a poll typed into the new-thread form. Both fields empty means no poll.
pub fn parse(question: &str, options: &str) -> Result<Option<(String, Vec<String>)>, &'static str> {
    let question = question.trim();
    let options: Vec<&str> = options.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    if question.is_empty() && options.is_empty() {
        return Ok(None);
    }
    if question.is_empty() {
        return Err("The poll needs a question.");
    }
    if question.chars().count() > QUESTION_MAX {
        return Err("The poll question is too long (at most 200 characters).");
    }
    if !(OPTIONS_MIN..=OPTIONS_MAX).contains(&options.len()) {
        return Err("A poll needs two to ten options, one per line.");
    }
    if options.iter().any(|option| option.chars().count() > OPTION_MAX) {
        return Err("A poll option is too long (at most 100 characters).");
    }
    Ok(Some((question.to_owned(), options.into_iter().map(str::to_owned).collect())))
}

pub async fn create(tx: &mut Transaction<'_, Postgres>, thread_id: i64, question: &str, options: &[String]) -> Result<(), sqlx::Error> {
    let (poll_id,): (i64,) = sqlx::query_as("INSERT INTO polls (thread_id, question) VALUES ($1, $2) RETURNING id")
        .bind(thread_id)
        .bind(question)
        .fetch_one(&mut **tx)
        .await?;
    for (position, text) in options.iter().enumerate() {
        sqlx::query("INSERT INTO poll_options (poll_id, text, position) VALUES ($1, $2, $3)")
            .bind(poll_id)
            .bind(text)
            .bind(position as i32)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub async fn load(pool: &PgPool, thread_id: i64, viewer_id: Option<i64>) -> Result<Option<PollView>, sqlx::Error> {
    let Some((id, question)): Option<(i64, String)> = sqlx::query_as("SELECT id, question FROM polls WHERE thread_id = $1")
        .bind(thread_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT o.id, o.text, count(v.user_id) AS votes
           FROM poll_options o LEFT JOIN poll_votes v ON v.option_id = o.id
          WHERE o.poll_id = $1
          GROUP BY o.id, o.text, o.position
          ORDER BY o.position, o.id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let mine: Option<(i64,)> = match viewer_id {
        Some(user_id) => sqlx::query_as("SELECT option_id FROM poll_votes WHERE poll_id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?,
        None => None,
    };
    let options: Vec<PollOption> = rows.into_iter().map(|(id, text, votes)| PollOption { id, text, votes }).collect();
    let total = options.iter().map(|option| option.votes).sum();
    Ok(Some(PollView { id, question, options, total, mine: mine.map(|(option,)| option) }))
}

#[derive(Deserialize)]
pub struct VoteForm {
    csrf: String,
    option: i64,
}

pub async fn vote(
    State(app): State<AppState>,
    MaybeViewer(viewer): MaybeViewer,
    Path(poll_id): Path<i64>,
    axum::Form(form): axum::Form<VoteForm>,
) -> Result<Response, AppError> {
    let viewer = checked(viewer, &form.csrf)?;
    // The option must belong to this poll; otherwise nothing is written.
    let thread: Option<(i64,)> = sqlx::query_as(
        "WITH chosen AS (
             INSERT INTO poll_votes (poll_id, user_id, option_id)
             SELECT $1, $2, o.id FROM poll_options o WHERE o.id = $3 AND o.poll_id = $1
             ON CONFLICT (poll_id, user_id) DO UPDATE SET option_id = EXCLUDED.option_id
             RETURNING poll_id
         )
         SELECT p.thread_id FROM polls p WHERE p.id = (SELECT poll_id FROM chosen LIMIT 1)",
    )
    .bind(poll_id)
    .bind(viewer.user_id)
    .bind(form.option)
    .fetch_optional(&app.pool)
    .await?;
    let Some((thread_id,)) = thread else {
        return Err(AppError::NotFound);
    };
    Ok(Redirect::to(&format!("/threads/{thread_id}#poll")).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_poll_when_both_fields_are_empty() {
        assert_eq!(parse("  ", "\n \n"), Ok(None));
    }

    #[test]
    fn a_poll_needs_a_question_and_two_to_ten_options() {
        assert!(parse("", "a\nb").is_err());
        assert!(parse("Q?", "only one").is_err());
        assert!(parse("Q?", &(0..11).map(|i| format!("o{i}\n")).collect::<String>()).is_err());
        let (question, options) = parse(" Best? ", "Rust\n\n Go \n").unwrap().unwrap();
        assert_eq!(question, "Best?");
        assert_eq!(options, vec!["Rust", "Go"]);
    }
}
