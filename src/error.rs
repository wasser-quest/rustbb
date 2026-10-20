use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use maud::html;

use crate::views;

#[derive(Debug)]
pub enum AppError {
    NotFound,
    Forbidden,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        AppError::Database(error)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound => (
                StatusCode::NOT_FOUND,
                views::page("Not found", None, html! { p { "This page does not exist." } }),
            )
                .into_response(),
            AppError::Forbidden => (
                StatusCode::FORBIDDEN,
                views::page("Forbidden", None, html! { p { "The form expired. Reload the page and try again." } }),
            )
                .into_response(),
            AppError::Database(error) => {
                // The details go to the log only; the visitor gets a plain message.
                eprintln!("database error: {error}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    views::page("Error", None, html! { p { "Something went wrong. Try again later." } }),
                )
                    .into_response()
            }
        }
    }
}
