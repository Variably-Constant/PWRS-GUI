//! A Markdown widget's document as the HTML its page shows, read as
//! [`proto::markdown`] says, as the egui renderer reads it. Of raw HTML the
//! tags both renderers draw are written as tags, without their attributes,
//! and every other is written as text. A link has its address for the
//! runtime to raise Click with and a heading its `{#id}` for a link to
//! scroll to, both in data attributes the page does not act on by itself.
//! An image loads the picture its window has for it or its https address,
//! or shows its alt text. Code is colored by egui_extras's rules, and
//! GitHub's alerts are drawn as the egui renderer draws them.

use crate::proto::markdown::{self, Html, Token, pulldown_cmark};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Tag, TagEnd};

/// `text` as HTML, `served` returning the address a page loads the picture
/// the session read for an image's address from, if any.
pub fn html(text: &str, served: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out: Vec<Event<'_>> = Vec::new();
    let mut events = pulldown_cmark::Parser::new_ext(text, markdown::options());
    while let Some(event) = events.next() {
        match event {
            Event::Html(raw) | Event::InlineHtml(raw) => out.extend(raw_html(&raw)),
            Event::Start(Tag::Link { dest_url, .. }) => {
                let address = escaped(&dest_url);
                out.push(tag(format!("<a href=\"#\" data-link=\"{address}\" title=\"{address}\">")));
            }
            Event::End(TagEnd::Link) => out.push(tag(String::from("</a>"))),
            Event::Start(Tag::Image { dest_url, .. }) => {
                let alt = alt_text(&mut events);
                out.push(image(&dest_url, &alt, served));
            }
            Event::Start(Tag::Heading { level, id, .. }) => {
                let named = match id {
                    Some(id) => format!(" data-heading=\"{}\"", escaped(&id)),
                    None => String::new(),
                };
                out.push(tag(format!("<{level}{named}>")));
            }
            Event::End(TagEnd::Heading(level)) => out.push(tag(format!("</{level}>\n"))),
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(language) if !language.is_empty() => Some(language),
                    CodeBlockKind::Fenced(_) | CodeBlockKind::Indented => None,
                };
                let code = code_text(&mut events);
                out.push(tag(code_html(&code, language.as_deref())));
            }
            Event::FootnoteReference(name) => out.push(tag(format!("<sup class=\"footnote-reference\">{}</sup>", escaped(&name)))),
            Event::Start(Tag::FootnoteDefinition(name)) => out.push(tag(format!("<div class=\"footnote-definition\"><span class=\"footnote-label\">{}.</span>", escaped(&name)))),
            Event::End(TagEnd::FootnoteDefinition) => out.push(tag(String::from("</div>\n"))),
            other => out.push(other),
        }
    }
    mark_alerts(&mut out);
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, out.into_iter());
    html
}

/// HTML the renderer writes itself, passed through as it is.
fn tag(html: String) -> Event<'static> {
    Event::InlineHtml(CowStr::from(html))
}

/// `text` with the characters HTML gives a meaning escaped, for text or a
/// value in double quotes.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Raw HTML as both renderers draw it: each drawn tag without its
/// attributes, and everything else as the text it is written as.
fn raw_html(raw: &str) -> Vec<Event<'static>> {
    let mut out = Vec::new();
    for piece in markdown::html_pieces(raw) {
        match piece {
            Html::Open("br") => out.push(tag(String::from("<br>"))),
            Html::Close("br") => {}
            Html::Open(name) => out.push(tag(format!("<{name}>"))),
            Html::Close(name) => out.push(tag(format!("</{name}>"))),
            Html::Text(text) => out.push(Event::Text(CowStr::from(text.to_string()))),
        }
    }
    out
}

/// An image's alt text, its events taken up to the image's end.
fn alt_text<'a>(events: &mut impl Iterator<Item = Event<'a>>) -> String {
    let mut alt = String::new();
    let mut inside = 0;
    for event in events.by_ref() {
        match event {
            Event::Start(Tag::Image { .. }) => inside += 1,
            Event::End(TagEnd::Image) if inside == 0 => break,
            Event::End(TagEnd::Image) => inside -= 1,
            Event::Text(text) | Event::Code(text) => alt.push_str(&text),
            Event::SoftBreak | Event::HardBreak => alt.push(' '),
            _ => {}
        }
    }
    alt
}

/// An image: the picture the window has for it, or its https address,
/// which the page draws its alt text in place of when it cannot load it;
/// for any other address, its alt text.
fn image(address: &str, alt: &str, served: &dyn Fn(&str) -> Option<String>) -> Event<'static> {
    let src = match served(address) {
        Some(src) => src,
        None => match markdown::source(address) {
            markdown::Source::Web(url) => url.to_string(),
            markdown::Source::Data(_) | markdown::Source::File(_) | markdown::Source::Refused(_) => return Event::Text(CowStr::from(alt.to_string())),
        },
    };
    let alt = escaped(alt);
    tag(format!("<img src=\"{}\" alt=\"{alt}\" title=\"{alt}\" data-address=\"{}\">", escaped(&src), escaped(address)))
}

/// A code block's text, its events taken up to the block's end.
fn code_text<'a>(events: &mut impl Iterator<Item = Event<'a>>) -> String {
    let mut code = String::new();
    for event in events.by_ref() {
        match event {
            Event::End(TagEnd::CodeBlock) => break,
            Event::Text(text) => code.push_str(&text),
            _ => {}
        }
    }
    code
}

/// A code block in a `pre`, as the egui renderer draws it: its pieces
/// colored by egui_extras's rules for a language they cover, gray for any
/// other language, and plain with none.
fn code_html(code: &str, language: Option<&str>) -> String {
    let code = code.strip_suffix('\n').unwrap_or(code);
    let mut out = String::from("<pre class=\"code\"><code>");
    match language {
        None => out.push_str(&escaped(code)),
        Some(language) => match markdown::code_tokens(code, language) {
            None => out.push_str(&format!("<span class=\"code-plain\">{}</span>", escaped(code))),
            Some(tokens) => {
                for (token, piece) in tokens {
                    match token_class(token) {
                        Some(class) => out.push_str(&format!("<span class=\"{class}\">{}</span>", escaped(piece))),
                        None => out.push_str(&escaped(piece)),
                    }
                }
            }
        },
    }
    out.push_str("</code></pre>\n");
    out
}

/// The class a piece of code is colored by; white space takes none.
fn token_class(token: Token) -> Option<&'static str> {
    match token {
        Token::Comment => Some("tok-comment"),
        Token::Function => Some("tok-function"),
        Token::Keyword => Some("tok-keyword"),
        Token::Literal => Some("tok-literal"),
        Token::String => Some("tok-string"),
        Token::Punctuation => Some("tok-punctuation"),
        Token::Whitespace => None,
    }
}

/// Writes each block quote that is a GitHub alert as that alert: its opening
/// taken out as [`markdown::alert_of`] takes it, under a title with its
/// icon and name, in its color.
fn mark_alerts(events: &mut Vec<Event<'_>>) {
    let mut at = 0;
    while at < events.len() {
        if matches!(events[at], Event::Start(Tag::BlockQuote(_))) {
            let end = quote_end(events, at);
            let mut inside: Vec<Event<'_>> = events[at + 1..end].to_vec();
            if let Some(alert) = markdown::alert_of(&mut inside, |e| e) {
                let (class, icon) = alert_look(alert);
                let tail = events.split_off(end + 1);
                events.truncate(at);
                events.push(tag(format!("<div class=\"alert {class}\"><p class=\"alert-title\">{icon} {}</p>\n", alert.name())));
                events.extend(inside);
                events.push(tag(String::from("</div>\n")));
                events.extend(tail);
            }
        }
        at += 1;
    }
}

/// Where the block quote starting at `start` ends: its own end, past the
/// quotes inside it; the last event when it has none.
fn quote_end(events: &[Event<'_>], start: usize) -> usize {
    let mut open = 0;
    for (i, event) in events.iter().enumerate().skip(start) {
        match event {
            Event::Start(Tag::BlockQuote(_)) => open += 1,
            Event::End(TagEnd::BlockQuote(_)) => {
                open -= 1;
                if open == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    events.len() - 1
}

/// An alert's class and icon, the egui renderer's icon and a class for its
/// color.
fn alert_look(alert: markdown::Alert) -> (&'static str, &'static str) {
    match alert {
        markdown::Alert::Note => ("alert-note", "❕"),
        markdown::Alert::Tip => ("alert-tip", "💡"),
        markdown::Alert::Important => ("alert-important", "💬"),
        markdown::Alert::Warning => ("alert-warning", "⚠"),
        markdown::Alert::Caution => ("alert-caution", "🔴"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(address: &str) -> Option<String> {
        (address == "shots/a.png").then(|| String::from("picture/picture-7.png"))
    }

    #[test]
    fn a_document_draws_its_few_tags_and_writes_every_other_as_text() {
        let html = html("Hi <b class=\"x\">bold</b> <script>alert(1)</script> <span onclick=\"x()\">s</span>", &served);
        assert!(html.contains("<b>bold</b>"), "{html}");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"), "{html}");
        assert!(html.contains("&lt;span onclick=\"x()\"&gt;s&lt;/span&gt;"), "{html}");
        assert!(!html.contains("<script") && !html.contains("<span"), "{html}");
    }

    #[test]
    fn links_and_headings_carry_their_addresses_in_data_the_page_does_not_follow() {
        let html = html("# Top {#top}\n\n[up](#top) [site](https://example.com/?a=1&b=\"2\") [bad](javascript:alert(1))", &served);
        assert!(html.contains("<h1 data-heading=\"top\">Top</h1>"), "{html}");
        assert!(html.contains("<a href=\"#\" data-link=\"#top\" title=\"#top\">up</a>"), "{html}");
        assert!(html.contains("data-link=\"https://example.com/?a=1&amp;b=&quot;2&quot;\""), "{html}");
        assert!(html.contains("<a href=\"#\" data-link=\"javascript:alert(1)\""), "{html}");
        assert!(!html.contains("href=\"javascript") && !html.contains("href=\"https"), "{html}");
    }

    #[test]
    fn an_image_loads_its_served_picture_or_its_https_address_or_shows_its_alt_text() {
        let html = html("![a red *box*](shots/a.png) ![logo](https://example.com/l.svg) ![gone](//server/share/x.png) ![file](other.png)", &served);
        assert!(html.contains("<img src=\"picture/picture-7.png\" alt=\"a red box\" title=\"a red box\" data-address=\"shots/a.png\">"), "{html}");
        assert!(html.contains("<img src=\"https://example.com/l.svg\" alt=\"logo\""), "{html}");
        assert!(html.contains(" gone ") && html.contains(" file"), "{html}");
        assert_eq!(html.matches("<img").count(), 2, "{html}");
    }

    #[test]
    fn code_is_colored_by_egui_extras_rules_and_alerts_are_drawn_as_alerts() {
        let html = html("```rust\nlet s = \"<b>\";\n```\n\n```powershell\nGet-Item\n```\n\n```\nplain\n```\n\n> [!NOTE]\n> Read this.\n\n> plain", &served);
        assert!(html.contains("<pre class=\"code\"><code><span class=\"tok-keyword\">let</span> <span class=\"tok-literal\">s</span> <span class=\"tok-punctuation\">=</span> <span class=\"tok-string\">&quot;&lt;b&gt;&quot;</span><span class=\"tok-punctuation\">;</span></code></pre>"), "{html}");
        assert!(html.contains("<code><span class=\"code-plain\">Get-Item</span></code>"), "{html}");
        assert!(html.contains("<code>plain</code>"), "{html}");
        assert!(html.contains("<div class=\"alert alert-note\"><p class=\"alert-title\">❕ Note</p>\n<p>Read this.</p>\n</div>"), "{html}");
        assert!(html.contains("<blockquote>\n<p>plain</p>\n</blockquote>"), "{html}");
    }
}
