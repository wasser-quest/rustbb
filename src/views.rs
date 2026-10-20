//! Browsing pages. maud escapes every value; post text goes through content::render.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use maud::{html, Markup, DOCTYPE};

use crate::auth::Viewer;
use crate::content;
use crate::models::{AttachmentRow, Board, PostRow, SearchRow, ThreadHead, ThreadRow};
use crate::polls::PollView;
use crate::reactions::{Kind, Tally};
use crate::views_community;

/// What a thread page shows besides its posts: the poll, whether the viewer follows the thread, and
/// the reaction kinds with the tallies of each post.
pub struct ThreadExtras<'a> {
    pub poll: Option<&'a PollView>,
    pub following: bool,
    pub kinds: &'a [Kind],
    pub tallies: &'a HashMap<i64, Vec<Tally>>,
}

/// What search engines and link previews read about a page. A page without it gets its title only.
#[derive(Default)]
pub struct Seo {
    /// One or two sentences about the page, without markup.
    pub description: Option<String>,
    /// The page's own absolute address (the public address of the forum plus the path).
    pub canonical: Option<String>,
}

pub fn page(title: &str, viewer: Option<&Viewer>, content: Markup) -> Markup {
    page_seo(title, &Seo::default(), viewer, content)
}

pub fn page_seo(title: &str, seo: &Seo, viewer: Option<&Viewer>, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="theme-color" content="#1c1915";
                title { (title) " · rustbb" }
                @if let Some(description) = &seo.description { meta name="description" content=(description); }
                @if let Some(canonical) = &seo.canonical { link rel="canonical" href=(canonical); }
                meta property="og:site_name" content="rustbb";
                meta property="og:type" content="website";
                meta property="og:title" content=(title);
                @if let Some(description) = &seo.description { meta property="og:description" content=(description); }
                @if let Some(canonical) = &seo.canonical { meta property="og:url" content=(canonical); }
                link rel="stylesheet" href="/style.css";
            }
            body {
                a.skip href="#content" { "Skip to content" }
                header.top {
                    a.brand href="/" { span.mark aria-hidden="true" { "rb" } span { "rustbb" } }
                    form.search method="get" action="/search" {
                        input type="search" name="q" placeholder="Search threads" maxlength="100" aria-label="Search threads";
                    }
                    nav {
                        @if let Some(viewer) = viewer {
                            a href="/notifications" {
                                "Notifications"
                                @if viewer.unread > 0 { span.count { (viewer.unread) } }
                            }
                            @if viewer.messages { a href="/messages" { "Messages" } }
                            @if viewer.is_admin {
                                a href="/admin/reports" { "Reports" }
                                a href="/admin/reactions" { "Reactions" }
                                a href="/admin/settings" { "Site" }
                            }
                            a href=(format!("/users/{}", viewer.username)) { (viewer.username) }
                            a href="/settings" { "Settings" }
                            form.inline method="post" action="/logout" {
                                input type="hidden" name="csrf" value=(viewer.csrf);
                                button type="submit" { "Log out" }
                            }
                        } @else {
                            a href="/login" { "Log in" }
                            a href="/register" { "Register" }
                        }
                    }
                }
                main #content { (content) }
                footer {
                    p { "No scripts run on this site. " a href="/help" { "How to format posts" } }
                }
            }
        }
    }
}

/// The address of a thread: `/threads/<id>/<words of the title>`. The id decides; the words are for
/// people and search engines, and a different spelling of them still reaches the thread.
pub fn thread_path(id: i64, title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase).flat_map(|c| match c {
        'ä' => vec!['a', 'e'],
        'ö' => vec!['o', 'e'],
        'ü' => vec!['u', 'e'],
        'ß' => vec!['s', 's'],
        _ => vec![c],
    }) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 60 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() { format!("/threads/{id}") } else { format!("/threads/{id}/{slug}") }
}

/// A description for search results: the start of a post as plain text, about 160 characters.
pub fn summary(body: &str) -> String {
    let mut plain = String::new();
    let mut in_tag = false;
    for c in body.chars() {
        match c {
            '[' => in_tag = true,
            ']' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = plain.split_whitespace().collect();
    let mut text = String::new();
    for word in words {
        if text.chars().count() + word.chars().count() > 157 {
            return format!("{}…", text.trim_end());
        }
        text.push_str(word);
        text.push(' ');
    }
    text.trim_end().to_owned()
}

pub fn when(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%d %H:%M UTC").to_string()
}

/// Previous and next links for a paged list.
pub fn pager(base: &str, page: u32, has_next: bool) -> Markup {
    html! {
        nav.pager aria-label="Pages" {
            @if page > 1 { a href=(format!("{base}?page={}", page - 1)) { "Previous" } }
            span.meta { "Page " (page) }
            @if has_next { a href=(format!("{base}?page={}", page + 1)) { "Next" } }
        }
    }
}

pub fn boards(boards: &[Board]) -> Markup {
    html! {
        h1 { "Boards" }
        ul.list {
            @for board in boards {
                li {
                    a href=(format!("/boards/{}", board.slug)) { (board.title) }
                    p.meta { (board.description) }
                }
            }
        }
    }
}

pub fn board(board: &Board, threads: &[ThreadRow], viewer: Option<&Viewer>, page_no: u32, has_next: bool) -> Markup {
    html! {
        p.crumbs { a href="/" { "Boards" } " / " (board.title) }
        h1 { (board.title) }
        p.meta { (board.description) }
        @if viewer.is_some() {
            p { a.button href=(format!("/boards/{}/threads/new", board.slug)) { "New thread" } }
        } @else {
            p.meta { a href="/login" { "Log in" } " to start a thread." }
        }
        @if threads.is_empty() {
            p.meta { "No threads on this page." }
        }
        ul.list {
            @for thread in threads {
                li {
                    a href=(thread_path(thread.id, &thread.title)) { (thread.title) }
                    p.meta {
                        a href=(format!("/users/{}", thread.author)) { (thread.author) }
                        " · " (thread.replies) " replies · last activity " (when(thread.last_post_at))
                    }
                }
            }
        }
        (pager(&format!("/boards/{}", board.slug), page_no, has_next))
    }
}

/// The author's picture, or the first letter of the name when there is no picture.
pub fn avatar(username: &str, avatar_id: Option<&str>) -> Markup {
    html! {
        @if let Some(id) = avatar_id {
            img.avatar src=(format!("/avatars/{id}")) alt="" width="40" height="40" loading="lazy";
        } @else {
            span.avatar.avatar-initial aria-hidden="true" { (username.chars().next().unwrap_or('?').to_uppercase()) }
        }
    }
}

/// A post with its image attachments. Deleted posts keep their place but show no content.
fn post_card(
    post: &PostRow,
    attachments: &[&AttachmentRow],
    viewer: Option<&Viewer>,
    anchor_thread: i64,
    extras: &ThreadExtras<'_>,
) -> Markup {
    let can_delete = viewer.is_some_and(|v| v.user_id == post.author_id);
    let tallies = extras.tallies.get(&post.id).map_or(&[][..], Vec::as_slice);
    html! {
        article.post id=(format!("post-{}", post.id)) {
            div.post-head {
                (avatar(&post.author, post.avatar_id.as_deref()))
                div {
                    p.meta {
                        a href=(format!("/users/{}", post.author)) { strong { (post.author) } }
                        " · " (crate::profile::rank(post.author_posts))
                        @if let Some(title) = &post.author_title { " · " (title) }
                    }
                    p.meta { (when(post.created_at)) }
                }
            }
            @if post.deleted {
                p.meta { "This post was removed." }
            } @else {
                div.body { (content::render(&post.body)) }
                @for file in attachments {
                    img.attachment src=(format!("/files/{}", file.id)) alt="Image attached to this post" loading="lazy";
                }
                (views_community::reaction_bar(post.id, extras.kinds, tallies, viewer.map(|v| v.csrf.as_str())))
                @if let Some(viewer) = viewer {
                    div.actions {
                        @if can_delete {
                            form.inline method="post" action=(format!("/posts/{}/delete", post.id)) {
                                input type="hidden" name="csrf" value=(viewer.csrf);
                                input type="hidden" name="thread" value=(anchor_thread);
                                button.ghost type="submit" { "Delete" }
                            }
                        }
                        form.inline method="post" action=(format!("/posts/{}/report", post.id)) {
                            input type="hidden" name="csrf" value=(viewer.csrf);
                            input type="hidden" name="thread" value=(anchor_thread);
                            input type="text" name="reason" placeholder="Why? (optional)" maxlength="300" aria-label="Reason for the report";
                            button.ghost type="submit" { "Report" }
                        }
                    }
                }
            }
        }
    }
}

pub fn thread(
    head: &ThreadHead,
    posts: &[PostRow],
    attachments: &[AttachmentRow],
    viewer: Option<&Viewer>,
    page_no: u32,
    has_next: bool,
    extras: &ThreadExtras<'_>,
) -> Markup {
    html! {
        p.crumbs {
            a href="/" { "Boards" } " / "
            a href=(format!("/boards/{}", head.board_slug)) { (head.board_title) } " / "
            (head.title)
        }
        h1 { (head.title) }
        @if let Some(viewer) = viewer {
            form.inline method="post" action=(format!("/threads/{}/follow", head.id)) {
                input type="hidden" name="csrf" value=(viewer.csrf);
                input type="hidden" name="action" value=(if extras.following { "unfollow" } else { "follow" });
                button.ghost type="submit" { (if extras.following { "Following: stop" } else { "Follow replies" }) }
            }
        }
        @if let Some(poll) = extras.poll {
            (views_community::poll(poll, viewer.map(|v| v.csrf.as_str())))
        }
        @for post in posts {
            @let files: Vec<&AttachmentRow> = attachments.iter().filter(|a| a.post_id == post.id).collect();
            (post_card(post, &files, viewer, head.id, extras))
        }
        (pager(&format!("/threads/{}", head.id), page_no, has_next))
    }
}

pub fn search(query: &str, rows: &[SearchRow]) -> Markup {
    html! {
        h1 { "Search" }
        form method="get" action="/search" {
            label { "Threads with this text in the title" input type="search" name="q" value=(query) maxlength="100" required; }
            button type="submit" { "Search" }
        }
        @if rows.is_empty() && !query.is_empty() { p.meta { "No threads found." } }
        ul.list {
            @for row in rows {
                li {
                    a href=(thread_path(row.id, &row.title)) { (row.title) }
                    p.meta {
                        (row.board_slug) " · " (row.author) " · " (when(row.last_post_at))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_addresses_keep_the_id_and_read_the_title() {
        assert_eq!(thread_path(4, "Hallo, Welt!"), "/threads/4/hallo-welt");
        assert_eq!(thread_path(9, "Größe & Übung"), "/threads/9/groesse-uebung");
        assert_eq!(thread_path(2, "???"), "/threads/2");
        assert!(thread_path(1, &"a ".repeat(100)).len() < 80);
    }

    #[test]
    fn summaries_are_plain_text_and_short() {
        assert_eq!(summary("[b]Hello[/b] [quote=x]there[/quote] friend"), "Hello there friend");
        let long = summary(&"word ".repeat(100));
        assert!(long.chars().count() <= 160, "{long}");
        assert!(long.ends_with('…'));
    }
}
