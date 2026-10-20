//! A single-use arithmetic question for registration. It needs no JavaScript and no image.
//! Each question is stored with a hash of its answer and expires after ten minutes.

use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use crate::auth::random_token;

/// The question as shown to the visitor, with the id to send back and the answer to check.
pub struct Challenge {
    pub id: String,
    pub question: String,
}

fn answer_hash(id: &str, answer: i64) -> String {
    hex::encode(Sha256::digest(format!("{id}:{answer}").as_bytes()))
}

/// Creates a question and stores its answer. The numbers stay small so people can solve them.
pub async fn issue(pool: &sqlx::PgPool) -> Result<Challenge, sqlx::Error> {
    let bytes = random_token();
    let a = 1 + (u8::from_str_radix(&bytes[0..2], 16).unwrap_or(0) % 19) as i64;
    let b = 1 + (u8::from_str_radix(&bytes[2..4], 16).unwrap_or(0) % 19) as i64;
    let id = random_token();
    let answer = a + b;
    sqlx::query("INSERT INTO captchas (id, answer_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(&id)
        .bind(answer_hash(&id, answer))
        .bind(Utc::now() + Duration::minutes(10))
        .execute(pool)
        .await?;
    Ok(Challenge { id, question: format!("What is {a} + {b}?") })
}

/// Checks an answer. The question is deleted either way, so it cannot be tried again.
pub async fn verify(pool: &sqlx::PgPool, id: &str, answer: &str) -> Result<bool, sqlx::Error> {
    let stored: Option<(String, chrono::DateTime<Utc>)> =
        sqlx::query_as("DELETE FROM captchas WHERE id = $1 RETURNING answer_hash, expires_at")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let Some((hash, expires)) = stored else {
        return Ok(false);
    };
    let Ok(number) = answer.trim().parse::<i64>() else {
        return Ok(false);
    };
    Ok(expires > Utc::now() && hash == answer_hash(id, number))
}

/// Removes expired questions; run from time to time so the table stays small.
pub async fn purge(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM captchas WHERE expires_at < now()").execute(pool).await?.rows_affected())
}
