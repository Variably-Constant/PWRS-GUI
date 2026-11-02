//! What the session and every renderer read a Markdown document as: the
//! CommonMark extensions it takes, where each of its images comes from,
//! and which of its raw HTML is drawn.

pub use pulldown_cmark;

/// The extensions a document is read with: tables, task lists,
/// strikethrough, footnotes and definition lists, as egui_commonmark reads
/// a document, and heading attributes, which give a heading the `{#id}` a
/// link scrolls to.
pub fn options() -> pulldown_cmark::Options {
    use pulldown_cmark::Options;
    Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES | Options::ENABLE_DEFINITION_LIST | Options::ENABLE_HEADING_ATTRIBUTES
}

/// The size in points of a heading of `level`, 1 to 6, from `body`, the
/// size of body text, to `heading`, a first-level heading's, as
/// egui_commonmark sizes headings.
pub fn heading_size(level: u8, body: f32, heading: f32) -> f32 {
    let share = match level {
        0 | 1 => return heading,
        2 => 0.835,
        3 => 0.668,
        4 => 0.501,
        5 => 0.334,
        _ => 0.167,
    };
    body + (heading - body) * share
}

/// Whether a heading of `level` is drawn in the strong text color, as
/// egui_commonmark draws the first four levels; the rest take body text's.
pub fn heading_is_strong(level: u8) -> bool {
    level <= 4
}

/// The addresses of a document's images, each once, in the order they
/// first appear.
pub fn image_addresses(text: &str) -> Vec<String> {
    addresses(text, true)
}

/// The addresses of a document's links, each once, in the order they first
/// appear; a link to a heading's `{#id}` is `#id`.
pub fn link_addresses(text: &str) -> Vec<String> {
    addresses(text, false)
}

/// The addresses of a document's images, or with `images` false of its
/// links, each once, in the order they first appear.
fn addresses(text: &str, images: bool) -> Vec<String> {
    use pulldown_cmark::{Event, Tag};
    let mut out: Vec<String> = Vec::new();
    for event in pulldown_cmark::Parser::new_ext(text, options()) {
        let address = match event {
            Event::Start(Tag::Image { dest_url, .. }) if images => dest_url,
            Event::Start(Tag::Link { dest_url, .. }) if !images => dest_url,
            _ => continue,
        };
        if !out.iter().any(|a| a.as_str() == address.as_ref()) {
            out.push(address.into_string());
        }
    }
    out
}

/// Where an image's address says its picture comes from.
#[derive(Debug, PartialEq)]
pub enum Source<'a> {
    /// The picture in the address itself, a `data:` URL: its bytes,
    /// or why they could not be read.
    Data(Result<Vec<u8>, String>),
    /// An https address, which a renderer fetches.
    Web(&'a str),
    /// A file, by its path: relative to the document's folder unless it is
    /// absolute. A `file:` URL with no host gives one.
    File(String),
    /// No source this widget reads from, and why.
    Refused(&'static str),
}

/// Where the image at `address` comes from. A UNC path, a device path and a
/// `file:` URL naming a host are refused, since opening one makes Windows
/// send the user's credentials to the host it names, and so is every other
/// scheme, http among them. A path's percent escapes are decoded, as
/// a web page's would be.
pub fn source(address: &str) -> Source<'_> {
    match scheme(address) {
        Some(s) if s.eq_ignore_ascii_case("data") => Source::Data(data_url(&address[s.len() + 1..])),
        Some(s) if s.eq_ignore_ascii_case("https") => Source::Web(address),
        Some(s) if s.eq_ignore_ascii_case("file") => file_url(&address[s.len() + 1..]),
        Some(_) => Source::Refused("an image's address is https, data, a file or a path; this one names another scheme"),
        None => {
            let path = percent_decoded(address);
            if names_a_share(&path) { Source::Refused(SHARED) } else { Source::File(path) }
        }
    }
}

/// Why a UNC or device path is refused.
pub const SHARED: &str = "a UNC or device path names a share or a device, and opening one makes Windows send the user's credentials to its host";

/// The scheme an address opens with: letters, digits, `+`, `-` and `.`
/// after a letter, two characters or more, before a colon, so that a
/// Windows drive is no scheme.
fn scheme(address: &str) -> Option<&str> {
    let colon = address.find(':')?;
    let s = &address[..colon];
    let mut chars = s.chars();
    let first = chars.next()?;
    let ok = first.is_ascii_alphabetic() && s.len() >= 2 && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    ok.then_some(s)
}

/// Whether `path` is a UNC or device path: one that opens with two slashes
/// of either kind. A relative path joined to a document's folder on a share
/// is one too.
pub fn names_a_share(path: &str) -> bool {
    let mut opening = path.chars().take(2);
    matches!((opening.next(), opening.next()), (Some('/' | '\\'), Some('/' | '\\')))
}

/// The file a `file:` URL's `rest`, what follows the colon, names: a host
/// is refused, and a drive's path loses the slash before it.
fn file_url(rest: &str) -> Source<'_> {
    let path = match rest.strip_prefix("//") {
        Some(after) if after.starts_with('/') => after,
        Some(_) => return Source::Refused("a file: URL that names a host names a share, and opening it makes Windows send the user's credentials to that host"),
        None => rest,
    };
    let path = percent_decoded(path);
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    let path = if drive { path[1..].to_string() } else { path };
    if names_a_share(&path) { Source::Refused(SHARED) } else { Source::File(path) }
}

/// The bytes of a `data:` URL's `rest`, what follows the colon:
/// `[media type][;base64],data`, the data in base64 or percent escaped.
fn data_url(rest: &str) -> Result<Vec<u8>, String> {
    let comma = rest.find(',').ok_or_else(|| String::from("a data: URL has a comma before its data"))?;
    let (head, data) = (&rest[..comma], &rest[comma + 1..]);
    if head.to_ascii_lowercase().ends_with(";base64") {
        crate::base64_bytes(data).map_err(|other| format!("a data: URL's base64 contains {other:?}"))
    } else {
        Ok(percent_decoded(data).into_bytes())
    }
}

/// `text` with its percent escapes decoded; an escape that is not two hex
/// digits, or bytes that are not UTF-8, stay as they were written.
fn percent_decoded(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    match String::from_utf8(out) {
        Ok(decoded) => decoded,
        Err(_not_utf8) => text.to_string(),
    }
}

/// The raw HTML tags a document draws; any other tag shows as the text it
/// is written as.
pub const DRAWN_TAGS: [&str; 9] = ["b", "i", "em", "strong", "br", "sup", "sub", "code", "kbd"];

/// One piece of a document's raw HTML.
#[derive(Debug, PartialEq)]
pub enum Html<'a> {
    /// One of the drawn tags opening, by its lower-case name, its
    /// attributes dropped; a `br` opens only.
    Open(&'static str),
    /// One of the drawn tags closing.
    Close(&'static str),
    /// Anything else, shown as it is written.
    Text(&'a str),
}

/// The pieces of the raw HTML `html`: each of the drawn tags, and between
/// them everything else as text.
pub fn html_pieces(html: &str) -> Vec<Html<'_>> {
    let mut out = Vec::new();
    let (mut text_from, mut at) = (0, 0);
    while let Some(open) = html[at..].find('<') {
        let start = at + open;
        let Some(length) = html[start..].find('>') else {
            break;
        };
        let end = start + length + 1;
        if let Some(piece) = drawn_tag(&html[start..end]) {
            if start > text_from {
                out.push(Html::Text(&html[text_from..start]));
            }
            out.push(piece);
            text_from = end;
        }
        at = end;
    }
    if text_from < html.len() {
        out.push(Html::Text(&html[text_from..]));
    }
    out
}

/// The drawn tag `tag` is, written `<name ...>`, `</name>` or `<name/>`;
/// none for any other.
fn drawn_tag(tag: &str) -> Option<Html<'static>> {
    let inner = tag.strip_prefix('<')?.strip_suffix('>')?;
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner),
    };
    let inner = inner.strip_suffix('/').unwrap_or(inner);
    let name_end = inner.find(|c: char| c.is_ascii_whitespace()).unwrap_or(inner.len());
    let (name, rest) = inner.split_at(name_end);
    if closing && !rest.trim().is_empty() {
        return None;
    }
    let found = DRAWN_TAGS.iter().find(|t| t.eq_ignore_ascii_case(name))?;
    Some(if closing { Html::Close(found) } else { Html::Open(found) })
}

/// A GitHub alert: a block quote that opens with `[!NOTE]`, `[!TIP]`,
/// `[!IMPORTANT]`, `[!WARNING]` or `[!CAUTION]`, written in any case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl Alert {
    /// The alert's name, as its title shows it.
    pub fn name(self) -> &'static str {
        match self {
            Alert::Note => "Note",
            Alert::Tip => "Tip",
            Alert::Important => "Important",
            Alert::Warning => "Warning",
            Alert::Caution => "Caution",
        }
    }
}

/// The alert a block quote opens with, read from its events after its
/// start, `event` reading each item's, with that opening taken out of them:
/// with the paragraph it ends when a line of its own, or else with the line
/// break after it. None for a plain quote, whose events stay as they are.
pub fn alert_of<'e, T>(events: &mut Vec<T>, event: impl Fn(&T) -> &pulldown_cmark::Event<'e>) -> Option<Alert> {
    use pulldown_cmark::Event;
    let mut written = String::new();
    let mut ends_at = 0;
    let mut own_paragraph = false;
    for (i, item) in events.iter().enumerate() {
        match event(item) {
            Event::End(_) => {
                ends_at = i;
                own_paragraph = true;
                break;
            }
            Event::SoftBreak | Event::HardBreak => {
                ends_at = i;
                break;
            }
            Event::Text(text) => written.push_str(text),
            _ => {}
        }
    }
    let alert = match written.to_uppercase().as_str() {
        "[!NOTE]" => Alert::Note,
        "[!TIP]" => Alert::Tip,
        "[!IMPORTANT]" => Alert::Important,
        "[!WARNING]" => Alert::Warning,
        "[!CAUTION]" => Alert::Caution,
        _ => return None,
    };
    if own_paragraph {
        events.drain(..=ends_at);
    } else {
        events.drain(1..=ends_at);
    }
    Some(alert)
}

/// What a piece of a fenced code block is, as egui_extras's own highlighter
/// tells the pieces apart; each renderer colors each kind its own way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Comment,
    /// A word directly followed by `(`, as `f` in `f(x)`.
    Function,
    Keyword,
    Literal,
    String,
    Punctuation,
    Whitespace,
}

/// How egui_extras's highlighter reads one language.
struct Language {
    /// `// comment`
    double_slash_comments: bool,
    /// `/* comment */`
    block_comments: bool,
    /// `-- comment`
    double_dash_comments: bool,
    /// `# comment`
    hash_comments: bool,
    /// What strings start and end with, longer ones first, so `"""` is not
    /// read as an empty `""`.
    string_delimiters: &'static [&'static str],
    /// Whether a word is a keyword without regard to case; the keywords
    /// are then lower case.
    case_insensitive_keywords: bool,
    keywords: &'static [&'static str],
}

impl Language {
    /// The language `name` names, as egui_extras reads the names.
    fn named(name: &str) -> Option<Language> {
        let (line, block, dash, hash, string_delimiters, case_insensitive_keywords, keywords): (bool, bool, bool, bool, &'static [&'static str], bool, &'static [&'static str]) = match name.to_lowercase().as_str() {
            "c" | "h" | "hpp" | "cpp" | "c++" => (true, true, false, false, &["\"", "'"], false, CPP_KEYWORDS),
            "js" | "javascript" | "jsx" | "mjs" | "ts" | "typescript" | "tsx" => (true, true, false, false, &["\"", "'", "`"], false, JAVASCRIPT_KEYWORDS),
            "json" | "jsonc" | "json5" => (true, true, false, false, &["\""], false, &["false", "null", "true"]),
            "py" | "python" => (false, false, false, true, &["\"\"\"", "'''", "\"", "'"], false, PYTHON_KEYWORDS),
            "rs" | "rust" => (true, true, false, false, &["\""], false, RUST_KEYWORDS),
            "bash" | "sh" | "shell" | "zsh" => (false, false, false, true, &["\"", "'"], false, SHELL_KEYWORDS),
            "sql" => (false, false, true, false, &["\"", "'"], true, SQL_KEYWORDS),
            "toml" => (false, false, false, true, &["\"\"\"", "'''", "\"", "'"], false, &[]),
            "yaml" | "yml" => (false, false, false, true, &["\"", "'"], false, &["false", "null", "true"]),
            _ => return None,
        };
        Some(Language { double_slash_comments: line, block_comments: block, double_dash_comments: dash, hash_comments: hash, string_delimiters, case_insensitive_keywords, keywords })
    }

    fn is_keyword(&self, word: &str) -> bool {
        if self.case_insensitive_keywords {
            self.keywords.contains(&word.to_ascii_lowercase().as_str())
        } else {
            self.keywords.contains(&word)
        }
    }
}

/// Whether `c` can be part of a word, a name or a keyword.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// How many bytes the string at the start of `text` takes, both
/// delimiters included: a backslash escapes the character after it, and a
/// string never closed ends with its line.
fn string_len(text: &str, delimiter: &str) -> usize {
    let body = &text[delimiter.len()..];
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            chars.next();
        } else if body[i..].starts_with(delimiter) {
            return delimiter.len() + i + delimiter.len();
        }
    }
    text.find('\n').unwrap_or(text.len())
}

/// The pieces of a fenced code block in `language`, told apart by the rules
/// of egui_extras's own highlighter, which the egui renderer draws code
/// with: for C and C++, JavaScript and TypeScript, JSON, Python, Rust,
/// shell, SQL, TOML and YAML. None for any other language, which shows
/// plain.
pub fn code_tokens<'a>(text: &'a str, language: &str) -> Option<Vec<(Token, &'a str)>> {
    let language = Language::named(language)?;
    let mut out = Vec::new();
    let mut text = text;
    while !text.is_empty() {
        let (token, end) = if language.double_slash_comments && text.starts_with("//") || language.double_dash_comments && text.starts_with("--") || language.hash_comments && text.starts_with('#') {
            (Token::Comment, text.find('\n').unwrap_or(text.len()))
        } else if language.block_comments && text.starts_with("/*") {
            (Token::Comment, text[2..].find("*/").map_or(text.len(), |i| i + 4))
        } else if let Some(delimiter) = language.string_delimiters.iter().find(|d| text.starts_with(**d)) {
            (Token::String, string_len(text, delimiter))
        } else if text.starts_with(is_word_char) {
            let end = text[1..].find(|c: char| !is_word_char(c)).map_or(text.len(), |i| i + 1);
            let word = &text[..end];
            let token = if language.is_keyword(word) {
                Token::Keyword
            } else if text[end..].starts_with('(') && !word.starts_with(|c: char| c.is_ascii_digit()) {
                Token::Function
            } else {
                Token::Literal
            };
            (token, end)
        } else if text.starts_with(|c: char| c.is_ascii_whitespace()) {
            (Token::Whitespace, text[1..].find(|c: char| !c.is_ascii_whitespace()).map_or(text.len(), |i| i + 1))
        } else {
            let mut chars = text.char_indices();
            chars.next();
            (Token::Punctuation, chars.next().map_or(text.len(), |(at, _)| at))
        };
        out.push((token, &text[..end]));
        text = &text[end..];
    }
    Some(out)
}

/// The words egui_extras's highlighter takes for C and C++ keywords.
const CPP_KEYWORDS: &[&str] = &[
    "alignas", "alignof", "and_eq", "and", "asm", "atomic_cancel", "atomic_commit", "atomic_noexcept", "auto", "bitand", "bitor", "bool", "break", "case", "catch", "char", "char16_t",
    "char32_t", "char8_t", "class", "co_await", "co_return", "co_yield", "compl", "concept", "const_cast", "const", "consteval", "constexpr", "constinit", "continue", "decltype",
    "default", "delete", "do", "double", "dynamic_cast", "else", "enum", "explicit", "export", "extern", "false", "float", "for", "friend", "goto", "if", "inline", "int", "long",
    "mutable", "namespace", "new", "noexcept", "not_eq", "not", "nullptr", "operator", "or_eq", "or", "private", "protected", "public", "reflexpr", "register", "reinterpret_cast",
    "requires", "return", "short", "signed", "sizeof", "static_assert", "static_cast", "static", "struct", "switch", "synchronized", "template", "this", "thread_local", "throw",
    "true", "try", "typedef", "typeid", "typename", "union", "unsigned", "using", "virtual", "void", "volatile", "wchar_t", "while", "xor_eq", "xor",
];

/// The words egui_extras's highlighter takes for JavaScript and TypeScript
/// keywords.
const JAVASCRIPT_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "declare", "default", "delete", "do", "else", "enum", "export",
    "extends", "false", "finally", "for", "from", "function", "if", "implements", "import", "in", "instanceof", "interface", "keyof", "let", "namespace", "new", "null", "of",
    "private", "protected", "public", "readonly", "return", "static", "super", "switch", "this", "throw", "true", "try", "type", "typeof", "undefined", "var", "void", "while",
    "with", "yield",
];

/// The words egui_extras's highlighter takes for Python keywords.
const PYTHON_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while", "with", "yield",
];

/// The words egui_extras's highlighter takes for Rust keywords.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
];

/// The words egui_extras's highlighter takes for shell keywords.
const SHELL_KEYWORDS: &[&str] = &["case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in", "local", "return", "then", "until", "while"];

/// The words egui_extras's highlighter takes for SQL keywords, in any case.
const SQL_KEYWORDS: &[&str] = &[
    "add", "all", "alter", "and", "any", "as", "asc", "between", "by", "case", "cast", "check", "column", "constraint", "create", "cross", "database", "default", "delete", "desc",
    "distinct", "drop", "else", "end", "exists", "false", "foreign", "from", "full", "group", "having", "if", "in", "index", "inner", "insert", "intersect", "into", "is", "join",
    "key", "left", "like", "limit", "not", "null", "offset", "on", "or", "order", "outer", "primary", "references", "replace", "right", "select", "set", "table", "then", "true",
    "union", "unique", "update", "using", "values", "view", "when", "where", "with",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_images_address_names_a_file_the_web_or_its_own_data_and_never_a_share() {
        assert_eq!(source("shots/run 1.png"), Source::File(String::from("shots/run 1.png")));
        assert_eq!(source("shots/run%201.png"), Source::File(String::from("shots/run 1.png")));
        assert_eq!(source(r"C:\shots\a.png"), Source::File(String::from(r"C:\shots\a.png")));
        assert_eq!(source("/home/me/a.png"), Source::File(String::from("/home/me/a.png")));
        assert_eq!(source("file:///C:/shots/a%20b.png"), Source::File(String::from("C:/shots/a b.png")));
        assert_eq!(source("file:///home/me/a.png"), Source::File(String::from("/home/me/a.png")));
        assert_eq!(source("https://example.com/a.png"), Source::Web("https://example.com/a.png"));
        assert_eq!(source("data:image/png;base64,iVBO Rw=="), Source::Data(Ok(vec![0x89, 0x50, 0x4E, 0x47])));
        assert_eq!(source("data:image/svg+xml,%3Csvg%3E"), Source::Data(Ok(b"<svg>".to_vec())));
        for refused in [r"\\server\share\a.png", "//server/share/a.png", r"\\?\C:\a.png", "file://server/share/a.png", "file:////server/share/a.png", "http://example.com/a.png", "javascript:alert(1)"] {
            assert!(matches!(source(refused), Source::Refused(_)), "{refused} is refused");
        }
    }

    #[test]
    fn a_documents_images_and_links_are_named_once_each_in_the_order_they_appear() {
        let text = "![a](one.png) text ![b](two.png)\n\n| x |\n|---|\n| ![c](one.png) |\n\n[not an image](three.png) [up](#top) <https://example.com/> [again](three.png)";
        assert_eq!(image_addresses(text), ["one.png", "two.png"]);
        assert_eq!(link_addresses(text), ["three.png", "#top", "https://example.com/"]);
    }

    #[test]
    fn raw_html_draws_its_few_tags_and_shows_every_other_as_text() {
        assert_eq!(
            html_pieces("<b>bold</B><br/><Sup class=\"x\">2</sup><span>no</span>"),
            [Html::Open("b"), Html::Text("bold"), Html::Close("b"), Html::Open("br"), Html::Open("sup"), Html::Text("2"), Html::Close("sup"), Html::Text("<span>no</span>")]
        );
        assert_eq!(html_pieces("<script>alert(1)</script>"), [Html::Text("<script>alert(1)</script>")]);
        assert_eq!(html_pieces("<!-- a note --> 1 < 2"), [Html::Text("<!-- a note --> 1 < 2")]);
        assert_eq!(html_pieces("</kbd extra>"), [Html::Text("</kbd extra>")]);
    }

    #[test]
    fn a_block_quote_opening_with_an_alert_is_that_alert_without_its_opening() {
        use pulldown_cmark::{Event, Tag};
        let inside = |text: &str| -> Vec<Event<'static>> { pulldown_cmark::Parser::new_ext(text, options()).skip(1).map(Event::into_static).collect() };
        let mut events = inside("> [!warning]\n> Back up first.");
        assert_eq!(alert_of(&mut events, |e| e), Some(Alert::Warning));
        assert!(matches!(&events[..2], [Event::Start(Tag::Paragraph), Event::Text(t)] if t.as_ref() == "Back up first."), "the opening and its line break go: {events:?}");
        let mut apart = inside("> [!TIP]\n>\n> Use a branch.");
        assert_eq!(alert_of(&mut apart, |e| e).map(Alert::name), Some("Tip"));
        assert!(matches!(&apart[..2], [Event::Start(Tag::Paragraph), Event::Text(t)] if t.as_ref() == "Use a branch."), "an opening on a paragraph of its own goes with it: {apart:?}");
        let mut plain = inside("> just a quote");
        let before = plain.clone();
        assert_eq!(alert_of(&mut plain, |e| e), None);
        assert_eq!(plain, before);
    }

    #[test]
    fn code_is_told_apart_by_egui_extras_rules_for_the_languages_it_knows() {
        use Token::*;
        assert_eq!(
            code_tokens("let s = \"hi\"; // why", "Rust"),
            Some(vec![(Keyword, "let"), (Whitespace, " "), (Literal, "s"), (Whitespace, " "), (Punctuation, "="), (Whitespace, " "), (String, "\"hi\""), (Punctuation, ";"), (Whitespace, " "), (Comment, "// why")])
        );
        assert_eq!(code_tokens("# note\nx", "toml"), Some(vec![(Comment, "# note"), (Whitespace, "\n"), (Literal, "x")]));
        assert_eq!(code_tokens("é", "py"), Some(vec![(Punctuation, "é")]));
        assert_eq!(code_tokens("my_fn(2x)", "rs"), Some(vec![(Function, "my_fn"), (Punctuation, "("), (Literal, "2x"), (Punctuation, ")")]), "a word before ( is a function's name");
        assert_eq!(code_tokens("\"a \\\" b\" /* c */", "c"), Some(vec![(String, "\"a \\\" b\""), (Whitespace, " "), (Comment, "/* c */")]), "an escaped quote stays in its string");
        assert_eq!(code_tokens("Select 1 -- n", "SQL"), Some(vec![(Keyword, "Select"), (Whitespace, " "), (Literal, "1"), (Whitespace, " "), (Comment, "-- n")]), "SQL's keywords are in any case");
        assert_eq!(code_tokens("'''x\ny'''", "python"), Some(vec![(String, "'''x\ny'''")]), "a triple-quoted string runs over lines");
        assert_eq!(code_tokens("\"open\nnext", "json"), Some(vec![(String, "\"open"), (Whitespace, "\n"), (Literal, "next")]), "a string never closed ends with its line");
        assert_eq!(code_tokens("Get-Item", "powershell"), None);
    }
}
