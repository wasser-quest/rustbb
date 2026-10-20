//! Forms: new threads, replies, accounts and the admin report queue.

use maud::{html, Markup};

use crate::auth::Viewer;
use crate::captcha::Challenge;
use crate::models::{Board, ReportRow, ThreadHead};
use crate::views::when;

pub fn new_thread(
    board: &Board,
    viewer: &Viewer,
    error: Option<&str>,
    title: &str,
    body: &str,
    poll_question: &str,
    poll_options: &str,
) -> Markup {
    html! {
        p.crumbs { a href="/" { "Boards" } " / " a href=(format!("/boards/{}", board.slug)) { (board.title) } " / New thread" }
        h1 { "New thread" }
        @if let Some(error) = error { p.error role="alert" { (error) } }
        form method="post" action=(format!("/boards/{}/threads/new", board.slug)) enctype="multipart/form-data" {
            input type="hidden" name="csrf" value=(viewer.csrf);
            label { "Title" input type="text" name="title" value=(title) maxlength="120" required; }
            label { "Message" textarea name="body" rows="10" maxlength="10000" required { (body) } }
            (formatting_help())
            (attachment_field())
            details.optional {
                summary { "Add a poll (optional)" }
                label { "Question" input type="text" name="poll_question" value=(poll_question) maxlength="200"; }
                label { "Options, one per line (two to ten)"
                    textarea name="poll_options" rows="4" maxlength="1200" { (poll_options) }
                }
            }
            button type="submit" { "Post thread" }
        }
    }
}

pub fn reply_box(head: &ThreadHead, viewer: &Viewer, error: Option<&str>) -> Markup {
    html! {
        h2 { "Reply" }
        @if let Some(error) = error { p.error role="alert" { (error) } }
        form method="post" action=(format!("/threads/{}/reply", head.id)) enctype="multipart/form-data" {
            input type="hidden" name="csrf" value=(viewer.csrf);
            label { "Message" textarea name="body" rows="6" maxlength="10000" required {} }
            (formatting_help())
            (attachment_field())
            button type="submit" { "Reply" }
        }
    }
}

/// The tags a post may use, with an example each. The full list is on the help page.
fn formatting_help() -> Markup {
    html! {
        details.help {
            summary { "Formatting: quotes, embeds and more" }
            p.meta {
                "[b]bold[/b] · [i]italic[/i] · [spoiler]hidden[/spoiler] · [code]raw[/code]" br;
                "[quote=name]a quote with its author[/quote]" br;
                "[embed title=\"Title\" url=\"https://example.org\" color=\"blue\"]a card[/embed]" br;
                "Colors: rust, blue, green, gray. " a href="/help" { "Full reference" }
            }
        }
    }
}

fn attachment_field() -> Markup {
    html! {
        label {
            "Images (PNG, JPEG, GIF or WebP, up to 4 files of 5 MB each)"
            input type="file" name="files" accept="image/png,image/jpeg,image/gif,image/webp" multiple;
        }
    }
}

fn captcha_field(challenge: &Challenge, answer_error: bool) -> Markup {
    html! {
        label {
            "Anti-bot check: " (challenge.question)
            input type="hidden" name="captcha_id" value=(challenge.id);
            input type="text" name="captcha_answer" inputmode="numeric" autocomplete="off" required
                aria-invalid=(if answer_error { "true" } else { "false" });
        }
    }
}

pub fn register(error: Option<&str>, username: &str, email: &str, challenge: &Challenge) -> Markup {
    html! {
        h1 { "Register" }
        @if let Some(error) = error { p.error role="alert" { (error) } }
        form method="post" action="/register" {
            label { "Username" input type="text" name="username" value=(username) autocomplete="username" maxlength="32" required; }
            label { "E-mail address (we send the confirmation link here)" input type="email" name="email" value=(email) autocomplete="email" maxlength="254" required; }
            label { "Password (at least 10 characters)" input type="password" name="password" autocomplete="new-password" minlength="10" maxlength="256" required; }
            (captcha_field(challenge, error.is_some()))
            button type="submit" { "Create account" }
        }
    }
}

/// Shown instead of the sign-up form while an admin has closed registration.
pub fn registration_closed() -> Markup {
    html! {
        h1 { "Registration is closed" }
        p { "New accounts are not being made at the moment." }
        p.meta { a href="/login" { "Log in" } " if you already have an account." }
    }
}

/// Shown after registering: the account works once the link in the mail has been opened.
pub fn check_mail(email: &str) -> Markup {
    html! {
        h1 { "Check your e-mail" }
        p { "We sent a link to " strong { (email) } ". Open it to activate your account. The link works for 24 hours." }
        p.meta { "No mail after a few minutes? Check the spam folder, or request a new link on the " a href="/verify" { "confirmation page" } "." }
    }
}

/// The confirmation page: the result of an opened link, and a form for a new link.
pub fn verify(message: Option<&str>) -> Markup {
    html! {
        h1 { "Confirm your e-mail" }
        @if let Some(message) = message { p role="status" { (message) } }
        h2 { "Request a new link" }
        form method="post" action="/verify/resend" {
            label { "E-mail address" input type="email" name="email" autocomplete="email" maxlength="254" required; }
            button type="submit" { "Send a new link" }
        }
        p.meta { "Confirmed already? " a href="/login" { "Log in" } }
    }
}

pub fn login(error: Option<&str>, username: &str) -> Markup {
    html! {
        h1 { "Log in" }
        @if let Some(error) = error { p.error role="alert" { (error) } }
        form method="post" action="/login" {
            label { "Username" input type="text" name="username" value=(username) autocomplete="username" required; }
            label { "Password" input type="password" name="password" autocomplete="current-password" required; }
            button type="submit" { "Log in" }
        }
        p.meta { "No account yet? " a href="/register" { "Register" } " · " a href="/password/forgot" { "Forgot your password?" } }
    }
}

/// Asks for the address of an account whose password is to be reset.
pub fn forgot(message: Option<&str>) -> Markup {
    html! {
        h1 { "Forgot your password" }
        @if let Some(message) = message { p role="status" { (message) } }
        form method="post" action="/password/forgot" {
            label { "E-mail address" input type="email" name="email" autocomplete="email" maxlength="254" required; }
            button type="submit" { "Send a link" }
        }
        p.meta { a href="/login" { "Back to log in" } }
    }
}

/// The form for a new password, reached from the link in the mail.
pub fn reset(token: &str, error: Option<&str>) -> Markup {
    html! {
        h1 { "Set a new password" }
        @if let Some(error) = error { p.error role="alert" { (error) } }
        form method="post" action="/password/reset" {
            input type="hidden" name="token" value=(token);
            label { "New password (at least 10 characters)" input type="password" name="password" autocomplete="new-password" minlength="10" maxlength="256" required; }
            button type="submit" { "Set password" }
        }
    }
}

pub fn reset_unusable() -> Markup {
    html! {
        h1 { "This link cannot be used" }
        p { "It has expired or was used already. " a href="/password/forgot" { "Request a new link." } }
    }
}

pub fn reset_done() -> Markup {
    html! {
        h1 { "Password changed" }
        p { "You are signed out everywhere. " a href="/login" { "Log in with the new password." } }
    }
}

pub fn reports(rows: &[ReportRow], csrf: &str) -> Markup {
    html! {
        h1 { "Reports" }
        @if rows.is_empty() { p.meta { "No open reports." } }
        ul.list {
            @for row in rows {
                li {
                    p.meta { "Post " (row.post_id) " reported by " (row.reporter) " · " (when(row.created_at)) }
                    @if !row.reason.is_empty() { p { "Reason: " (row.reason) } }
                    blockquote { (row.body) }
                    a href=(format!("/threads/{}#post-{}", row.thread_id, row.post_id)) { "Open the post" }
                    form method="post" action=(format!("/admin/posts/{}/remove", row.post_id)) {
                        input type="hidden" name="csrf" value=(csrf);
                        button.ghost type="submit" { "Remove post" }
                    }
                }
            }
        }
    }
}

/// The account deletion: what is removed and what stays, and a password to confirm it.
pub fn delete_account(error: Option<&str>, csrf: &str) -> Markup {
    html! {
        h1 { "Delete your account" }
        p { "Your e-mail address, your password, your sessions, your picture, your attachments, your reactions, your votes and your memberships are deleted. This cannot be undone." }
        p { "Your posts and threads stay, so that the replies of others still make sense. They are shown as written by " strong { "[deleted]" } "." }
        @if let Some(error) = error { p.error role="alert" { (error) } }
        form method="post" action="/settings/delete" {
            input type="hidden" name="csrf" value=(csrf);
            label { "Your password, to confirm" input type="password" name="password" autocomplete="current-password" required; }
            button type="submit" { "Delete my account" }
        }
        p.meta { a href="/settings" { "Back to settings" } }
    }
}

