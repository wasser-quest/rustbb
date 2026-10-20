//! The reference for the tags a post may use. The renderer is in content.rs.

use axum::extract::State;
use maud::{html, Markup};

use crate::auth::MaybeViewer;
use crate::views;
use crate::AppState;

pub async fn help(State(_app): State<AppState>, MaybeViewer(viewer): MaybeViewer) -> Markup {
    views::page("Formatting", viewer.as_ref(), page())
}

fn page() -> Markup {
    html! {
        h1 { "Formatting posts" }
        p { "Posts are plain text. Line breaks are kept, and a web address is turned into a link. The tags below are the only markup. Anything else is shown as typed." }
        table.tags {
            thead { tr { th { "Write" } th { "You get" } } }
            tbody {
                tr { td { code { "[b]bold[/b]" } } td { "bold" } }
                tr { td { code { "[i]italic[/i]" } } td { "italic" } }
                tr { td { code { "[code]raw text, [b] is not read[/code]" } } td { "a block of code" } }
                tr { td { code { "[spoiler]hidden text[/spoiler]" } } td { "text that opens on a click" } }
                tr { td { code { "[quote=name]what name wrote[/quote]" } } td { "a quote with the author's name" } }
                tr {
                    td { code { "[embed title=\"Title\" url=\"https://example.org\" color=\"blue\"]a card[/embed]" } }
                    td { "a card with a title, a link and a color" }
                }
            }
        }
        h2 { "Embeds" }
        p { "title, url and color are optional. The url must start with https:// or http://. The colors are rust (the default), blue, green and gray. Values cannot contain a closing bracket." }
        h2 { "Nesting" }
        p { "Tags can be put inside each other, up to four levels deep. A tag that is not closed stays as typed." }
    }
}
