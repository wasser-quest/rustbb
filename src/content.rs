//! Post text. Everything is escaped. Besides plain links, the forum knows a few tags of its own,
//! so a post can quote or embed like a chat app, and anyone can write them by hand:
//!
//!   [b]bold[/b]   [i]italic[/i]   [code]raw text, not parsed[/code]
//!   [spoiler]hidden until opened[/spoiler]
//!   [quote=Name]what Name wrote[/quote]
//!   [embed title="Title" url="https://example.org" color="blue"]a card[/embed]
//!
//! Colors for embeds: rust (default), blue, green, gray. Tags nest up to MAX_DEPTH. A tag that is
//! unknown, not closed or badly written stays as typed. A URL is linked only if it is a single
//! token of safe characters, so `javascript:` and anything with quotes stay plain text.

use std::sync::atomic::{AtomicUsize, Ordering};

use maud::{html, Markup};

const LINK_REL: &str = "nofollow ugc noopener noreferrer";
const MAX_DEPTH: usize = 4;
/// The CSS has rules for this many tabs (`.t1` to `.t8`).
const MAX_TABS: usize = 8;
const MAX_ATTRIBUTE: usize = 300;
const COLORS: [&str; 4] = ["rust", "blue", "green", "gray"];

#[derive(Debug, PartialEq)]
enum Node<'a> {
    Text(&'a str),
    Bold(Vec<Node<'a>>),
    Italic(Vec<Node<'a>>),
    Code(&'a str),
    Spoiler(Vec<Node<'a>>),
    Quote { author: Option<&'a str>, body: Vec<Node<'a>> },
    Embed { title: Option<&'a str>, url: Option<&'a str>, color: &'static str, body: Vec<Node<'a>> },
    /// Tabs: the `Tab` nodes inside them become the tabs, in order.
    Tabs(Vec<Node<'a>>),
    Tab { title: &'a str, body: Vec<Node<'a>> },
}

impl Node<'_> {
    /// Blocks start on a line of their own, so a newline next to them is not shown.
    fn is_block(&self) -> bool {
        matches!(
            self,
            Node::Code(_) | Node::Spoiler(_) | Node::Quote { .. } | Node::Embed { .. } | Node::Tabs(_) | Node::Tab { .. }
        )
    }
}

pub fn render(body: &str) -> Markup {
    let body = body.replace("\r\n", "\n");
    let nodes = Parser { src: &body, pos: 0 }.nodes(None, 0).0;
    render_nodes(&nodes)
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    /// Reads nodes until the closing tag `close` (which is consumed), or the end of the text.
    /// The flag says whether the closing tag was found.
    fn nodes(&mut self, close: Option<&str>, depth: usize) -> (Vec<Node<'a>>, bool) {
        let src = self.src;
        let mut out = Vec::new();
        let mut text_start = self.pos;
        while let Some(c) = src[self.pos..].chars().next() {
            let rest = &src[self.pos..];
            if let Some(name) = close {
                if rest.starts_with(&format!("[/{name}]")) {
                    push_text(&mut out, &src[text_start..self.pos]);
                    self.pos += name.len() + 3;
                    return (out, true);
                }
            }
            if c == '[' {
                let start = self.pos;
                if let Some(node) = self.tag(depth) {
                    push_text(&mut out, &src[text_start..start]);
                    out.push(node);
                    text_start = self.pos;
                    continue;
                }
                self.pos = start + 1; // a literal bracket
                continue;
            }
            self.pos += c.len_utf8();
        }
        push_text(&mut out, &src[text_start..self.pos]);
        (out, false)
    }

    /// Reads a known tag at `pos`. Returns None, with `pos` unchanged, if there is no complete one.
    fn tag(&mut self, depth: usize) -> Option<Node<'a>> {
        let src = self.src;
        let start = self.pos;
        let rest = &src[start..];
        let end = rest.find(']')?;
        let inside = &rest[1..end];
        let name_end = inside.find(['=', ' ']).unwrap_or(inside.len());
        let (name, attributes) = (&inside[..name_end], &inside[name_end..]);
        self.pos = start + end + 1;
        let node = match name {
            "code" if attributes.is_empty() => self.raw_code(),
            "b" | "i" | "spoiler" if attributes.is_empty() && depth < MAX_DEPTH => {
                let (body, closed) = self.nodes(Some(name), depth + 1);
                closed.then(|| match name {
                    "b" => Node::Bold(body),
                    "i" => Node::Italic(body),
                    _ => Node::Spoiler(body),
                })
            }
            "quote" if depth < MAX_DEPTH => {
                let author = match attributes {
                    "" => Some(None),
                    _ => attributes.strip_prefix('=').filter(|name| valid_name(name)).map(Some),
                };
                let (body, closed) = self.nodes(Some(name), depth + 1);
                author.zip(closed.then_some(body)).map(|(author, body)| Node::Quote { author, body })
            }
            "tabs" if attributes.is_empty() && depth < MAX_DEPTH => {
                let (body, closed) = self.nodes(Some(name), depth + 1);
                closed.then(|| Node::Tabs(body))
            }
            "tab" if depth < MAX_DEPTH => {
                // The title is `[tab=Title]`, or `[tab title="Title"]`.
                let title = match attributes.strip_prefix('=') {
                    Some(title) => Some(title),
                    None => attributes_of(attributes).and_then(|pairs| {
                        pairs.iter().find(|(key, _)| *key == "title").map(|(_, value)| *value)
                    }),
                };
                let title = title.filter(|title| !title.trim().is_empty() && title.chars().count() <= 40);
                let (body, closed) = self.nodes(Some(name), depth + 1);
                title.zip(closed.then_some(body)).map(|(title, body)| Node::Tab { title, body })
            }
            "embed" if depth < MAX_DEPTH => {
                let pairs = attributes_of(attributes);
                let attribute = |key: &str| pairs.as_ref()?.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
                let color = attribute("color").and_then(|c| COLORS.iter().find(|known| **known == c).copied());
                let title = attribute("title");
                let url = attribute("url").filter(|url| is_safe_link(url));
                let (body, closed) = self.nodes(Some(name), depth + 1);
                closed.then(|| Node::Embed { title, url, color: color.unwrap_or("rust"), body })
            }
            _ => None,
        };
        if node.is_none() {
            self.pos = start;
        }
        node
    }

    /// The raw text of a code tag; its closing tag must come later in the text.
    fn raw_code(&mut self) -> Option<Node<'a>> {
        let src = self.src;
        let end = src[self.pos..].find("[/code]")?;
        let raw = &src[self.pos..self.pos + end];
        self.pos += end + "[/code]".len();
        Some(Node::Code(raw))
    }
}

fn push_text<'a>(out: &mut Vec<Node<'a>>, text: &'a str) {
    if !text.is_empty() {
        out.push(Node::Text(text));
    }
}

/// `key="value"` or `key=value` pairs, separated by spaces. None if the syntax is off.
fn attributes_of(text: &str) -> Option<Vec<(&str, &str)>> {
    let mut pairs = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let eq = rest.find('=')?;
        let key = &rest[..eq];
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        rest = &rest[eq + 1..];
        let (value, after) = match rest.strip_prefix('"') {
            Some(quoted) => {
                let end = quoted.find('"')?;
                (&quoted[..end], &quoted[end + 1..])
            }
            None => {
                let end = rest.find(' ').unwrap_or(rest.len());
                (&rest[..end], &rest[end..])
            }
        };
        if value.len() > MAX_ATTRIBUTE {
            return None;
        }
        pairs.push((key, value));
        rest = after.trim_start();
    }
    Some(pairs)
}

/// A quote's author: a username, so 1 to 32 letters, digits, `_` or `-`.
fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn render_nodes(nodes: &[Node<'_>]) -> Markup {
    html! {
        @for (index, node) in nodes.iter().enumerate() {
            @if let Node::Text(text) = node {
                @let after_block = index > 0 && nodes[index - 1].is_block();
                @let before_block = nodes.get(index + 1).is_some_and(Node::is_block);
                @let text = trim_at_blocks(text, after_block, before_block);
                (text_markup(text))
            } @else {
                (render_node(node))
            }
        }
    }
}

/// The newline that ends a line before or after a block is not a line of its own.
fn trim_at_blocks(text: &str, after_block: bool, before_block: bool) -> &str {
    let text = if after_block { text.strip_prefix('\n').unwrap_or(text) } else { text };
    if before_block { text.strip_suffix('\n').unwrap_or(text) } else { text }
}

fn render_node(node: &Node<'_>) -> Markup {
    html! {
        @match node {
            Node::Text(text) => (text_markup(text)),
            Node::Bold(body) => strong { (render_nodes(body)) },
            Node::Italic(body) => em { (render_nodes(body)) },
            Node::Code(raw) => pre { code { (raw) } },
            Node::Spoiler(body) => details.spoiler {
                summary { "Spoiler: show" }
                div { (render_nodes(body)) }
            },
            Node::Quote { author, body } => blockquote.quote {
                @if let Some(author) = author { cite { (author) } }
                (render_nodes(body))
            },
            Node::Tabs(children) => (tabs_markup(children)),
            Node::Tab { title, body } => div.tab-single {
                p.tab-title { (title) }
                div { (render_nodes(body)) }
            },
            Node::Embed { title, url, color, body } => div class=(format!("embed {color}")) {
                @if let Some(title) = title {
                    p.embed-title {
                        @if let Some(url) = url {
                            a href=(url) rel=(LINK_REL) { (title) }
                        } @else {
                            (title)
                        }
                    }
                }
                div.embed-body { (render_nodes(body)) }
            },
        }
    }
}

/// Tabs without scripts: one radio button per tab, and the CSS in style.css shows the panel of the
/// checked button (`.t1:checked ~ .panels > .p1`). Each block gets its own group name.
fn tabs_markup(children: &[Node<'_>]) -> Markup {
    static GROUPS: AtomicUsize = AtomicUsize::new(0);
    let tabs: Vec<(&str, &[Node<'_>])> = children
        .iter()
        .filter_map(|node| match node {
            Node::Tab { title, body } => Some((*title, body.as_slice())),
            _ => None,
        })
        .take(MAX_TABS)
        .collect();
    if tabs.is_empty() {
        return render_nodes(children);
    }
    let group = format!("tabs-{}", GROUPS.fetch_add(1, Ordering::Relaxed));
    html! {
        div.tabs {
            @for (index, _) in tabs.iter().enumerate() {
                input class=(format!("t{}", index + 1)) type="radio" name=(group) id=(format!("{group}-{index}")) checked?[index == 0];
            }
            div.tab-bar {
                @for (index, (title, _)) in tabs.iter().enumerate() {
                    label for=(format!("{group}-{index}")) { (title) }
                }
            }
            div.panels {
                @for (index, (_, body)) in tabs.iter().enumerate() {
                    section class=(format!("p{}", index + 1)) { (render_nodes(body)) }
                }
            }
        }
    }
}

/// Plain text: newlines become line breaks, and links are made where a word is a safe URL.
fn text_markup(text: &str) -> Markup {
    html! {
        @for (index, line) in text.split('\n').enumerate() {
            @if index > 0 { br; }
            (line_markup(line))
        }
    }
}

fn line_markup(line: &str) -> Markup {
    // Split on whitespace but keep it, so the line reads as typed.
    let mut parts: Vec<(&str, bool)> = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let start_ws = rest.len() - rest.trim_start().len();
        if start_ws > 0 {
            parts.push((&rest[..start_ws], false));
            rest = &rest[start_ws..];
            continue;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        parts.push((&rest[..end], true));
        rest = &rest[end..];
    }
    html! {
        @for (text, is_word) in &parts {
            @let (core, tail) = split_trailing(text);
            @if *is_word && is_safe_link(core) {
                a href=(core) rel=(LINK_REL) { (core) }
                (tail)
            } @else {
                (text)
            }
        }
    }
}

/// Splits sentence punctuation off the end of a word: ("page", ".") for "page.".
fn split_trailing(word: &str) -> (&str, &str) {
    let core = word.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
    (core, &word[core.len()..])
}

/// True for an absolute http or https URL made only of characters that cannot break out of an
/// attribute or start a script.
pub fn is_safe_link(word: &str) -> bool {
    let Some(rest) = word.strip_prefix("https://").or_else(|| word.strip_prefix("http://")) else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 2048
        && rest.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c))
        && !word.contains('\'')
        && !word.contains('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html_of(body: &str) -> String {
        render(body).into_string()
    }

    #[test]
    fn escapes_markup_in_text() {
        let out = html_of("<script>alert(1)</script>");
        assert!(!out.contains("<script>"));
        assert!(out.contains("&lt;script&gt;"));
    }

    #[test]
    fn links_plain_http_urls_with_nofollow() {
        let out = html_of("see https://example.org/page.");
        assert!(out.contains(r#"href="https://example.org/page""#), "{out}");
        assert!(out.contains("nofollow ugc noopener noreferrer"));
    }

    #[test]
    fn never_links_script_schemes_or_attribute_breakers() {
        for word in ["javascript:alert(1)", "data:text/html,x", "https://x.org/\"onmouseover=alert(1)", "https://x.org/'a", "ftp://x.org"] {
            assert!(!is_safe_link(word), "{word}");
            assert!(!html_of(word).contains("<a "), "{word}");
        }
    }

    #[test]
    fn keeps_line_breaks_and_windows_line_ends() {
        assert_eq!(html_of("one\ntwo").matches("<br>").count(), 1);
        assert_eq!(html_of("one\r\ntwo").matches("<br>").count(), 1);
    }

    #[test]
    fn quotes_show_their_author_and_nest_formatting() {
        let out = html_of("[quote=alice]hi [b]there[/b][/quote]");
        assert!(out.contains(r#"<blockquote class="quote"><cite>alice</cite>"#), "{out}");
        assert!(out.contains("<strong>there</strong>"), "{out}");
    }

    #[test]
    fn an_author_must_be_a_username() {
        let out = html_of("[quote=<b>x</b>]text[/quote]");
        assert!(!out.contains("<blockquote"), "{out}");
        assert!(out.contains("[quote="), "{out}");
    }

    #[test]
    fn embeds_have_a_title_a_link_and_a_color_from_the_list() {
        let out = html_of(r#"[embed title="Docs" url="https://example.org/docs" color="blue"]Read me[/embed]"#);
        assert!(out.contains(r#"class="embed blue""#), "{out}");
        assert!(out.contains(r#"<a href="https://example.org/docs""#), "{out}");
        assert!(out.contains(">Docs</a>"), "{out}");
        assert!(out.contains("Read me"), "{out}");
        let unknown = html_of(r#"[embed color="red" title="x"]y[/embed]"#);
        assert!(unknown.contains(r#"class="embed rust""#), "{unknown}");
    }

    #[test]
    fn an_embed_never_links_an_unsafe_url_and_escapes_its_title() {
        let out = html_of(r#"[embed title="<i>T</i>" url="javascript:alert(1)"]x[/embed]"#);
        assert!(!out.contains("javascript:"), "{out}");
        assert!(!out.contains("<a "), "{out}");
        assert!(out.contains("&lt;i&gt;T&lt;/i&gt;"), "{out}");
    }

    #[test]
    fn code_is_shown_as_typed() {
        let out = html_of("[code][b]x[/b][/code]");
        assert!(out.contains("<pre><code>[b]x[/b]</code></pre>"), "{out}");
    }

    #[test]
    fn unknown_or_unclosed_tags_stay_as_typed() {
        let out = html_of("[x]a[/x] [b]open");
        assert!(out.contains("[x]a[/x]"), "{out}");
        assert!(out.contains("[b]open"), "{out}");
        assert!(!out.contains("<strong>"), "{out}");
    }

    #[test]
    fn spoilers_are_closed_by_details_without_scripts() {
        let out = html_of("[spoiler]secret[/spoiler]");
        assert!(out.contains(r#"<details class="spoiler">"#), "{out}");
        assert!(out.contains("secret"), "{out}");
    }

    #[test]
    fn tabs_inside_an_embed_show_one_panel_per_tab() {
        let out = html_of(r#"[embed title="Setup"][tabs][tab=Linux]run this[/tab][tab title="Windows"]or that[/tab][/tabs][/embed]"#);
        assert!(out.contains(r#"class="tabs""#), "{out}");
        assert_eq!(out.matches(r#"type="radio""#).count(), 2, "{out}");
        assert!(out.contains(r#"<label for="tabs-"#) && out.contains(">Linux</label>"), "{out}");
        assert!(out.contains(">Windows</label>"), "{out}");
        assert!(out.contains(r#"<section class="p1">run this</section>"#), "{out}");
        assert!(out.contains(r#"<section class="p2">or that</section>"#), "{out}");
    }

    #[test]
    fn a_tab_needs_a_title_and_tabs_without_tabs_stay_text() {
        assert!(!html_of("[tabs][tab=]x[/tab][/tabs]").contains(r#"class="tabs""#));
        assert!(!html_of("[tabs]just text[/tabs]").contains(r#"class="tabs""#));
    }

    #[test]
    fn each_tabs_block_has_its_own_group() {
        let out = html_of("[tabs][tab=a]1[/tab][/tabs] [tabs][tab=b]2[/tab][/tabs]");
        let groups: Vec<&str> = out.match_indices(r#"name="tabs-"#).map(|(index, _)| &out[index..index + 14]).collect();
        assert_eq!(groups.len(), 2, "{out}");
        assert_ne!(groups[0], groups[1], "{out}");
    }

    #[test]
    fn nesting_stops_at_the_depth_limit() {
        let deep = "[b]".repeat(12) + "x" + &"[/b]".repeat(12);
        let out = html_of(&deep);
        assert!(out.contains('x'));
        assert!(out.contains("[b]"), "the tags past the limit stay as typed");
    }
}
