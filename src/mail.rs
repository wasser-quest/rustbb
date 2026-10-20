//! Outgoing mail. `sendmail` hands each message to the local mail server; `file` appends it to a
//! file, which the end-to-end test reads to find the confirmation link.
//!
//! Every message passes a quota, so the forum cannot be used to flood a mailbox: a few messages per
//! recipient, and a ceiling for all messages together. The quota holds even when the request limits
//! are left to a load balancer (FORUM_RATE_LIMIT=off).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use crate::ratelimit::Limiter;

const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(86_400);
/// Messages one address may receive.
const PER_RECIPIENT_HOUR: u32 = 3;
const PER_RECIPIENT_DAY: u32 = 6;
/// Messages the forum may send in all.
const ALL_HOUR: u32 = 100;
const ALL_DAY: u32 = 300;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Clone)]
pub struct Mailer {
    transport: Transport,
    quota: Arc<Limiter>,
}

#[derive(Clone)]
enum Transport {
    Sendmail { executable: PathBuf, from: String },
    File(PathBuf),
}

impl Mailer {
    /// `FORUM_MAIL_MODE` is `sendmail` (the default) or `file`.
    pub fn from_env() -> Result<Self, String> {
        Ok(Self { transport: Transport::from_env()?, quota: Arc::new(Limiter::default()) })
    }

    /// Sends one message, unless the recipient or the forum as a whole has used up its quota.
    pub async fn send(&self, to: &str, subject: &str, text: &str) -> Result<(), String> {
        if to.is_empty() || to.contains(['\r', '\n']) {
            return Err("the recipient is not a mailbox".to_owned());
        }
        let recipient = to.trim().to_lowercase();
        let allowed = self.quota.allow(&format!("to-hour:{recipient}"), PER_RECIPIENT_HOUR, HOUR)
            && self.quota.allow(&format!("to-day:{recipient}"), PER_RECIPIENT_DAY, DAY)
            && self.quota.allow("all-hour", ALL_HOUR, HOUR)
            && self.quota.allow("all-day", ALL_DAY, DAY);
        if !allowed {
            return Err("the mail quota is used up; try again later".to_owned());
        }
        self.transport.send(to, subject, text).await
    }
}

impl Transport {
    fn from_env() -> Result<Self, String> {
        match std::env::var("FORUM_MAIL_MODE").unwrap_or_else(|_| "sendmail".to_owned()).as_str() {
            "sendmail" => Ok(Self::Sendmail {
                executable: std::env::var("FORUM_SENDMAIL").unwrap_or_else(|_| "/usr/sbin/sendmail".to_owned()).into(),
                from: std::env::var("FORUM_MAIL_FROM").map_err(|_| "FORUM_MAIL_FROM is required in sendmail mode")?,
            }),
            "file" => Ok(Self::File(
                std::env::var("FORUM_MAIL_FILE").map_err(|_| "FORUM_MAIL_FILE is required in file mode")?.into(),
            )),
            other => Err(format!("FORUM_MAIL_MODE must be sendmail or file, not {other}")),
        }
    }

    async fn send(&self, to: &str, subject: &str, text: &str) -> Result<(), String> {
        match self {
            Self::File(path) => {
                let message = format!("To: {to}\nSubject: {subject}\n\n{text}\n----\n");
                let mut file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .await
                    .map_err(|error| error.to_string())?;
                file.write_all(message.as_bytes()).await.map_err(|error| error.to_string())
            }
            Self::Sendmail { executable, from } => {
                let message = format!(
                    "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\n\
                     MIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{text}\r\n"
                );
                let mut process = Command::new(executable)
                    .args(["-t", "-i"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| format!("{} could not start: {error}", executable.display()))?;
                let mut stdin = process.stdin.take().ok_or("sendmail has no stdin")?;
                stdin.write_all(message.as_bytes()).await.map_err(|error| error.to_string())?;
                drop(stdin);
                let output = tokio::time::timeout(Duration::from_secs(60), process.wait_with_output())
                    .await
                    .map_err(|_| "sendmail did not finish in time".to_owned())?
                    .map_err(|error| error.to_string())?;
                if output.status.success() {
                    Ok(())
                } else {
                    Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_file(path: PathBuf) -> Mailer {
        Mailer { transport: Transport::File(path), quota: Arc::new(Limiter::default()) }
    }

    #[tokio::test]
    async fn one_address_gets_three_messages_an_hour_and_others_still_get_theirs() {
        let path = std::env::temp_dir().join(format!("rustbb-mail-quota-{}", std::process::id()));
        let mailer = to_file(path.clone());
        for _ in 0..PER_RECIPIENT_HOUR {
            assert!(mailer.send("victim@example.org", "s", "t").await.is_ok());
        }
        assert!(mailer.send("Victim@Example.org", "s", "t").await.is_err(), "the address is compared without case");
        assert!(mailer.send("other@example.org", "s", "t").await.is_ok());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn the_forum_as_a_whole_has_a_ceiling() {
        let path = std::env::temp_dir().join(format!("rustbb-mail-ceiling-{}", std::process::id()));
        let mailer = to_file(path.clone());
        for n in 0..ALL_HOUR {
            assert!(mailer.send(&format!("user{n}@example.org"), "s", "t").await.is_ok());
        }
        assert!(mailer.send("one-more@example.org", "s", "t").await.is_err());
        let _ = std::fs::remove_file(path);
    }
}
