//! Draws a Markdown widget's document in egui. The renderer is egui_commonmark
//! 0.26's (MIT, Copyright (c) 2022-2025 Erlend Walstad, whose notice is
//! NOTICE-egui_commonmark beside this crate's manifest): it reads
//! pulldown-cmark's events and lays them out in rows that wrap. Here the
//! document is read as [`proto::markdown`] says, raw HTML draws the tags
//! [`proto::markdown::DRAWN_TAGS`] names and shows every other as text,
//! superscript and subscript are raised and lowered, a link is reported as
//! clicked rather than opened, and an image is a picture the session read
//! for its address, an https picture the helper fetches, or its alt text.

use crate::egui;
use crate::proto::markdown::{self, Html, pulldown_cmark};
use egui::text::LayoutJob;
use egui::{Id, NumExt, RichText, Sense, TextStyle, Ui, Vec2};
use pulldown_cmark::{CowStr, Event, HeadingLevel, Tag, TagEnd};
use std::iter::Peekable;
use std::ops::Range;
use std::sync::Arc;

/// Where an image's picture comes from, by its address.
#[derive(Clone)]
pub enum Picture {
    /// Bytes the session read, loaded under `uri` in egui's caches, and the
    /// picture's own size as they give it; none where they give none, so
    /// the image shows its alt text when it is drawn.
    Bytes { uri: String, bytes: Arc<[u8]>, size: Option<Vec2> },
    /// An https picture, which the helper's loader fetches under this
    /// address in egui's caches.
    Web(String),
    /// None, so the image shows its alt text.
    Absent,
}

impl Picture {
    /// The address the picture is loaded under in egui's caches; none for
    /// an image without one.
    pub fn uri(&self) -> Option<&str> {
        match self {
            Picture::Bytes { uri, .. } | Picture::Web(uri) => Some(uri),
            Picture::Absent => None,
        }
    }
}

/// What a document stores from one frame to the next: the heading a link
/// requested a scroll to, by its id.
#[derive(Default)]
pub struct Kept {
    scroll_to: Option<String>,
}

impl Kept {
    /// Has the next frame scroll to the heading whose id is `id`, as a
    /// link to `#id` does.
    pub fn scroll_to(&mut self, id: &str) {
        self.scroll_to = Some(id.to_string());
    }
}

/// What a frame of a document came to: the addresses of the links clicked
/// in it, and the images that showed their alt text because their picture
/// could not be drawn, each with why.
pub struct Shown {
    pub clicked: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Draws `text` in `ui` as a document, its images found through `picture`.
pub fn show(ui: &mut Ui, id: Id, text: &str, picture: &dyn Fn(&str) -> Picture, kept: &mut Kept) -> Shown {
    let max_width = ui.available_width();
    let layout = egui::Layout::left_to_right(egui::Align::BOTTOM).with_main_wrap(true);
    let mut renderer = Renderer::new(id, picture, kept.scroll_to.take());
    ui.allocate_ui_with_layout(egui::vec2(max_width, 0.0), layout, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let height = ui.text_style_height(&TextStyle::Body);
        ui.set_row_height(height);
        let mut events = pulldown_cmark::Parser::new_ext(text, markdown::options()).into_offset_iter().enumerate().peekable();
        while let Some((index, (e, span))) = events.next() {
            if events.peek().is_none() {
                renderer.line.should_end_newline_forced = false;
            }
            renderer.process_event(ui, &mut events, e, span, max_width);
            if index == 0 {
                renderer.line.should_not_start_newline_forced = false;
            }
        }
    });
    kept.scroll_to = renderer.scroll_requested.take().or(renderer.scroll_to.take());
    Shown { clicked: renderer.clicked, failed: renderer.failed }
}

type EventIteratorItem<'e> = (usize, (Event<'e>, Range<usize>));

/// Newlines: each element puts one before itself where it may, and ends
/// its own line.
struct Newline {
    /// No newline before the first element.
    should_not_start_newline_forced: bool,
    /// Whether an element puts a newline before itself.
    should_start_newline: bool,
    /// Whether an element ends its own line, which blocks inside a list
    /// item do not.
    should_end_newline: bool,
    /// False only for the last element.
    should_end_newline_forced: bool,
}

impl Default for Newline {
    fn default() -> Self {
        Self { should_not_start_newline_forced: true, should_start_newline: true, should_end_newline: true, should_end_newline_forced: true }
    }
}

impl Newline {
    fn can_insert_end(&self) -> bool {
        self.should_end_newline && self.should_end_newline_forced
    }

    fn can_insert_start(&self) -> bool {
        self.should_start_newline && !self.should_not_start_newline_forced
    }

    fn try_insert_start(&self, ui: &mut Ui) {
        if self.can_insert_start() {
            newline(ui);
        }
    }

    fn try_insert_end(&self, ui: &mut Ui) {
        if self.can_insert_end() {
            newline(ui);
        }
    }
}

/// How the text being drawn looks: each counts the tags and Markdown
/// spans open, so that nested ones close in turn.
#[derive(Default, Clone)]
struct Style {
    heading: Option<u8>,
    strong: u32,
    emphasis: u32,
    strikethrough: u32,
    quote: bool,
    code: u32,
    superscript: u32,
    subscript: u32,
}

impl Style {
    fn rich(&self, ui: &Ui, text: &str) -> RichText {
        let mut rich = RichText::new(text);
        if let Some(level) = self.heading {
            let heading = ui.style().text_styles.get(&TextStyle::Heading).map_or(32.0, |d| d.size);
            let body = ui.style().text_styles.get(&TextStyle::Body).map_or(14.0, |d| d.size);
            rich = rich.size(markdown::heading_size(level + 1, body, heading));
            if markdown::heading_is_strong(level + 1) {
                rich = rich.strong();
            }
        }
        if self.quote {
            rich = rich.weak();
        }
        if self.strong > 0 {
            rich = rich.strong();
        }
        if self.emphasis > 0 {
            rich = rich.italics();
        }
        if self.strikethrough > 0 {
            rich = rich.strikethrough();
        }
        if self.code > 0 {
            rich = rich.code();
        }
        if self.superscript > 0 {
            rich = rich.raised();
        }
        rich
    }

    /// `text` laid out as it looks: subscript lowered and smaller, which
    /// a rich text cannot say, everything else as [`Style::rich`] gives.
    fn job(&self, ui: &Ui, text: &str) -> LayoutJob {
        let mut job = LayoutJob::default();
        self.rich(ui, text).append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::LEFT);
        if self.subscript > 0 {
            for section in &mut job.sections {
                section.format.valign = egui::Align::BOTTOM;
                section.format.font_id.size *= 0.75;
            }
        }
        job
    }
}

/// A link being read: its address and its text.
struct Link {
    destination: String,
    text: Vec<LayoutJob>,
}

/// An image being read: its address and its alt text.
struct ImageLink {
    address: String,
    alt: Vec<LayoutJob>,
}

/// A code block being read: its language, when fenced with one, and its
/// text.
struct CodeBlock {
    lang: Option<String>,
    content: String,
}

#[derive(Default)]
struct DefinitionList {
    is_first_item: bool,
    is_def_list_def: bool,
}

struct ListLevel {
    current_number: Option<u64>,
}

#[derive(Default)]
struct List {
    items: Vec<ListLevel>,
    has_list_begun: bool,
}

impl List {
    fn start_level_with_number(&mut self, start_number: u64) {
        self.items.push(ListLevel { current_number: Some(start_number) });
    }

    fn start_level_without_number(&mut self) {
        self.items.push(ListLevel { current_number: None });
    }

    fn is_inside_a_list(&self) -> bool {
        !self.items.is_empty()
    }

    fn is_last_level(&self) -> bool {
        self.items.len() == 1
    }

    fn start_item(&mut self, ui: &mut Ui) {
        if self.has_list_begun {
            newline(ui);
        } else {
            self.has_list_begun = true;
        }
        let len = self.items.len();
        if let Some(item) = self.items.last_mut() {
            ui.label(" ".repeat((len - 1) * INDENTATION));
            match &mut item.current_number {
                Some(number) => {
                    number_point(ui, &number.to_string());
                    *number += 1;
                }
                None if len > 1 => bullet_point_hollow(ui),
                None => bullet_point(ui),
            }
        }
        ui.add_space(4.0);
    }

    fn end_level(&mut self, ui: &mut Ui, insert_newline: bool) {
        self.items.pop();
        if self.items.is_empty() && insert_newline {
            newline(ui);
        }
    }
}

/// The spaces a list's nested level is indented by.
const INDENTATION: usize = 4;

struct Renderer<'p> {
    id: Id,
    curr_table: usize,
    text_style: Style,
    list: List,
    link: Option<Link>,
    image: Option<ImageLink>,
    line: Newline,
    code_block: Option<CodeBlock>,
    is_list_item: bool,
    def_list: DefinitionList,
    is_table: bool,
    is_blockquote: bool,
    picture: &'p dyn Fn(&str) -> Picture,
    /// The heading this frame scrolls to, requested by a link last frame.
    scroll_to: Option<String>,
    /// A heading a link clicked in this frame requests a scroll to.
    scroll_requested: Option<String>,
    clicked: Vec<String>,
    failed: Vec<(String, String)>,
}

impl<'p> Renderer<'p> {
    fn new(id: Id, picture: &'p dyn Fn(&str) -> Picture, scroll_to: Option<String>) -> Self {
        Self {
            id,
            curr_table: 0,
            text_style: Style::default(),
            list: List::default(),
            link: None,
            image: None,
            line: Newline::default(),
            code_block: None,
            is_list_item: false,
            def_list: DefinitionList::default(),
            is_table: false,
            is_blockquote: false,
            picture,
            scroll_to,
            scroll_requested: None,
            clicked: Vec::new(),
            failed: Vec::new(),
        }
    }

    fn process_event<'e>(&mut self, ui: &mut Ui, events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>, event: Event, span: Range<usize>, max_width: f32) {
        self.event(ui, event, span, max_width);
        self.def_list_def_wrapping(events, max_width, ui);
        self.item_list_wrapping(events, max_width, ui);
        self.table(events, ui, max_width);
        self.blockquote(events, max_width, ui);
    }

    fn def_list_def_wrapping<'e>(&mut self, events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>, max_width: f32, ui: &mut Ui) {
        if !self.def_list.is_def_list_def {
            return;
        }
        self.def_list.is_def_list_def = false;
        let item_events = delayed_events(events, |tag| matches!(tag, TagEnd::DefinitionListDefinition));
        let mut events_iter = item_events.into_iter().enumerate().peekable();
        self.line.try_insert_start(ui);
        self.line.should_start_newline = false;
        if let Some((_, (e, span))) = events_iter.next() {
            self.process_event(ui, &mut events_iter, e, span, max_width);
        }
        ui.label(" ".repeat(INDENTATION));
        self.line.should_start_newline = true;
        self.line.should_end_newline = false;
        ui.horizontal_wrapped(|ui| {
            while let Some((_, (e, span))) = events_iter.next() {
                self.process_event(ui, &mut events_iter, e, span, max_width);
            }
        });
        self.line.should_end_newline = true;
        if !matches!(events.peek(), Some((_, (Event::End(TagEnd::DefinitionList), _)))) {
            self.line.try_insert_end(ui);
        }
    }

    fn item_list_wrapping<'e>(&mut self, events: &mut impl Iterator<Item = EventIteratorItem<'e>>, max_width: f32, ui: &mut Ui) {
        if !self.is_list_item {
            return;
        }
        self.is_list_item = false;
        let item_events = delayed_events_list_item(events);
        let mut events_iter = item_events.into_iter().enumerate().peekable();
        ui.horizontal_wrapped(|ui| {
            while let Some((_, (e, span))) = events_iter.next() {
                self.process_event(ui, &mut events_iter, e, span, max_width);
            }
        });
    }

    fn blockquote<'e>(&mut self, events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>, max_width: f32, ui: &mut Ui) {
        if !self.is_blockquote {
            return;
        }
        let mut collected = delayed_events(events, |tag| matches!(tag, TagEnd::BlockQuote(_)));
        self.line.try_insert_start(ui);
        self.line.should_not_start_newline_forced = false;
        match markdown::alert_of(&mut collected, |(e, _)| e) {
            Some(alert) => alert_ui(alert, ui, |ui| {
                for (event, span) in collected {
                    self.event(ui, event, span, max_width);
                }
            }),
            None => blockquote(ui, ui.visuals().weak_text_color(), |ui| {
                self.text_style.quote = true;
                for (event, span) in collected {
                    self.event(ui, event, span, max_width);
                }
                self.text_style.quote = false;
            }),
        }
        if events.peek().is_none() {
            self.line.should_end_newline_forced = false;
        }
        self.line.try_insert_end(ui);
        self.is_blockquote = false;
    }

    fn table<'e>(&mut self, events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>, ui: &mut Ui, max_width: f32) {
        if !self.is_table {
            return;
        }
        self.line.try_insert_start(ui);
        let id = self.id.with("table").with(self.curr_table);
        self.curr_table += 1;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            let Table { header, rows } = parse_table(events);
            egui::Grid::new(id).striped(true).show(ui, |ui| {
                for col in header {
                    ui.horizontal(|ui| self.cell(ui, col, max_width));
                }
                ui.end_row();
                for row in rows {
                    for col in row {
                        ui.horizontal(|ui| self.cell(ui, col, max_width));
                    }
                    ui.end_row();
                }
            });
        });
        self.is_table = false;
        if events.peek().is_none() {
            self.line.should_end_newline_forced = false;
        }
        self.line.try_insert_end(ui);
    }

    fn cell(&mut self, ui: &mut Ui, col: Vec<(Event<'_>, Range<usize>)>, max_width: f32) {
        for (e, span) in col {
            let start = std::mem::replace(&mut self.line.should_start_newline, false);
            let end = std::mem::replace(&mut self.line.should_end_newline, false);
            self.event(ui, e, span, max_width);
            self.line.should_start_newline = start;
            self.line.should_end_newline = end;
        }
    }

    fn event(&mut self, ui: &mut Ui, event: Event, _span: Range<usize>, max_width: f32) {
        match event {
            Event::Start(tag) => self.start_tag(ui, tag),
            Event::End(tag) => self.end_tag(ui, tag, max_width),
            Event::Text(text) => self.event_text(&text, ui),
            Event::Code(text) => {
                self.text_style.code += 1;
                self.event_text(&text, ui);
                self.text_style.code -= 1;
            }
            Event::InlineHtml(html) | Event::Html(html) => self.html(&html, ui),
            Event::FootnoteReference(note) => footnote_start(ui, &note),
            Event::SoftBreak => soft_break(ui),
            Event::HardBreak => newline(ui),
            Event::Rule => {
                self.line.try_insert_start(ui);
                rule(ui, self.line.can_insert_end());
            }
            Event::TaskListMarker(mut checked) => {
                ui.add(ImmutableCheckbox::without_text(&mut checked));
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => self.event_text(&text, ui),
        }
    }

    /// Raw HTML: each drawn tag changes how the text after it looks, `br`
    /// ends the line, and everything else is shown as the text it is.
    fn html(&mut self, html: &CowStr, ui: &mut Ui) {
        for piece in markdown::html_pieces(html) {
            match piece {
                Html::Open("br") => newline(ui),
                Html::Close("br") => {}
                Html::Open(tag) => *self.counter(tag) += 1,
                Html::Close(tag) => {
                    let open = self.counter(tag);
                    *open = open.saturating_sub(1);
                }
                Html::Text(text) => self.event_text(text, ui),
            }
        }
    }

    /// The count of open tags of the kind `tag`, a drawn tag but `br`.
    fn counter(&mut self, tag: &str) -> &mut u32 {
        match tag {
            "b" | "strong" => &mut self.text_style.strong,
            "i" | "em" => &mut self.text_style.emphasis,
            "sup" => &mut self.text_style.superscript,
            "sub" => &mut self.text_style.subscript,
            _ => &mut self.text_style.code,
        }
    }

    fn event_text(&mut self, text: &str, ui: &mut Ui) {
        let job = self.text_style.job(ui, text);
        if let Some(image) = &mut self.image {
            image.alt.push(job);
        } else if let Some(block) = &mut self.code_block {
            block.content.push_str(text);
        } else if let Some(link) = &mut self.link {
            link.text.push(job);
        } else {
            ui.label(job);
        }
    }

    fn start_tag(&mut self, ui: &mut Ui, tag: Tag) {
        match tag {
            Tag::Paragraph => self.line.try_insert_start(ui),
            Tag::Heading { level, id, .. } => {
                if let (Some(wanted), Some(id)) = (&self.scroll_to, &id)
                    && wanted.as_str() == id.as_ref()
                {
                    ui.scroll_to_cursor(Some(egui::Align::TOP));
                    self.scroll_to = None;
                }
                newline(ui);
                self.text_style.heading = Some(match level {
                    HeadingLevel::H1 => 0,
                    HeadingLevel::H2 => 1,
                    HeadingLevel::H3 => 2,
                    HeadingLevel::H4 => 3,
                    HeadingLevel::H5 => 4,
                    HeadingLevel::H6 => 5,
                });
            }
            Tag::BlockQuote(_) => self.is_blockquote = true,
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.to_string()),
                    pulldown_cmark::CodeBlockKind::Fenced(_) | pulldown_cmark::CodeBlockKind::Indented => None,
                };
                self.code_block = Some(CodeBlock { lang, content: String::new() });
                // A code block takes a line of its own, in a list item as well.
                if !self.line.should_not_start_newline_forced {
                    newline(ui);
                }
            }
            Tag::List(point) => {
                if !self.list.is_inside_a_list() && self.line.can_insert_start() {
                    newline(ui);
                }
                match point {
                    Some(number) => self.list.start_level_with_number(number),
                    None => self.list.start_level_without_number(),
                }
                self.line.should_start_newline = false;
                self.line.should_end_newline = false;
            }
            Tag::Item => {
                self.is_list_item = true;
                self.list.start_item(ui);
            }
            Tag::FootnoteDefinition(note) => {
                self.line.try_insert_start(ui);
                self.line.should_start_newline = false;
                self.line.should_end_newline = false;
                footnote(ui, &note);
            }
            Tag::Table(_) => self.is_table = true,
            Tag::TableHead | Tag::TableRow | Tag::TableCell => {}
            Tag::Emphasis => self.text_style.emphasis += 1,
            Tag::Strong => self.text_style.strong += 1,
            Tag::Strikethrough => self.text_style.strikethrough += 1,
            Tag::Superscript => self.text_style.superscript += 1,
            Tag::Subscript => self.text_style.subscript += 1,
            Tag::Link { dest_url, .. } => self.link = Some(Link { destination: dest_url.to_string(), text: Vec::new() }),
            Tag::Image { dest_url, .. } => self.image = Some(ImageLink { address: dest_url.to_string(), alt: Vec::new() }),
            Tag::HtmlBlock => self.line.try_insert_start(ui),
            Tag::MetadataBlock(_) => {}
            Tag::DefinitionList => {
                self.line.try_insert_start(ui);
                self.def_list.is_first_item = true;
            }
            Tag::DefinitionListTitle => {
                if self.def_list.is_first_item {
                    self.def_list.is_first_item = false;
                } else {
                    self.line.try_insert_start(ui);
                }
            }
            Tag::DefinitionListDefinition => self.def_list.is_def_list_def = true,
        }
    }

    fn end_tag(&mut self, ui: &mut Ui, tag: TagEnd, max_width: f32) {
        match tag {
            TagEnd::Paragraph => self.line.try_insert_end(ui),
            TagEnd::Heading(_) => {
                self.line.try_insert_end(ui);
                self.text_style.heading = None;
            }
            TagEnd::BlockQuote(_) => {}
            TagEnd::CodeBlock => {
                if let Some(block) = self.code_block.take() {
                    block.end(ui, max_width);
                    if self.line.should_end_newline_forced {
                        newline(ui);
                    }
                }
            }
            TagEnd::List(_) => {
                if self.list.is_last_level() {
                    self.line.should_start_newline = true;
                    self.line.should_end_newline = true;
                }
                self.list.end_level(ui, self.line.can_insert_end());
                if !self.list.is_inside_a_list() {
                    self.list = List::default();
                }
            }
            TagEnd::Item => {}
            TagEnd::FootnoteDefinition => {
                self.line.should_start_newline = true;
                self.line.should_end_newline = true;
                self.line.try_insert_end(ui);
            }
            TagEnd::Table | TagEnd::TableHead | TagEnd::TableRow => {}
            TagEnd::TableCell => {
                ui.label("  ");
            }
            TagEnd::Emphasis => self.text_style.emphasis = self.text_style.emphasis.saturating_sub(1),
            TagEnd::Strong => self.text_style.strong = self.text_style.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.text_style.strikethrough = self.text_style.strikethrough.saturating_sub(1),
            TagEnd::Superscript => self.text_style.superscript = self.text_style.superscript.saturating_sub(1),
            TagEnd::Subscript => self.text_style.subscript = self.text_style.subscript.saturating_sub(1),
            TagEnd::Link => {
                if let Some(link) = self.link.take() {
                    self.end_link(ui, link);
                }
            }
            TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    self.end_image(ui, image, max_width);
                }
            }
            TagEnd::HtmlBlock | TagEnd::MetadataBlock(_) => {}
            TagEnd::DefinitionList => self.line.try_insert_end(ui),
            TagEnd::DefinitionListTitle | TagEnd::DefinitionListDefinition => {}
        }
    }

    /// A link: its text drawn as a link, a click reported, or for a link
    /// to a heading's `{#id}`, the document scrolled to it.
    fn end_link(&mut self, ui: &mut Ui, link: Link) {
        let Link { destination, text } = link;
        if text.is_empty() {
            return;
        }
        let mut job = LayoutJob::default();
        for part in text {
            for section in part.sections {
                let piece = part.text[section.byte_range.start.0..section.byte_range.end.0].to_string();
                job.append(&piece, section.leading_space, section.format);
            }
        }
        let response = ui.link(job).on_hover_text(destination.as_str());
        if response.clicked() {
            match destination.strip_prefix('#') {
                Some(heading) => self.scroll_requested = Some(heading.to_string()),
                None => self.clicked.push(destination),
            }
        }
    }

    /// An image: its picture drawn at its own size, shrunk to the room
    /// across, or its alt text where it has none or it cannot be drawn,
    /// which is noted with why. Measured, it takes the room its first frame
    /// draws it in and loads nothing: a picture read from bytes at its own
    /// size, an https picture at the size egui gives one still loading, and
    /// one whose bytes give no size as its alt text.
    fn end_image(&mut self, ui: &mut Ui, image: ImageLink, max_width: f32) {
        let ImageLink { address, alt } = image;
        let picture = (self.picture)(&address);
        let own = match &picture {
            Picture::Bytes { size, .. } => *size,
            Picture::Web(_) | Picture::Absent => None,
        };
        let loading = matches!(picture, Picture::Web(_));
        let source = match picture {
            Picture::Bytes { uri, bytes, .. } => Some(egui::ImageSource::Bytes { uri: uri.into(), bytes: egui::load::Bytes::Shared(bytes) }),
            Picture::Web(uri) => Some(egui::ImageSource::Uri(uri.into())),
            Picture::Absent => None,
        };
        let drawn = source.map(|s| egui::Image::new(s).fit_to_original_size(1.0).max_width(max_width));
        if ui.is_sizing_pass() {
            match drawn {
                Some(img) if own.is_some() || loading => {
                    ui.allocate_exact_size(img.calc_size(ui.available_size(), own), Sense::hover());
                }
                _ => {
                    for part in alt {
                        ui.label(part);
                    }
                }
            }
            return;
        }
        let failure = match &drawn {
            Some(img) => match img.load_for_size(ui.ctx(), Vec2::splat(max_width)) {
                Ok(_) => None,
                Err(e) => Some(e.to_string()),
            },
            None => None,
        };
        match (drawn, failure) {
            (Some(img), None) => {
                let response = ui.add(img);
                if !alt.is_empty() {
                    response.on_hover_ui_at_pointer(|ui| {
                        for part in alt {
                            ui.label(part);
                        }
                    });
                }
            }
            (_, failure) => {
                if let Some(why) = failure {
                    self.failed.push((address, why));
                }
                for part in alt {
                    ui.label(part);
                }
            }
        }
    }
}

impl CodeBlock {
    /// The block in a frame, colored by egui_extras's rules for its
    /// language when it has one, plain otherwise.
    fn end(&self, ui: &mut Ui, max_width: f32) {
        ui.scope(|ui| {
            let mut layout = |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                let mut job = match &self.lang {
                    Some(lang) => egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style()), text.as_str(), lang),
                    None => {
                        let mut job = LayoutJob::default();
                        job.append(text.as_str(), 0.0, egui::TextFormat::simple(TextStyle::Monospace.resolve(ui.style()), ui.style().visuals.text_color()));
                        job
                    }
                };
                job.wrap.max_width = wrap_width;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            code_block(ui, max_width, &self.content, &mut layout);
        });
    }
}

/// The events up to and with the one `end_at` matches, or to the end,
/// drawn as one widget.
fn delayed_events<'e>(events: &mut impl Iterator<Item = EventIteratorItem<'e>>, end_at: impl Fn(TagEnd) -> bool) -> Vec<(Event<'e>, Range<usize>)> {
    let mut out = Vec::new();
    for (_, (event, span)) in events.by_ref() {
        let ends = matches!(&event, Event::End(tag) if end_at(*tag));
        out.push((event, span));
        if ends {
            break;
        }
    }
    out
}

/// A list item's events, up to its end or a list it opens.
fn delayed_events_list_item<'e>(events: &mut impl Iterator<Item = EventIteratorItem<'e>>) -> Vec<(Event<'e>, Range<usize>)> {
    let mut out = Vec::new();
    for (_, (event, span)) in events.by_ref() {
        let ends = matches!(&event, Event::End(TagEnd::Item) | Event::Start(Tag::List(_)));
        out.push((event, span));
        if ends {
            break;
        }
    }
    out
}

type Column<'e> = Vec<(Event<'e>, Range<usize>)>;
type Row<'e> = Vec<Column<'e>>;

struct Table<'e> {
    header: Row<'e>,
    rows: Vec<Row<'e>>,
}

fn parse_row<'e>(events: &mut impl Iterator<Item = (Event<'e>, Range<usize>)>) -> Row<'e> {
    let mut row = Vec::new();
    let mut column = Vec::new();
    for (e, span) in events.by_ref() {
        match e {
            Event::End(TagEnd::TableCell) => {
                row.push(std::mem::take(&mut column));
                continue;
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => break,
            other => column.push((other, span)),
        }
    }
    row
}

fn parse_table<'e>(events: &mut impl Iterator<Item = EventIteratorItem<'e>>) -> Table<'e> {
    let mut all = delayed_events(events, |end| matches!(end, TagEnd::Table)).into_iter().peekable();
    let header = parse_row(&mut all);
    let mut rows = Vec::new();
    while all.peek().is_some() {
        rows.push(parse_row(&mut all));
    }
    Table { header, rows }
}

/// An alert's color and icon, egui_commonmark's for GitHub's alerts.
fn alert_look(alert: markdown::Alert) -> (egui::Color32, char) {
    match alert {
        markdown::Alert::Note => (egui::Color32::from_rgb(10, 80, 210), '❕'),
        markdown::Alert::Tip => (egui::Color32::from_rgb(0, 130, 20), '💡'),
        markdown::Alert::Important => (egui::Color32::from_rgb(150, 30, 140), '💬'),
        markdown::Alert::Warning => (egui::Color32::from_rgb(200, 120, 0), '⚠'),
        markdown::Alert::Caution => (egui::Color32::from_rgb(220, 0, 0), '🔴'),
    }
}

/// An alert: a quote in its color under its icon and name.
fn alert_ui(alert: markdown::Alert, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    let (accent, icon) = alert_look(alert);
    blockquote(ui, accent, |ui| {
        newline(ui);
        ui.colored_label(accent, icon.to_string());
        ui.add_space(3.0);
        ui.colored_label(accent, alert.name());
        newline(ui);
        add_contents(ui);
    });
}

fn rule(ui: &mut Ui, end_line: bool) {
    ui.add(egui::Separator::default().horizontal());
    if end_line {
        newline(ui);
    }
}

fn soft_break(ui: &mut Ui) {
    ui.label(" ");
}

fn newline(ui: &mut Ui) {
    ui.label("\n");
}

fn bullet_point(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width_body_space(ui) * 4.0, height_body(ui)), Sense::hover());
    ui.painter().circle_filled(rect.center(), rect.height() / 6.0, ui.visuals().strong_text_color());
}

fn bullet_point_hollow(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width_body_space(ui) * 4.0, height_body(ui)), Sense::hover());
    ui.painter().circle(rect.center(), rect.height() / 6.0, egui::Color32::TRANSPARENT, egui::Stroke::new(0.6, ui.visuals().strong_text_color()));
}

fn number_point(ui: &mut Ui, number: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width_body_space(ui) * 4.0, height_body(ui)), Sense::hover());
    ui.painter().text(rect.right_center(), egui::Align2::RIGHT_CENTER, format!("{number}."), TextStyle::Body.resolve(ui.style()), ui.visuals().strong_text_color());
}

fn footnote_start(ui: &mut Ui, note: &str) {
    ui.label(RichText::new(note).raised().strong().small());
}

fn footnote(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width_body_space(ui) * 4.0, height_body(ui)), Sense::hover());
    ui.painter().text(rect.right_top(), egui::Align2::RIGHT_TOP, format!("{text}."), TextStyle::Small.resolve(ui.style()), ui.visuals().strong_text_color());
}

fn height_body(ui: &Ui) -> f32 {
    ui.text_style_height(&TextStyle::Body)
}

fn width_body_space(ui: &Ui) -> f32 {
    let id = TextStyle::Body.resolve(ui.style());
    ui.fonts_mut(|f| f.glyph_width(&id, ' '))
}

/// A code block: its text selectable, in a frame, with a button that
/// copies it.
fn code_block(ui: &mut Ui, max_width: f32, text: &str, layouter: &mut dyn FnMut(&Ui, &dyn egui::TextBuffer, f32) -> Arc<egui::Galley>) {
    let mut text = text.strip_suffix('\n').unwrap_or(text);
    let background = ui.painter().add(egui::Shape::Noop);
    let output = egui::TextEdit::multiline(&mut text).layouter(layouter).desired_width(max_width).desired_rows(1).show(ui);
    let frame = output.response.rect;
    ui.painter().set(
        background,
        egui::epaint::RectShape::new(frame, ui.style().noninteractive().corner_radius, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Outside),
    );
    let spacing = &ui.style().spacing;
    let at = egui::pos2(frame.right_top().x - spacing.icon_width * 0.5 - spacing.button_padding.x, frame.right_top().y + spacing.button_padding.y * 2.0);
    let copied_id = ui.make_persistent_id(output.response.id);
    let copied = ui.memory_mut(|m| *m.data.get_temp_mut_or_default::<bool>(copied_id));
    let copy = ui
        .put(egui::Rect { min: at, max: at }, egui::Button::new(if copied { "✔" } else { "🗐" }).small().frame(false).fill(egui::Color32::TRANSPARENT))
        .on_hover_cursor(ui.visuals().interact_cursor.unwrap_or(egui::CursorIcon::Default));
    if copied && !copy.hovered() {
        ui.memory_mut(|m| *m.data.get_temp_mut_or_default(copied_id) = false);
    }
    if copy.clicked() {
        ui.memory_mut(|m| *m.data.get_temp_mut_or_default(copied_id) = true);
        let chosen = match output.cursor_range {
            Some(cursor) => {
                let chosen = egui::TextBuffer::char_range(&text, cursor.as_sorted_char_range());
                if chosen.is_empty() { text.to_owned() } else { chosen.to_owned() }
            }
            None => text.to_owned(),
        };
        ui.copy_text(chosen);
    }
}

/// A check box a person cannot change, as a task list's.
struct ImmutableCheckbox<'a> {
    checked: &'a mut bool,
}

impl<'a> ImmutableCheckbox<'a> {
    fn without_text(checked: &'a mut bool) -> Self {
        ImmutableCheckbox { checked }
    }
}

impl egui::Widget for ImmutableCheckbox<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let spacing = ui.spacing();
        let icon_width = spacing.icon_width;
        let mut desired = egui::vec2(icon_width, 0.0).at_least(Vec2::splat(spacing.interact_size.y));
        desired.y = desired.y.max(icon_width);
        let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().visuals.noninteractive();
            let (small, big) = ui.spacing().icon_rectangles(rect);
            ui.painter().add(egui::epaint::RectShape::new(big.expand(visuals.expansion), visuals.corner_radius, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside));
            if *self.checked {
                ui.painter().add(egui::Shape::line(
                    vec![egui::pos2(small.left(), small.center().y), egui::pos2(small.center().x, small.bottom()), egui::pos2(small.right(), small.top())],
                    visuals.fg_stroke,
                ));
            }
        }
        response
    }
}

/// A blockquote: its contents indented, with a line down its left.
fn blockquote(ui: &mut Ui, accent: egui::Color32, add_contents: impl FnOnce(&mut Ui)) {
    let start = ui.painter().add(egui::Shape::Noop);
    let response = egui::Frame::new().outer_margin(egui::Margin { left: 10, ..Default::default() }).show(ui, add_contents).response;
    ui.painter().set(
        start,
        egui::epaint::Shape::line_segment(
            [egui::pos2(response.rect.left_top().x, response.rect.left_top().y + 5.0), egui::pos2(response.rect.left_bottom().x, response.rect.left_bottom().y - 5.0)],
            egui::Stroke::new(3.0, accent),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// egui_extras's own highlighter, which this renderer draws code with,
    /// and [`markdown::code_tokens`], which the HTML renderer colors code
    /// by, tell every piece of code apart alike: the tokens in the light
    /// theme's colors for their kinds, neighbors of one color joined as a
    /// layout job joins them, are the highlighter's sections.
    #[test]
    fn code_is_told_apart_as_egui_extras_tells_it_apart() {
        let ctx = egui::Context::default();
        let style = ctx.style_of(egui::Theme::Light);
        let theme = egui_extras::syntax_highlighting::CodeTheme::light(12.0);
        let color = |token: markdown::Token| match token {
            markdown::Token::Comment => egui::Color32::GRAY,
            markdown::Token::Function => egui::Color32::from_rgb(170, 110, 0),
            markdown::Token::Keyword => egui::Color32::from_rgb(235, 0, 0),
            markdown::Token::Literal => egui::Color32::from_rgb(153, 134, 255),
            markdown::Token::String => egui::Color32::from_rgb(37, 203, 105),
            markdown::Token::Punctuation => egui::Color32::DARK_GRAY,
            markdown::Token::Whitespace => egui::Color32::TRANSPARENT,
        };
        let samples = [
            ("rust", "fn main() {\n    let s = \"a \\\" b\"; // done\n    r#\"x\"#;\n    my_fn(2x);\n}"),
            ("c++", "#include <x>\nint main() { /* a\nb */ return 'c'; } // é"),
            ("ts", "const f = (x) => `t${x}`; // c\nawait g(\"s\");"),
            ("json", "{\"a\": [1, true, null], /* c */ \"b\": \"\\\"\"} // d"),
            ("py", "def f(x):\n    return None  # é\n\"\"\"doc\nmore\"\"\"\n'x'\n\"open"),
            ("sh", "if [ -f \"$x\" ]; then echo 'y'; fi # z"),
            ("SQL", "Select a, COUNT(*) FROM t -- n\nwhere b = 'c''d'"),
            ("TOML", "[a]\nb = \"c\" # d\ne = '''f'''"),
            ("yaml", "a: true\nb: \"c\" # d\n- 'e'"),
        ];
        for (language, code) in samples {
            let job = egui_extras::syntax_highlighting::highlight(&ctx, &style, &theme, code, language);
            let theirs: Vec<(egui::Color32, String)> = job.sections.iter().map(|s| (s.format.color, job.text[s.byte_range.start.0..s.byte_range.end.0].to_string())).collect();
            let mut ours: Vec<(egui::Color32, String)> = Vec::new();
            for (token, piece) in markdown::code_tokens(code, language).expect("a language egui_extras colors") {
                match ours.last_mut() {
                    Some((last, text)) if *last == color(token) => text.push_str(piece),
                    Some(_) | None => ours.push((color(token), piece.to_string())),
                }
            }
            assert_eq!(ours, theirs, "{language}");
        }
    }
}
