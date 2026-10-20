//! Reads the multipart form used for threads and replies (text fields plus image files).

use axum::extract::Multipart;
use bytes::Bytes;

use crate::upload::MAX_FILES_PER_POST;

#[derive(Default)]
pub struct PostForm {
    pub csrf: String,
    pub title: String,
    pub body: String,
    pub files: Vec<Bytes>,
    /// Optional poll of a new thread: a question and one option per line.
    pub poll_question: String,
    pub poll_options: String,
}

pub async fn read_post(mut multipart: Multipart) -> Result<PostForm, &'static str> {
    let mut form = PostForm::default();
    while let Some(field) = multipart.next_field().await.map_err(|_| "The form is invalid.")? {
        let name = field.name().unwrap_or_default().to_owned();
        if name == "files" {
            let bytes = field.bytes().await.map_err(|_| "The file could not be read.")?;
            if bytes.is_empty() {
                continue; // an empty file input is not an attachment
            }
            if form.files.len() >= MAX_FILES_PER_POST {
                return Err("At most 4 files per message.");
            }
            form.files.push(bytes);
            continue;
        }
        let text = field.text().await.map_err(|_| "The form is invalid.")?;
        match name.as_str() {
            "csrf" => form.csrf = text,
            "title" => form.title = text,
            "body" => form.body = text,
            "poll_question" => form.poll_question = text,
            "poll_options" => form.poll_options = text,
            _ => {}
        }
    }
    Ok(form)
}
