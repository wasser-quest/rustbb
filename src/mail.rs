//! Outgoing mail. `sendmail` hands each message to the local mail server; `file` appends it to a
//! file, which the end-to-end test reads to find the confirmation link.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Clone)]
pub enum Mailer {
    Sendmail { executable: PathBuf, from: String },
    File(PathBuf),
}

impl Mailer {
    /// `FORUM_MAIL_MODE` is `sendmail` (the default) or `file`.
    pub fn from_env() -> Result<Self, String> {
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

    pub async fn send(&self, to: &str, subject: &str, text: &str) -> Result<(), String> {
        if to.is_empty() || to.contains(['\r', '\n']) {
            return Err("the recipient is not a mailbox".to_owned());
        }
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
