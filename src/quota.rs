//! Upload limits, so the storage bill has a ceiling: what one user may keep in attachments and
//! upload per day, and what all attachments together may take. Set with environment variables;
//! the defaults keep the cost small and are the place to change the policy.

use sqlx::PgPool;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Bytes one user may keep in attachments (`FORUM_UPLOAD_USER_MB`, default 200).
    pub user_bytes: i64,
    /// New files one user may upload in 24 hours (`FORUM_UPLOAD_USER_FILES_PER_DAY`, default 40).
    pub user_files_per_day: i64,
    /// Bytes of all attachments together, the ceiling of the S3 bill (`FORUM_UPLOAD_TOTAL_GB`, default 20).
    pub total_bytes: i64,
}

impl Limits {
    pub fn from_env() -> Self {
        Self {
            user_bytes: number("FORUM_UPLOAD_USER_MB", 200) * 1024 * 1024,
            user_files_per_day: number("FORUM_UPLOAD_USER_FILES_PER_DAY", 40),
            total_bytes: number("FORUM_UPLOAD_TOTAL_GB", 20) * 1024 * 1024 * 1024,
        }
    }
}

fn number(name: &str, default: i64) -> i64 {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

/// Checks whether `user_id` may upload files of these sizes now. Returns the reason when not.
/// A soft check: uploads in the same moment can pass together, so the ceiling is exact only to
/// the size of a few uploads.
pub async fn check(pool: &PgPool, limits: Limits, user_id: i64, sizes: &[usize]) -> Result<(), &'static str> {
    let new_bytes: i64 = sizes.iter().map(|size| *size as i64).sum();
    let new_files = sizes.len() as i64;
    let (user_bytes, user_files, total_bytes): (i64, i64, i64) = sqlx::query_as(
        "SELECT
           COALESCE((SELECT SUM(size_bytes) FROM attachments WHERE user_id = $1), 0)::BIGINT,
           (SELECT COUNT(*) FROM attachments
              WHERE user_id = $1 AND created_at > now() - interval '24 hours')::BIGINT,
           COALESCE((SELECT SUM(size_bytes) FROM attachments), 0)::BIGINT",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "The upload could not be checked. Try again later.")?;
    if user_bytes + new_bytes > limits.user_bytes {
        return Err("You have reached your upload limit. Delete some files to upload more.");
    }
    if user_files + new_files > limits.user_files_per_day {
        return Err("You have uploaded too many files today. Try again tomorrow.");
    }
    if total_bytes + new_bytes > limits.total_bytes {
        return Err("The file storage of the forum is full at the moment. Try again later.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_small_and_bounded() {
        let limits = Limits::from_env();
        assert_eq!(limits.user_bytes, 200 * 1024 * 1024);
        assert_eq!(limits.user_files_per_day, 40);
        assert_eq!(limits.total_bytes, 20 * 1024 * 1024 * 1024);
    }
}
