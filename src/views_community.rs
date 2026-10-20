//! Pages of the community features: notifications, polls, profiles, settings, the admin switches
//! and private messages. maud escapes every value.

use chrono::{DateTime, Datelike, Utc};
use maud::{html, Markup};

use crate::auth::Viewer;
use crate::content;
use crate::messages::{InboxRow, MessageRow};
use crate::models::{ProfileRow, ThreadRow};
use crate::notify::NotificationRow;
use crate::reactions::{Kind, Tally};
use crate::polls::PollView;
use crate::profile::rank;
use crate::views::{avatar, when};

pub fn notifications(rows: &[NotificationRow], csrf: &str) -> Markup {
    html! {
        h1 { "Notifications" }
        @if rows.is_empty() { p.meta { "Nothing yet. Follow a thread to hear about its replies, or wait for a mention." } }
        @if rows.iter().any(|row| row.unread) {
            form method="post" action="/notifications/read" {
                input type="hidden" name="csrf" value=(csrf);
                button.ghost type="submit" { "Mark all as read" }
            }
        }
        ul.list {
            @for row in rows {
                li class=(if row.unread { "unread" } else { "" }) {
                    (notification_text(row))
                    p.meta { (when(row.created_at)) }
                }
            }
        }
    }
}

fn notification_text(row: &NotificationRow) -> Markup {
    let who = row.actor.as_deref().unwrap_or("Someone");
    let thread = row.thread_title.as_deref().unwrap_or("a thread");
    html! {
        @match row.kind.as_str() {
            "reply" => a href=(reply_link(row)) { (who) " replied in " (thread) },
            "mention" => a href=(reply_link(row)) { (who) " mentioned you in " (thread) },
            _ => a href=(format!("/messages/{}", row.conversation_id.unwrap_or(0))) { (who) " sent you a private message" },
        }
    }
}

fn reply_link(row: &NotificationRow) -> String {
    match (row.thread_id, row.post_id) {
        (Some(thread), Some(post)) => format!("/threads/{thread}#post-{post}"),
        (Some(thread), None) => format!("/threads/{thread}"),
        _ => "/".to_owned(),
    }
}

/// The reactions under a post: a button per kind for members (the ones they gave are marked), and
/// only the kinds that have been given, as text, for visitors.
pub fn reaction_bar(post_id: i64, kinds: &[Kind], tallies: &[Tally], csrf: Option<&str>) -> Markup {
    html! {
        @if !kinds.is_empty() {
            div.reactions aria-label="Reactions" {
                @for kind in kinds {
                    @let tally = tallies.iter().find(|tally| tally.kind_id == kind.id);
                    @let count = tally.map_or(0, |tally| tally.count);
                    @let mine = tally.is_some_and(|tally| tally.mine);
                    @if let Some(csrf) = csrf {
                        form.inline method="post" action=(format!("/posts/{post_id}/react")) {
                            input type="hidden" name="csrf" value=(csrf);
                            input type="hidden" name="kind" value=(kind.id);
                            button class=(if mine { "reaction mine" } else { "reaction" }) type="submit" title=(kind.name) aria-pressed=(mine) {
                                span.symbol { (kind.symbol) }
                                @if count > 0 { span.n { (count) } }
                            }
                        }
                    } @else if count > 0 {
                        span.reaction title=(kind.name) {
                            span.symbol { (kind.symbol) }
                            span.n { (count) }
                        }
                    }
                }
            }
        }
    }
}

/// The reaction kinds under Admin > Reactions: the list with a remove button each, and the form to add one.
pub fn admin_reactions(kinds: &[Kind], csrf: &str) -> Markup {
    html! {
        h1 { "Reactions" }
        p.meta { "Members can react to posts with these. A symbol is an emoji or a short text." }
        @if kinds.is_empty() { p.meta { "No reactions yet." } }
        ul.list {
            @for kind in kinds {
                li {
                    span.symbol { (kind.symbol) } " " strong { (kind.name) }
                    form.inline method="post" action=(format!("/admin/reactions/{}/delete", kind.id)) {
                        input type="hidden" name="csrf" value=(csrf);
                        button.ghost type="submit" { "Remove" }
                    }
                }
            }
        }
        h2 { "Add a reaction" }
        form method="post" action="/admin/reactions" {
            input type="hidden" name="csrf" value=(csrf);
            label { "Symbol (an emoji or up to 16 characters)" input type="text" name="symbol" maxlength="16" required; }
            label { "Name (shown as the hint)" input type="text" name="name" maxlength="32" required; }
            button type="submit" { "Add" }
        }
    }
}

/// A poll under its thread: the vote buttons for members, and the results for everyone.
pub fn poll(poll: &PollView, csrf: Option<&str>) -> Markup {
    let shown_total = poll.total.max(1);
    html! {
        section.poll id="poll" aria-label="Poll" {
            h2 { (poll.question) }
            form method="post" action=(format!("/polls/{}/vote", poll.id)) {
                @if let Some(csrf) = csrf { input type="hidden" name="csrf" value=(csrf); }
                @for option in &poll.options {
                    label.poll-option {
                        @if csrf.is_some() {
                            input type="radio" name="option" value=(option.id) checked?[poll.mine == Some(option.id)];
                        }
                        span.poll-text { (option.text) }
                        progress max=(shown_total) value=(option.votes) {}
                        span.meta { (option.votes) " · " (percent(option.votes, poll.total)) "%" }
                    }
                }
                @if csrf.is_some() { button.ghost type="submit" { "Vote" } }
            }
            p.meta { (poll.total) " " (if poll.total == 1 { "vote" } else { "votes" }) }
        }
    }
}

/// The end of a ban; a permanent ban is stored as the year 9999.
fn until_text(until: Option<DateTime<Utc>>) -> String {
    match until {
        Some(until) if until.year() >= 9999 => "further notice".to_owned(),
        Some(until) => when(until),
        None => String::new(),
    }
}

fn percent(votes: i64, total: i64) -> i64 {
    if total == 0 { 0 } else { votes * 100 / total }
}

pub fn profile(profile: &ProfileRow, recent: &[ThreadRow], viewer: Option<&Viewer>) -> Markup {
    let banned = profile.banned_until.is_some_and(|until| until > Utc::now());
    html! {
        div.profile-head {
            (avatar(&profile.username, profile.avatar_id.as_deref()))
            div {
                h1 { (profile.username) }
                p.meta {
                    (rank(profile.posts))
                    @if let Some(title) = &profile.title { " · " (title) }
                    " · " (profile.posts) " posts · member since " (when(profile.created_at))
                }
            }
        }
        @if banned {
            p.error role="status" { "This account is banned." }
        }
        @if let Some(viewer) = viewer {
            @if viewer.messages && viewer.user_id != profile.id {
                p { a.button href=(format!("/messages/new/{}", profile.username)) { "Write a message" } }
            }
        }
        h2 { "Recent threads" }
        ul.list {
            @for thread in recent {
                li {
                    a href=(crate::views::thread_path(thread.id, &thread.title)) { (thread.title) }
                    p.meta { (when(thread.last_post_at)) }
                }
            }
        }
        @if let Some(viewer) = viewer {
            @if viewer.is_admin && viewer.user_id != profile.id {
                (admin_tools(profile, banned, &viewer.csrf))
            }
        }
    }
}

/// What an admin sees under a profile: the title, and the ban or the unban.
fn admin_tools(profile: &ProfileRow, banned: bool, csrf: &str) -> Markup {
    let action = |path: &str| format!("/admin/users/{}/{path}", profile.username);
    html! {
        section.admin-tools aria-label="Moderation" {
            h2 { "Moderation" }
            form method="post" action=(action("title")) {
                input type="hidden" name="csrf" value=(csrf);
                label { "Title under the name (empty removes it)"
                    input type="text" name="title" value=(profile.title.as_deref().unwrap_or("")) maxlength="40";
                }
                button type="submit" { "Save title" }
            }
            @if banned {
                p.meta { "Banned until " (until_text(profile.banned_until)) "." }
                @if !profile.ban_reason.is_empty() { p.meta { "Reason: " (profile.ban_reason) } }
                form method="post" action=(action("unban")) {
                    input type="hidden" name="csrf" value=(csrf);
                    button.ghost type="submit" { "Lift the ban" }
                }
            } @else {
                form method="post" action=(action("ban")) {
                    input type="hidden" name="csrf" value=(csrf);
                    label { "Ban for"
                        select name="duration" {
                            option value="1" { "1 day" }
                            option value="7" selected { "7 days" }
                            option value="30" { "30 days" }
                            option value="365" { "one year" }
                            option value="permanent" { "permanently" }
                        }
                    }
                    label { "Reason (shown to the admins)" input type="text" name="reason" maxlength="300"; }
                    button type="submit" { "Ban" }
                }
            }
        }
    }
}

pub fn settings(viewer: &Viewer, avatar_id: Option<&str>, error: Option<&str>) -> Markup {
    html! {
        h1 { "Settings" }
        h2 { "Picture" }
        p.meta { "A picture of up to 1 MB: PNG, JPEG, GIF or WebP. It is shown beside your posts and on your profile." }
        @if let Some(avatar_id) = avatar_id {
            p { img.avatar.large src=(format!("/avatars/{avatar_id}")) alt="Your picture" width="96" height="96"; }
        }
        @if let Some(error) = error { p.error role="alert" { (error) } }
        form method="post" action="/settings/avatar" enctype="multipart/form-data" {
            input type="hidden" name="csrf" value=(viewer.csrf);
            label { "Picture"
                input type="file" name="avatar" accept="image/png,image/jpeg,image/gif,image/webp";
            }
            button type="submit" { "Save picture" }
        }
        @if avatar_id.is_some() {
            form method="post" action="/settings/avatar/remove" {
                input type="hidden" name="csrf" value=(viewer.csrf);
                button.ghost type="submit" { "Remove picture" }
            }
        }
        p.meta { a href=(format!("/users/{}", viewer.username)) { "Your profile" } " · " a href="/settings/delete" { "Delete your account" } }
    }
}

pub fn admin_settings(viewer: &Viewer, messages: bool) -> Markup {
    html! {
        h1 { "Site settings" }
        form method="post" action="/admin/settings" {
            input type="hidden" name="csrf" value=(viewer.csrf);
            label.check {
                input type="checkbox" name="private_messages" value="on" checked?[messages];
                "Private messages between members (off by default)"
            }
            p.meta { "Members can write to each other from a profile only while this is on." }
            button type="submit" { "Save" }
        }
    }
}

pub fn inbox(rows: &[InboxRow]) -> Markup {
    html! {
        h1 { "Messages" }
        p.meta { "Write to a member from their profile." }
        @if rows.is_empty() { p.meta { "No conversations yet." } }
        ul.list {
            @for row in rows {
                li class=(if row.unread { "unread" } else { "" }) {
                    a href=(format!("/messages/{}", row.id)) { (row.subject) }
                    p.meta {
                        "with " (row.others.as_deref().unwrap_or("nobody")) " · " (when(row.updated_at))
                        @if row.unread { " · new" }
                    }
                }
            }
        }
    }
}

pub fn compose(viewer: &Viewer, username: &str, error: Option<&str>, subject: &str, body: &str) -> Markup {
    html! {
        p.crumbs { a href="/messages" { "Messages" } " / " (username) }
        h1 { "Message to " (username) }
        @if let Some(error) = error { p.error role="alert" { (error) } }
        form method="post" action=(format!("/messages/new/{username}")) {
            input type="hidden" name="csrf" value=(viewer.csrf);
            label { "Subject" input type="text" name="subject" value=(subject) maxlength="120" required; }
            label { "Message" textarea name="body" rows="8" maxlength="10000" required { (body) } }
            button type="submit" { "Send" }
        }
    }
}

pub fn conversation(id: i64, subject: &str, messages: &[MessageRow], viewer: &Viewer) -> Markup {
    html! {
        p.crumbs { a href="/messages" { "Messages" } " / " (subject) }
        h1 { (subject) }
        @for message in messages {
            article.post {
                div.post-head {
                    (avatar(&message.author, None))
                    div {
                        p.meta { strong { (message.author) } " · " (when(message.created_at)) }
                    }
                }
                div.body { (content::render(&message.body)) }
            }
        }
        h2 { "Reply" }
        form method="post" action=(format!("/messages/{id}/reply")) {
            input type="hidden" name="csrf" value=(viewer.csrf);
            label { "Message" textarea name="body" rows="5" maxlength="10000" required {} }
            button type="submit" { "Send" }
        }
    }
}
