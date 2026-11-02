//! The messages between a PWRS-GUI session and its renderers, the window
//! tree they describe, the transport that moves them between processes,
//! the link every renderer takes ([`renderer`]), and a helper process's
//! side of the session ([`helper`]).
//!
//! Between processes a message travels postcard-encoded on a SubEtha ring
//! ([`transport`]); in-process the same values move over in-memory
//! channels, unencoded. A helper's first message is [`FromHelper::Hello`],
//! on the start ring the session named on its command line; the session
//! accepts only a helper whose `protocol` equals [`PROTOCOL`].
//!
//! Where a helper's messages go: `Hello` to the start ring; `Opened`,
//! `Event`, `Closed`, `BridgeMessage`, `PageClosed` and an `Error` naming
//! a window to that window's ring;
//! `Adapters`, `Windows`, `TestEventDone` and `Snapshot` to the reply ring
//! the request named. `Log` and an `Error` naming no window go to the
//! helper's standard error, which the session writes to a log file.

pub mod button_bar;
pub mod camera;
pub mod canvas;
pub mod chart;
pub mod chooser;
pub mod clock;
pub mod date;
pub mod gpu;
pub mod helper;
pub mod markdown;
pub mod media;
pub mod outbound;
pub mod peek;
pub mod picture;
pub mod place;
pub mod renderer;
pub mod rows;
pub mod shared;
pub mod source;
pub mod svg_path;
pub mod test_input;
pub mod tokens;
pub mod trace;
pub mod transport;
#[cfg(feature = "typeface")]
pub mod typeface;
pub mod view;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The protocol number both sides must share.
pub const PROTOCOL: u32 = 7;

/// A window, numbered by the session.
pub type WindowId = u64;
/// A node, numbered within its window from 1.
pub type NodeId = u32;
/// A request's number, which its one reply gives back.
pub type RequestId = u64;
/// The name a ring is created and opened under.
pub type RingName = String;

/// A property value or an event's value. Bytes are shared, so a copy of a
/// tree with a picture in it shares the picture's bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
    Bytes(Arc<[u8]>),
}

impl Value {
    /// The string in a `Str`; none for any other variant.
    pub fn as_str(&self) -> Option<&str> {
        if let Value::Str(s) = self { Some(s) } else { None }
    }

    /// The flag in a `Bool`; none for any other variant.
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self { Some(*b) } else { None }
    }

    /// The integer in an `Int`; none for any other variant.
    pub fn as_int(&self) -> Option<i64> {
        if let Value::Int(i) = self { Some(*i) } else { None }
    }

    /// The number in an `Int` or a `Float`; none for any other variant.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// The items in a `List`; none for any other variant.
    pub fn as_list(&self) -> Option<&[Value]> {
        if let Value::List(items) = self { Some(items) } else { None }
    }

    /// The entries in a `Map`; none for any other variant.
    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        if let Value::Map(m) = self { Some(m) } else { None }
    }
}

/// What a node draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// Children stacked top to bottom, each placed across the room by
    /// `align` ([`Align`]): at its left with `start`, the default, in the
    /// middle with `center`, at its right with `end`, or as wide as the room
    /// with `stretch`. A child that fills ([`fills_down`]) shares with the
    /// others that fill the height the rest leave, so it scrolls rather than
    /// push them out of the window.
    Column,
    /// Children side by side, each centered top to bottom, placed as a group
    /// across the room by `align`: at its left with `start`, the default, in
    /// the middle with `center`, at its right with `end`, or sharing the
    /// room equally with `stretch`.
    Row,
    /// Children in `columns` columns, a whole number above 0, filled a row
    /// at a time. `align` places each child in its cell as a Column places
    /// its children, and with `stretch` the last column also takes the room
    /// left across.
    Grid,
    /// Children stacked in a frame, under `title` when given.
    Group,
    /// Its `Tab` children as tabs, one shown at a time: the one in
    /// `selected`, a row number, the first when absent or null. Raises
    /// `Change` with the tab chosen.
    Tabs,
    /// One tab of a `Tabs`: `title`, and children stacked as a `Column`'s,
    /// drawn while the tab is chosen.
    Tab,
    /// Its two children side by side, or with `orientation` `vertical` one
    /// above the other, the first taking `position` of the space, a share
    /// from 0 to 1 (0.5 when absent), which a person changes by dragging
    /// the divider between them.
    Split,
    /// Children stacked in an area that scrolls: at most `height` points
    /// high when given, else as high as its Column leaves it
    /// ([`fills_down`]).
    Scroll,
    /// Text: `text`, and `style` of `heading`, `body`, `mono`, `link` or
    /// `instruction`, a dialog's main line in the size
    /// [`tokens::DIALOG`] gives it and the normal text color. A link raises
    /// `Click`.
    Text,
    /// A picture: `image`, a map of its `bytes` and their `format`, `png`,
    /// `jpeg`, `gif`, `webp`, `bmp`, `ico` or `svg`. It shows at its size in
    /// pixels as points, shrunk to fit the space it gets without changing
    /// its proportions; `width` or `height`, in points, sets that side and
    /// scales the other, and both make a box that `fit` fills: `contain`,
    /// when absent, shows the whole picture inside it, `cover` fills it and
    /// crops what spills over, `fill` stretches the picture to it, and
    /// `none` shows the picture at its own size, centered and cropped to the
    /// box. `alt` is the picture's text, read for it and shown where it
    /// cannot be drawn.
    Image,
    /// A CommonMark document, read as [`markdown`] says: `document`, a map
    /// of its `text` and its `pictures`, a map from each image address the
    /// session read a picture for to a map of the picture's `bytes` and
    /// `format`, as an Image's. A renderer fetches an https image itself;
    /// any other image shows its alt text. Of raw HTML, the tags
    /// [`markdown::DRAWN_TAGS`] names are drawn and every other shows as
    /// text. A link raises `Click` with its address, and one to a heading's
    /// `{#id}` scrolls to it instead; a test Click names the link by its
    /// address.
    Markdown,
    /// A progress bar: `value` out of `maximum` (100 when absent), from 0,
    /// with `text` on it when given and the percentage when not. A `ring`
    /// names a SubEtha ring whose items set them ([`source`]).
    ProgressBar,
    /// A spinner that turns while it is shown, for work of unknown length.
    Spinner,
    /// Lines of text, `lines` (strings), monospaced, which stay scrolled to
    /// the newest unless a person scrolls up: at most `height` points high
    /// when given, else as high as its Column leaves it. A `ring` names a
    /// SubEtha ring whose items each add a line ([`source`]).
    LogView,
    /// A chart of `series`, a list of maps, each with its `name`, `y` (a
    /// list of numbers), `x` (numbers, or with `dates` true dates written
    /// as [`date`] writes them; 0, 1, 2 and on when absent) and `type`,
    /// `line`, `bar` or `points` (the chart's own `type` when absent,
    /// `line` when neither has one). `x_label` and `y_label` name the axes,
    /// and the chart is `height` points high when given, else as high as
    /// its Column leaves it. Each point in `points`, a list of maps of
    /// `series`, `x` and `y` that grows by [`PatchOp::Append`], joins the
    /// series of that name, or starts one of the chart's type; one without
    /// `x` takes its series' count of points. [`chart::Chart`] reads it. A
    /// person pans, zooms and reads the values under the pointer, and a
    /// click on a point raises `Select` with a map of its `series`, its `x`
    /// as the session wrote it, and `y`. A `ring` names a SubEtha ring whose
    /// items each add a point ([`source`]).
    Chart,
    /// A texture the session's WGSL draws on with the egui renderer's GPU,
    /// `width` by `height` points, each above 0: the room across its place
    /// and the height its Column leaves it when absent. What it draws comes
    /// from [`gpu::GpuOp::SetSurface`], and each frame clears it to its color
    /// and draws that; until then it shows its window's own background.
    /// [`gpu::GpuOp::SaveSurface`] reads it back.
    GpuSurface,
    /// A canvas of `items`, a list of item maps [`canvas::Scene`] reads,
    /// each its `kind`, its `name` and its properties under the lowercase
    /// names of the keys a script writes, a group's items under `children`,
    /// drawn in order with the first underneath; `width` by `height`
    /// points, each above 0, the room across its place and the height its
    /// Column leaves it when absent. [`PatchOp::SetItem`],
    /// [`PatchOp::AddItems`] and [`PatchOp::RemoveItems`] change one item,
    /// add items and take them away by name. With `click`, `doubleClick`,
    /// `pointerEnter`, `pointerLeave`, `dragStart`, `dragMove` or `dragEnd`
    /// true it raises that event for the topmost named item under the
    /// pointer, and with `pointerDown`, `pointerUp`, `pointerMove` or
    /// `wheel` true those of a GpuSurface for the canvas itself, each
    /// event's value the map a GpuSurface's PointerDown gives, with `item`,
    /// the item's name, or null where no named item is under the pointer. A
    /// `ring` names a SubEtha ring whose items change its items
    /// ([`source::canvas_changes`]), and `shared` names shared memory whose
    /// rows set its items' properties ([`shared`]).
    Canvas,
    /// A media player of the clip in `source`, the full path of its file or
    /// an https address, played by the renderer's own backend. The helper's
    /// loader fetches an https clip, unless `engine_fetch`: then the
    /// player's own engine fetches it on the system's network stack, its
    /// server allowing pages of [`media::PAGE_ORIGIN`] under CORS. It shows a
    /// picture of the clip's size in pixels as points, shrunk to fit its
    /// place with its proportions kept, `width` and `height` points when
    /// given, or with `audio_only` the clip's sound alone and no picture. It
    /// starts at once with `autoplay`, plays again from the start at each
    /// end with `loop`, and plays at `volume`, from 0 to 1 (1 when absent),
    /// silenced while `muted`, at `rate` times the clip's speed, within
    /// [`media::RATES`] (1 when absent). It draws its controls unless
    /// `no_controls`, and shows the cues of `subtitles`
    /// ([`media::subtitles::value`]) and of the clip's own text track at
    /// their times. [`ToHelper::Media`] plays, pauses, seeks and sets it
    /// ([`media`]). It raises `Loaded` once the clip is open, with
    /// [`media::loaded_value`]; `Playing`, `Paused` and `Ended` with the
    /// position in seconds; with `position` true, `Position` every
    /// `position_interval` seconds while it plays
    /// ([`media::POSITION_INTERVAL`] when absent), after each seek and at
    /// the end; and `Error` with [`media::error_value`] when the clip cannot
    /// be played.
    MediaPlayer,
    /// A button: `text`, `enabled`. Raises `Click`. With `default` true,
    /// the window's one default button: drawn in the selection's color, and
    /// pressed by Enter wherever the focused widget does not use Enter
    /// itself. With `cancel` true, the window's one cancel button, pressed
    /// by Escape wherever the focused widget does not use Escape itself.
    Button,
    /// A list of `items` (strings) in a frame, one or `multiple` selectable,
    /// each row taking the frame's width, the selection in `selected` (row
    /// numbers); it fills ([`fills_down`]) and scrolls. Raises `Select` with
    /// the selected row numbers and `Activate` with the row double-clicked
    /// or chosen with Enter.
    ListBox,
    /// A table: `columns` (a list of maps with a `label` and an `align` of
    /// `left`, `center` or `right`; without one, a column whose first row's
    /// cell sorts as a number is right-aligned, as PowerShell's tables align
    /// numbers, and any other left), rows sent
    /// with [`ToHelper::Rows`], sorting, a filter, and `selection` of
    /// `none`, `single` or `multiple`; it fills ([`fills_down`]) and
    /// scrolls. Raises `Select` with the selected row numbers and
    /// `Activate` with the row double-clicked.
    Table,
    /// A tree of its `TreeItem` children, each with its own, one or with
    /// `multiple` several selectable, the selection in `selected` (the
    /// items' node ids); it fills ([`fills_down`]) and scrolls. A person
    /// opens an item to show its children;
    /// opening one that has `more` and no children raises `Expand` with the
    /// item's id, and the session replies with the item's children and
    /// `more` false, or with `more` true and no children, which closes it
    /// again. Raises `Select` with the selected items' ids in tree order
    /// and `Activate` with the item double-clicked or chosen with Enter. A
    /// test event names items by their rows as shown, from 0.
    TreeView,
    /// One item of a `TreeView`: `text`, its `TreeItem` children, and
    /// `more` when it has children the session has not given yet.
    TreeItem,
    /// A check box: `text`, `checked` (unchecked when absent), `enabled`.
    /// Raises `Change` with whether it is checked.
    CheckBox,
    /// A text field: `text`, one line unless `multiline`, its characters
    /// hidden with `password` (a password field is one line),
    /// `placeholder` shown while it is empty, `enabled`. Raises `Change`
    /// with the text after each edit, and in a field of one line `Submit`
    /// with the text when Enter is pressed.
    TextBox,
    /// A number field: `value`, from `minimum` to `maximum` when they are
    /// given, changed by `step` (1 when absent), `enabled`.
    /// Raises `Change` with the number, a `Float`.
    NumberBox,
    /// A slider: `value` between `minimum` and `maximum`, in steps of
    /// `step` (1 when absent), `enabled`. Raises `Change` with the number,
    /// a `Float`, as it moves.
    Slider,
    /// A drop-down of `items` (strings), the row chosen in `selected`
    /// (none when absent or null), `enabled`. Raises `Change` with the row
    /// chosen. With `editable`, a text field instead, its `text` typed or
    /// picked from `items`, which raises `Change` with the text.
    ComboBox,
    /// Option buttons, one per item of `items` (strings), the row chosen in
    /// `selected` (none when absent or null), `enabled`. Raises `Change`
    /// with the row chosen.
    RadioGroup,
    /// A color chooser: `color`, `#rrggbb`, `enabled`. Raises `Change` with
    /// the color chosen, `#rrggbb` in lower case.
    ColorPicker,
    /// A date and time chooser: `value`, written as [`date`] writes a date
    /// and time to the minute (`YYYY-MM-DDTHH:MM`), none when absent or
    /// null, from `minimum` to `maximum` when given, written the same way;
    /// `enabled`. Raises `Change` with the date and time chosen,
    /// written the same way.
    DatePicker,
    /// A file chooser: the paths chosen, `paths`, a list of at most one
    /// unless `multiple`, beside a button that opens the system's dialog for
    /// a file to open, with `save` a file to save, or with `folder` a
    /// folder. `filter`, a list of maps of a `name` and its `extensions`
    /// (without dots; `*` for any), limits the files the dialog shows,
    /// `title` names the dialog, and `enabled`. Raises `Change` with the
    /// paths chosen, a list.
    FilePicker,
    /// A bar of menus along the top of the window, whatever its place among
    /// the root's children: its `Menu` children.
    MenuBar,
    /// A menu, `title`, in a menu bar, a context menu or another menu: its
    /// `MenuItem`, `Menu` and `Separator` children.
    Menu,
    /// A menu's item: `text`, `enabled`, and `shortcut`, as `Ctrl+Shift+S`,
    /// shown beside the text, which pressed in the window acts as a click.
    /// Raises `Click`.
    MenuItem,
    /// A menu that opens where a person right-clicks the widget named in
    /// `for`: its `MenuItem`, `Menu` and `Separator` children.
    ContextMenu,
    /// A bar along the bottom of the window, whatever its place among the
    /// root's children: its children side by side.
    StatusBar,
    /// The window's buttons along its bottom, above a status bar, whatever
    /// its place among the root's children: its `Button` children at the
    /// right, in the order [`button_bar::order`] gives them on the system
    /// drawing it, each at least [`tokens::DIALOG`]'s button size, with the
    /// window's widgets drawn inside a dialog's margin. On Windows the bar
    /// is a strip of [`tokens::Palette::strip`] with a line above it, as
    /// Windows' own dialogs draw theirs.
    ButtonBar,
    /// A horizontal rule.
    Separator,
    /// Empty space, `size` points (8 when absent); with `fill` true, the
    /// room its Row or Column leaves, at least `size`.
    Spacer,
    /// A page of the caller's own as the whole of a window, which only the
    /// HTML renderer draws: `html`, the document's text, or `folder`, a
    /// folder whose files load through `pwrs://` and whose file `page`
    /// (`index.html` when absent) opens first. `allow_navigation` lets the page leave its
    /// own origin and load remote content; `dev_tools` lets the developer
    /// tools open. The page gets `window.pwrs.send(value)`, a Promise of
    /// the session's [`ToHelper::BridgeReply`], `window.pwrs.on(handler)`
    /// for [`ToHelper::Bridge`] values, `window.pwrs.close(value)`, and
    /// `window.pwrs.save(value)` with `window.pwrs.restore()`, a Promise of
    /// the last value saved, which a page loaded again after its engine
    /// ended gets back; null when nothing was saved.
    Html,
}

/// Where a Row, Column or Grid places its children across, as its `align`
/// says ([`Kind::Column`], [`Kind::Row`], [`Kind::Grid`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

impl Align {
    /// A node's `align`, [`Align::Start`] when absent or null; refused
    /// unless it is `start`, `center`, `end` or `stretch`.
    pub fn of(node: &Node) -> Result<Align, String> {
        match node.props.get("align") {
            None | Some(Value::Null) => Ok(Align::Start),
            Some(Value::Str(s)) => match s.as_str() {
                "start" => Ok(Align::Start),
                "center" => Ok(Align::Center),
                "end" => Ok(Align::End),
                "stretch" => Ok(Align::Stretch),
                other => Err(format!("align must be start, center, end or stretch, not {other}")),
            },
            Some(other) => Err(format!("align must be start, center, end or stretch, not {other:?}")),
        }
    }
}

/// Whether `node` fills down: takes, with the other children of its Column
/// that do, the height the rest of the Column leaves, so it scrolls rather
/// than push its neighbors out of the window. A list, a table and a tree do;
/// a log, a scroll area, a chart, a GPU surface and a canvas without a
/// `height`; a media player showing a picture, without a `height`; a Split
/// standing its children one above the other or with one
/// that fills; a Spacer with `fill`; and a Column, Group, Tabs or Tab with
/// a child that fills, or a Row or Grid with one that is not a Spacer,
/// whose `fill` fills across there.
pub fn fills_down(node: &Node) -> bool {
    let sized = node.props.get("height").is_some_and(|h| *h != Value::Null);
    let not_spacer = |c: &&Node| c.kind != Kind::Spacer;
    match node.kind {
        Kind::ListBox | Kind::Table | Kind::TreeView => true,
        Kind::LogView | Kind::Scroll | Kind::Chart | Kind::GpuSurface | Kind::Canvas => !sized,
        Kind::MediaPlayer => !sized && node.props.get("audio_only").and_then(Value::as_bool) != Some(true),
        Kind::Split => node.props.get("orientation").and_then(Value::as_str) == Some("vertical") || node.children.iter().any(fills_down),
        Kind::Spacer => node.props.get("fill").and_then(Value::as_bool) == Some(true),
        Kind::Column | Kind::Group | Kind::Tabs | Kind::Tab => node.children.iter().any(fills_down),
        Kind::Row | Kind::Grid => node.children.iter().filter(not_spacer).any(fills_down),
        _ => false,
    }
}

/// Whether `node` is laid out where it stands among its siblings, taking
/// room there: every widget but a menu bar, a status bar and a button bar,
/// which stand along the window's edges, and a context menu, which opens
/// over the window.
pub fn in_flow(node: &Node) -> bool {
    !matches!(node.kind, Kind::MenuBar | Kind::StatusBar | Kind::ButtonBar | Kind::ContextMenu)
}

/// One node of a window's tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: Kind,
    pub name: Option<String>,
    pub props: BTreeMap<String, Value>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(id: NodeId, kind: Kind) -> Node {
        Node { id, kind, name: None, props: BTreeMap::new(), children: Vec::new() }
    }

    pub fn named(mut self, name: impl Into<String>) -> Node {
        self.name = Some(name.into());
        self
    }

    pub fn prop(mut self, key: impl Into<String>, value: Value) -> Node {
        self.props.insert(key.into(), value);
        self
    }

    pub fn child(mut self, node: Node) -> Node {
        self.children.push(node);
        self
    }

    /// Whether the node can take the keyboard focus, as Tab moves it: an
    /// input, a button, a list, table or tree, a Text drawn as a link, a
    /// GpuSurface whose `keys` is true, and a Canvas, which takes the focus
    /// while it takes keys, for its node's `keys` or its view's controls.
    /// A disabled widget takes it once it is enabled again.
    pub fn takes_focus(&self) -> bool {
        match self.kind {
            Kind::Button
            | Kind::CheckBox
            | Kind::TextBox
            | Kind::NumberBox
            | Kind::Slider
            | Kind::ComboBox
            | Kind::RadioGroup
            | Kind::ColorPicker
            | Kind::DatePicker
            | Kind::FilePicker
            | Kind::ListBox
            | Kind::Table
            | Kind::TreeView
            | Kind::Canvas => true,
            Kind::GpuSurface => self.props.get("keys") == Some(&Value::Bool(true)),
            Kind::Text => self.props.get("style") == Some(&Value::Str(String::from("link"))),
            _ => false,
        }
    }

    /// Whether the node's `enabled` is false.
    pub fn disabled(&self) -> bool {
        self.props.get("enabled") == Some(&Value::Bool(false))
    }

    /// Whether the node is a password field: a TextBox whose `password` is
    /// true.
    pub fn is_password(&self) -> bool {
        self.kind == Kind::TextBox && self.props.get("password") == Some(&Value::Bool(true))
    }

    /// Why the widget cannot take the keyboard focus, as its properties say,
    /// which both renderers refuse a focus with; `None` when it can. A
    /// renderer also refuses a widget it has not drawn, and a Canvas taking
    /// no keys.
    pub fn focus_refusal(&self) -> Option<String> {
        if self.takes_focus() {
            return self.disabled().then(|| String::from("the widget is disabled"));
        }
        Some(match self.kind {
            Kind::Text => String::from("a Text takes the focus only when its style is link"),
            Kind::GpuSurface => String::from("a GpuSurface takes the focus only while its keys is true"),
            other => format!("a {other:?} takes no focus"),
        })
    }

    /// The widget with the focus while the focus is on node `id`: the
    /// innermost node that takes the focus ([`Node::takes_focus`]) among
    /// `id` and the nodes it is inside; `None` when there is none, or this
    /// subtree has no node `id`.
    pub fn focus_holder(&self, id: NodeId) -> Option<NodeId> {
        let own = self.takes_focus().then_some(self.id);
        if self.id == id {
            return own;
        }
        let inside = self.children.iter().find(|c| c.find(id).is_some())?;
        inside.focus_holder(id).or(own)
    }

    /// The node with this id in this subtree.
    pub fn find(&self, id: NodeId) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    /// The node with this id in this subtree, mutably.
    pub fn find_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_mut(id))
    }

    /// The node with this name in this subtree, compared without regard
    /// to case, as PowerShell compares names.
    pub fn find_named(&self, name: &str) -> Option<&Node> {
        if self.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)) {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find_named(name))
    }

    /// Every name in this subtree, in tree order.
    pub fn names(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_names(&mut out);
        out
    }

    fn collect_names(&self, out: &mut Vec<String>) {
        if let Some(n) = &self.name {
            out.push(n.clone());
        }
        for c in &self.children {
            c.collect_names(out);
        }
    }
}

/// A keyboard shortcut: the modifiers and the key pressed. A letter or a
/// digit is the one the key types, or, where the key types a symbol with
/// the modifiers pressed, as Shift and 1 types ! on many keyboards, the
/// one at the key's place; Plus, Minus and Equals are the keys typing +, -
/// and =, however the keyboard types them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The key, as [`Shortcut::KEYS`] names it, or a letter, a digit or
    /// `F1` to `F24`; a letter in upper case.
    pub key: String,
}

impl Shortcut {
    /// The keys a shortcut can name besides letters, digits and F keys.
    pub const KEYS: &'static [&'static str] =
        &["Enter", "Escape", "Tab", "Space", "Backspace", "Delete", "Insert", "Home", "End", "PageUp", "PageDown", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Plus", "Minus", "Equals"];

    /// Reads a shortcut written as `Ctrl+Shift+S`: `Ctrl`, `Shift` and
    /// `Alt`, without regard to case, joined by `+` to one key.
    pub fn parse(text: &str) -> Result<Shortcut, String> {
        let mut shortcut = Shortcut { ctrl: false, shift: false, alt: false, key: String::new() };
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let Some((key, modifiers)) = parts.split_last() else {
            return Err(format!("{text:?} names no key"));
        };
        for m in modifiers {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" => shortcut.ctrl = true,
                "shift" => shortcut.shift = true,
                "alt" => shortcut.alt = true,
                _ => return Err(format!("{text:?} has {m:?}, which is not Ctrl, Shift or Alt")),
            }
        }
        let f_key = (1..=24u8).map(|n| format!("F{n}")).find(|f| f.eq_ignore_ascii_case(key));
        shortcut.key = if key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric()) {
            key.to_ascii_uppercase()
        } else if let Some(f) = f_key {
            f
        } else if let Some(named) = Shortcut::KEYS.iter().find(|k| k.eq_ignore_ascii_case(key)) {
            named.to_string()
        } else {
            return Err(format!("{text:?} names the key {key:?}; a shortcut's key is a letter, a digit, F1 to F24 or one of {}", Shortcut::KEYS.join(", ")));
        };
        Ok(shortcut)
    }

    /// The shortcut as [`Shortcut::parse`] reads it, its modifiers in the
    /// order Ctrl, Shift, Alt.
    pub fn written(&self) -> String {
        let mut out = String::new();
        for (on, name) in [(self.ctrl, "Ctrl+"), (self.shift, "Shift+"), (self.alt, "Alt+")] {
            if on {
                out.push_str(name);
            }
        }
        out.push_str(&self.key);
        out
    }

    /// The keys a focused text field keeps with Ctrl, with or without
    /// Shift: those it selects all, copies, cuts, pastes, undoes and redoes
    /// with, and moves and deletes by word with.
    pub const FIELD_CTRL_KEYS: &'static [&'static str] = &["A", "C", "X", "V", "Z", "Y", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "Backspace", "Delete"];

    /// Whether a focused text field keeps this shortcut's key from the
    /// window's and its menus' shortcuts: a key without Ctrl or Alt other
    /// than F1 to F24, or Ctrl without Alt and one of
    /// [`Shortcut::FIELD_CTRL_KEYS`].
    pub fn field_keeps(&self) -> bool {
        let f_key = self.key.strip_prefix('F').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        match (self.ctrl, self.alt) {
            (false, false) => !f_key,
            (true, false) => Shortcut::FIELD_CTRL_KEYS.contains(&self.key.as_str()),
            (_, true) => false,
        }
    }

    /// Whether this is Alt+F8, which shows a password field's text while it
    /// is held and the field offers its peek button ([`peek::Peek`]), as
    /// Windows' PasswordBox does; a password field keeps it from the
    /// window's and its menus' shortcuts.
    pub fn is_peek(&self) -> bool {
        self.alt && !self.ctrl && !self.shift && self.key == "F8"
    }

    /// Whether a focused widget of `kind` uses this key itself, so that it
    /// raises no `Key` event. Tab, with Shift or not, moves the focus from
    /// any widget. A field typed into (`typed`: a text field, a number
    /// field, an editable drop-down) keeps what [`Shortcut::field_keeps`]
    /// says. A button, check box, color chooser, file chooser, set of
    /// option buttons, drop-down and date chooser keep Space and Enter,
    /// which work them. A list, table and tree keep the arrows, Home, End,
    /// PageUp and PageDown with Ctrl or Shift or neither, Space and Enter
    /// without Ctrl, and Ctrl+A. A table also keeps Ctrl+C, which copies its
    /// rows selected ([`rows::copied`]). A slider keeps the arrows, Home,
    /// End, PageUp and PageDown, with Shift or not. Option buttons keep the
    /// arrows, a drop-down the arrows, Home, End and Escape, and a date
    /// chooser Escape, which closes its calendar. A media player keeps the
    /// keys a browser's player takes: Space and Enter, the arrows, M, F and
    /// Escape, with no Ctrl or Alt ([`media::KEYS`]).
    pub fn widget_keeps(&self, kind: Kind, typed: bool) -> bool {
        let key = self.key.as_str();
        if key == "Tab" && !self.ctrl && !self.alt {
            return true;
        }
        if typed {
            return self.field_keeps();
        }
        let plain = !self.ctrl && !self.shift && !self.alt;
        let arrow = matches!(key, "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight");
        let moves = arrow || matches!(key, "Home" | "End" | "PageUp" | "PageDown");
        let press = plain && matches!(key, "Space" | "Enter");
        match kind {
            Kind::Table if self.ctrl && !self.shift && !self.alt && key == "C" => true,
            Kind::ListBox | Kind::Table | Kind::TreeView => !self.alt && (moves || (matches!(key, "Space" | "Enter") && !self.ctrl) || (self.ctrl && !self.shift && key == "A")),
            Kind::Slider => !self.ctrl && !self.alt && moves,
            Kind::RadioGroup => press || (plain && arrow),
            Kind::ComboBox => press || (plain && (arrow || matches!(key, "Home" | "End" | "Escape"))),
            Kind::DatePicker => press || (plain && key == "Escape"),
            Kind::Button | Kind::CheckBox | Kind::ColorPicker | Kind::FilePicker => press,
            Kind::MediaPlayer => !self.ctrl && !self.alt && media::Key::of(key).is_some(),
            _ => false,
        }
    }

    /// Whether the focused widget `node` uses this key itself, so that it
    /// raises no `Key` event: what [`Shortcut::widget_keeps`] says of its
    /// kind and `typed`, and for a password field Alt+F8 as well
    /// ([`Shortcut::is_peek`]).
    pub fn node_keeps(&self, node: &Node, typed: bool) -> bool {
        self.widget_keeps(node.kind, typed) || (self.is_peek() && node.is_password())
    }
}

/// The inside size, in points, across and down, of a page's window on a
/// side it is given no size for, the same in every renderer.
pub const PAGE_SIZE: [f32; 2] = [800.0, 600.0];

/// The most of the screen's work area, across and down, a window's side
/// fitted to its content takes.
pub const FIT_SHARE: [f32; 2] = [0.9, 0.8];

/// The work area, in points, a window is fitted within where the screen it
/// opens on cannot be read: the 1024 by 768 points Microsoft's layout
/// guidelines lay resizable windows out for.
pub const ASSUMED_WORK_AREA: [f32; 2] = [1024.0, 768.0];

/// The theme a window is drawn in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    /// The system's, light or dark, following it while the window is open.
    #[default]
    System,
    Light,
    Dark,
}

/// The fonts a renderer draws text in, from its start. Either way the
/// system's fonts draw what the chosen ones lack, as a script they have no
/// letters for, and egui's own fonts what the system lacks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fonts {
    /// Inter and JetBrains Mono NL, which the build includes, the same on
    /// every system.
    #[default]
    Bundled,
    /// The system's own: its UI font, and the monospace font its browser
    /// draws with.
    System,
}

impl Fonts {
    /// The name PowerShell gives it.
    pub fn name(self) -> &'static str {
        match self {
            Fonts::Bundled => "Bundled",
            Fonts::System => "System",
        }
    }

    /// As a helper's `--fonts` argument writes it.
    pub fn arg(self) -> &'static str {
        match self {
            Fonts::Bundled => "bundled",
            Fonts::System => "system",
        }
    }

    /// From a helper's `--fonts` argument.
    pub fn from_arg(written: &str) -> Result<Fonts, String> {
        [Fonts::Bundled, Fonts::System].into_iter().find(|f| f.arg() == written).ok_or_else(|| format!("--fonts is bundled or system, not {written}"))
    }
}

/// The least and most, in points across and down, a window's inside takes
/// on a side fitted to its content; a side with no most takes at most
/// [`FIT_SHARE`] of the work area.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    pub min: [f32; 2],
    pub max: [Option<f32>; 2],
}

impl Fit {
    /// A window of widgets: [`tokens::DIALOG`]'s least width, no most but
    /// the screen's.
    pub const WINDOW: Fit = Fit { min: [tokens::DIALOG.width[0], 0.0], max: [None, None] };
    /// A dialog: across from [`tokens::DIALOG`]'s least width to its most,
    /// and down at most its most height.
    pub const DIALOG: Fit = Fit { min: [tokens::DIALOG.width[0], 0.0], max: [Some(tokens::DIALOG.width[1]), Some(tokens::DIALOG.height_max)] };
}

/// How a window opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowOptions {
    pub title: String,
    /// The inside width in points; without it a window of widgets fits its
    /// content across, within `fit`.
    pub width: Option<f32>,
    /// The inside height in points; without it a window of widgets fits its
    /// content down, within `fit`.
    pub height: Option<f32>,
    pub resizable: bool,
    pub always_on_top: bool,
    /// Escape closes the window as its close button does, as a dialog's.
    pub close_on_escape: bool,
    /// The window is drawn as a dialog, inside a dialog's margin, as the
    /// ready-made windows are whether or not they have a button bar; a
    /// window with one is drawn so either way.
    pub dialog: bool,
    /// The bounds of a side fitted to the content.
    pub fit: Fit,
    pub theme: Theme,
    /// The window shows nothing: the egui renderer makes no viewport for
    /// it, and it is open as soon as it is made, as for a clip's sound
    /// played with no window. An HTML window shows as any other.
    pub hidden: bool,
}

impl WindowOptions {
    pub fn titled(title: impl Into<String>) -> WindowOptions {
        WindowOptions { title: title.into(), width: None, height: None, resizable: true, always_on_top: false, close_on_escape: false, dialog: false, fit: Fit::WINDOW, theme: Theme::System, hidden: false }
    }

    /// The inside size in points of a window of widgets whose content
    /// measures `content` across and down, on a screen whose work area is
    /// `work_area` when it is known: each side given, else the content's
    /// within `fit` and [`FIT_SHARE`] of the work area; no side larger than
    /// the work area.
    pub fn fitted(&self, content: [f32; 2], work_area: Option<[f32; 2]>) -> [f32; 2] {
        let given = [self.width, self.height];
        std::array::from_fn(|i| {
            let screen = work_area.map(|a| a[i]);
            match given[i] {
                Some(v) => screen.map_or(v, |s| v.min(s)),
                None => {
                    let most = screen.map_or(f32::INFINITY, |s| s * FIT_SHARE[i]).min(self.fit.max[i].unwrap_or(f32::INFINITY));
                    content[i].clamp(self.fit.min[i].min(most), most)
                }
            }
        })
    }

    /// The inside size in points of a page's window: each side given, else
    /// [`PAGE_SIZE`]'s; no side larger than the work area when it is known.
    pub fn page_size(&self, work_area: Option<[f32; 2]>) -> [f32; 2] {
        let given = [self.width, self.height];
        std::array::from_fn(|i| {
            let side = given[i].unwrap_or(PAGE_SIZE[i]);
            work_area.map_or(side, |a| side.min(a[i]))
        })
    }

    /// Whether a side is fitted to the content.
    pub fn fits(&self) -> bool {
        self.width.is_none() || self.height.is_none()
    }
}

/// What happened to a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventKind {
    Click,
    Change,
    Select,
    Activate,
    /// Enter pressed in a text field of one line.
    Submit,
    /// One of the window's own shortcuts, those in its root node's
    /// `shortcuts` (strings, as `Ctrl+S`), pressed; raised on the root node
    /// with the shortcut as written there.
    Shortcut,
    /// A tree item with `more` and no children opened; raised on its
    /// `TreeView` with the item's node id.
    Expand,
    /// Files or text dropped on a node whose `drop` is true, the window's
    /// root among them: the files' full paths, a list of strings, or the
    /// text, a string. A renderer whose window layer gives no dropped text
    /// raises files alone.
    Drop,
    /// A key pressed while a node whose `keys` is true has the focus, one
    /// the widget does not use itself, as a text field keeps its editing
    /// keys: written as a shortcut is, `Ctrl+K`, `F5` or `Enter`.
    Key,
    /// A pointer button pressed over a GpuSurface whose `pointerDown` is
    /// true: a map of where, `x` and `y` in physical pixels from its top
    /// left, `u` and `v` as fractions of its width and height, and `ppp`
    /// the display's pixels per point, with `button` (`Left`, `Right` or
    /// `Middle`) and the modifier keys pressed, `shift`, `ctrl` (Ctrl, or
    /// Command on macOS) and `alt`.
    PointerDown,
    /// A button pressed over a GpuSurface whose `pointerUp` is true,
    /// released wherever the pointer then is: the map PointerDown gives.
    PointerUp,
    /// The pointer moved over a GpuSurface whose `pointerMove` is true, or
    /// anywhere while a button pressed over it is still down, once a frame
    /// at most: the map PointerDown gives, `button` the one down or null.
    PointerMove,
    /// The wheel turned over a GpuSurface whose `wheel` is true: the map
    /// PointerDown gives, `button` null, and `notches`, positive when the
    /// wheel turns away from the person, a line or a page counting one and
    /// 100 points one.
    Wheel,
    /// A second click on a canvas item, whose canvas's `doubleClick` is
    /// true, soon after the first: the map a canvas event gives. The first
    /// click raises Click.
    DoubleClick,
    /// The pointer came over a canvas item whose canvas's `pointerEnter`
    /// is true, the topmost named one there: the map a canvas event gives.
    PointerEnter,
    /// The pointer left the canvas item it was over, whose canvas's
    /// `pointerLeave` is true, for another item or none: the map a canvas
    /// event gives, `item` naming the item left.
    PointerLeave,
    /// A button pressed over a canvas item, whose canvas's `dragStart` is
    /// true, still down while the pointer moved: the map a canvas event
    /// gives, at the point pressed.
    DragStart,
    /// The pointer moved during a drag, once a frame at most, for a canvas
    /// whose `dragMove` is true: the map a canvas event gives, at where the
    /// pointer is, `item` naming the item dragged.
    DragMove,
    /// The button of a drag released, for a canvas whose `dragEnd` is
    /// true: the map a canvas event gives, at where it was released, `item`
    /// naming the item dragged.
    DragEnd,
    /// A test's hold on a password field's peek button: its value true
    /// holds the button, as a person pressing it would, and false lets it
    /// go. Refused while the field does not offer the button
    /// ([`peek::Peek`]); no widget raises it.
    Peek,
    /// A MediaPlayer's clip is open: [`media::loaded_value`].
    Loaded,
    /// A MediaPlayer started or went on playing: the position in seconds.
    Playing,
    /// A MediaPlayer paused: the position in seconds.
    Paused,
    /// A MediaPlayer played to the clip's end: the position in seconds.
    Ended,
    /// A MediaPlayer whose `position` is true moved on, every
    /// `position_interval` seconds while it plays, after a seek and at the
    /// clip's end: the position in seconds.
    Position,
    /// A MediaPlayer's clip cannot be played: [`media::error_value`].
    Error,
}

/// Why a window closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseReason {
    /// The person closed it.
    User,
    /// The session closed it.
    Program,
    /// The helper is shutting down.
    Shutdown,
    /// The window's page could not be shown: its webview could not be
    /// made, or the engine drawing it ended and the page loaded again ended
    /// too before it could draw. The `Error` before `Closed` says why.
    Failed,
}

/// How a cell sorts: an amount by its value, standing at the right of its
/// column as PowerShell's tables place amounts; a date by its ticks,
/// standing at the left as text does; text without regard to case; and an
/// empty cell last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SortKey {
    Number(f64),
    Date(f64),
    Text(String),
    Empty,
}

/// One table cell: the text shown and the key it sorts by.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub text: String,
    pub sort: SortKey,
}

/// One table row, its cells in column order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub cells: Vec<Cell>,
}

/// A change to an open window's tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PatchOp {
    /// Sets properties of node `node`, leaving those it does not name as
    /// they are.
    SetProps { node: NodeId, props: BTreeMap<String, Value> },
    /// Puts `node` among the children of `parent`, at `index`, or last
    /// when `index` is past them.
    Insert { parent: NodeId, index: u32, node: Node },
    /// Takes node `node` and its children out of the tree.
    Remove { node: NodeId },
    /// Puts `node` in the place of the node with its id.
    Replace { node: Node },
    /// Adds `values` to the end of the list in property `prop` of node
    /// `node`, making the list when the node has none, so a log's new
    /// lines travel without the lines it has.
    Append { node: NodeId, prop: String, values: Vec<Value> },
    /// Takes the rows at `rows`, their places in row order before any is
    /// taken, out of the table `node`; the rows after them move up, and of
    /// the rows that stay, those selected remain selected.
    RemoveRows { node: NodeId, rows: Vec<u64> },
    /// Changes the properties of the canvas `node`'s item named `item`, as
    /// [`canvas::Board::set`] does: a property set directly ends any
    /// animation of it, and an `animate` among `props` starts the
    /// animations it gives.
    SetItem { node: NodeId, item: String, props: BTreeMap<String, Value> },
    /// Adds `items`, item maps, to the canvas `node` above the items it
    /// has, or into the Group `group` names above its own, as
    /// [`canvas::Board::add`] does.
    AddItems { node: NodeId, group: Option<String>, items: Vec<Value> },
    /// Takes the items of the canvas `node` that `names` name out of it,
    /// a Group with its items, as [`canvas::Board::remove`] does.
    RemoveItems { node: NodeId, names: Vec<String> },
}

/// Takes the items at `gone`, their places before any is taken, out of
/// `items`, as [`PatchOp::RemoveRows`] takes a table's rows, and returns
/// each item's new place by its old one, `None` for an item taken, so a
/// table's selection moves with its rows. A place past the items is passed
/// over.
pub fn remove_rows<T>(items: &mut Vec<T>, gone: &[u64]) -> Vec<Option<usize>> {
    let gone: std::collections::BTreeSet<u64> = gone.iter().copied().collect();
    let mut moved = Vec::with_capacity(items.len());
    let mut at = 0u64;
    let mut kept = 0usize;
    items.retain(|_| {
        let stays = !gone.contains(&at);
        at += 1;
        if stays {
            moved.push(Some(kept));
            kept += 1;
        } else {
            moved.push(None);
        }
        stays
    });
    moved
}

/// Base64 `text` as bytes, in the standard or the URL-safe alphabet, white
/// space passed over and padding optional; refused with the first
/// character of neither.
pub fn base64_bytes(text: &str) -> Result<Vec<u8>, char> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut held = 0u32;
    let mut bits = 0;
    for c in text.chars().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => break,
            other => return Err(other),
        };
        held = (held << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((held >> bits) as u8);
            held &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// A GPU adapter a renderer's backend finds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Adapter {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
    pub vendor: u32,
    pub device: u32,
}

/// Rendered pixels, RGBA, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A window's tree, the current value of each input node, its pixels when
/// requested, where each named node was drawn, and in a webview the engine
/// drawing it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub tree: Node,
    pub state: BTreeMap<NodeId, Value>,
    pub pixels: Option<Image>,
    /// The box each named node was last drawn in; a node not drawn, as one
    /// on a tab not shown, has none.
    pub bounds: BTreeMap<NodeId, Bounds>,
    /// The engine of a window drawn in a webview; `None` in egui.
    pub engine: Option<Engine>,
    /// The node that has the keyboard focus; `None` when no widget of the
    /// window has it.
    pub focused: Option<NodeId>,
    /// The text the window last put on the clipboard, as a copy of a
    /// table's rows or a field's selection; `None` until it has put any.
    pub copied: Option<String>,
    /// The sound each MediaPlayer handed the system, by node; `None` for a
    /// player whose renderer cannot measure it.
    pub sound: BTreeMap<NodeId, Option<media::Sound>>,
}

/// A box in a window, in points from the top left of its inside.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The engine a window's webview runs in, as the engine reports itself:
/// its name, `WebView2`, `WebKit` or `WebKitGTK`, its version, the folder
/// its code was loaded from, and each setting of its chrome the web helper
/// sets, read back from the webview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Engine {
    pub name: String,
    pub version: String,
    pub folder: Option<String>,
    pub settings: BTreeMap<String, Value>,
}

/// What a renderer draws with, as its `Hello` reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    /// The GPU backend, such as `Dx12`, `Vulkan`, `Metal` or `Gl`.
    pub backend: String,
    /// The adapter's name.
    pub adapter: String,
    /// The media backend; `None` in a renderer that plays no media.
    pub media: Option<String>,
}

/// What an HTML renderer's pages draw GpuSurfaces with, which a page
/// settles the first time a surface needs it: WebGPU when the engine's
/// WebGPU gives an adapter, and WebGL 2 otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebGraphics {
    WebGpu,
    WebGl2,
}

impl WebGraphics {
    /// The name a page reports and PowerShell gives it.
    pub fn name(self) -> &'static str {
        match self {
            WebGraphics::WebGpu => "WebGPU",
            WebGraphics::WebGl2 => "WebGL2",
        }
    }

    /// The graphics named `name`, as [`name`](Self::name) writes it.
    pub fn parse(name: &str) -> Option<WebGraphics> {
        [WebGraphics::WebGpu, WebGraphics::WebGl2].into_iter().find(|g| g.name() == name)
    }
}

/// An open window, as [`FromHelper::Windows`] lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSummary {
    pub window: WindowId,
    pub title: String,
    /// The names of the window's nodes, in tree order.
    pub names: Vec<String>,
}

/// What a native dialog is for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DialogKind {
    /// A message, its icon, and the buttons it is answered with.
    Message { text: String, icon: MessageIcon, buttons: MessageButtons },
    /// Files to open: one, or with `multiple` any number, of the kinds
    /// `filters` names, each a name and its extensions without dots, `*`
    /// for any file; starting in `directory`.
    Open { multiple: bool, filters: Vec<(String, Vec<String>)>, directory: Option<String> },
    /// A file to save, of the kinds `filters` names, starting in
    /// `directory` with `file_name` filled in.
    Save { filters: Vec<(String, Vec<String>)>, directory: Option<String>, file_name: Option<String> },
    /// A folder, or with `multiple` any number, starting in `directory`.
    Folder { multiple: bool, directory: Option<String> },
}

impl DialogKind {
    /// The kind's name, as Get-GuiDialog lists it.
    pub fn name(&self) -> &'static str {
        match self {
            DialogKind::Message { .. } => "Message",
            DialogKind::Open { .. } => "Open",
            DialogKind::Save { .. } => "Save",
            DialogKind::Folder { .. } => "Folder",
        }
    }
}

/// A message dialog's icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageIcon {
    Info,
    Warning,
    Error,
    Question,
}

/// The buttons a message dialog is answered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageButtons {
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
}

/// One of a message dialog's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageButton {
    Ok,
    Cancel,
    Yes,
    No,
}

impl MessageButtons {
    /// The buttons, in the order the dialog shows them.
    pub fn buttons(self) -> &'static [MessageButton] {
        match self {
            MessageButtons::Ok => &[MessageButton::Ok],
            MessageButtons::OkCancel => &[MessageButton::Ok, MessageButton::Cancel],
            MessageButtons::YesNo => &[MessageButton::Yes, MessageButton::No],
            MessageButtons::YesNoCancel => &[MessageButton::Yes, MessageButton::No, MessageButton::Cancel],
        }
    }
}

/// How a dialog was answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DialogAnswer {
    /// A message dialog's button.
    Button(MessageButton),
    /// The paths a file or folder dialog chose.
    Paths(Vec<String>),
    /// The dialog was canceled or closed.
    Canceled,
}

/// Why a dialog gave no answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DialogFailure {
    /// The system has no dialog of the kind requested, as where there is
    /// neither an XDG portal nor zenity; the egui renderer's own stands in.
    NoNative(String),
    /// The dialog could not open, or it ended without an answer.
    Failed(String),
}

/// An open dialog, as [`FromHelper::Dialogs`] lists it: the request that
/// opened it, its kind's name and its title.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DialogSummary {
    pub dialog: RequestId,
    pub kind: String,
    pub title: String,
}

/// From a session to a renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToHelper {
    /// Opens a window whose events go to the window ring `ring`, its tables
    /// filled with `rows` from their first row, by each table's node, so a
    /// window that fits its content is measured with them.
    OpenWindow { window: WindowId, ring: RingName, options: WindowOptions, tree: Node, rows: Vec<(NodeId, Vec<Row>)> },
    Patch { window: WindowId, ops: Vec<PatchOp> },
    /// Table rows in input order, starting at row `start`; `added` when
    /// Add-GuiItem added them, which a table showing its last row follows,
    /// rather than a grid view's pipeline bringing them as it opens.
    Rows { window: WindowId, node: NodeId, start: u64, rows: Vec<Row>, added: bool },
    /// Closes the window if it is open, with no reply.
    CloseWindow { window: WindowId },
    /// Closes the window; the reply on `reply` is [`FromHelper::CloseDone`],
    /// a refusal when the renderer has no window of that number open.
    CloseOpen { request: RequestId, reply: RingName, window: WindowId },
    /// Gives the widget `node` of the window the keyboard focus. The reply on
    /// `reply`, a [`FromHelper::FocusDone`], comes once the widget has the
    /// focus, so a snapshot taken after it names the widget; it refuses a
    /// window that is not open, a node the window does not have, a node that
    /// takes no focus or is disabled, and a node the window has not drawn,
    /// as one on a tab not shown.
    FocusNode { request: RequestId, reply: RingName, window: WindowId, node: NodeId },
    /// No command reads this window's events any more; the renderer stops
    /// sending them and leaves the window open.
    Detach { window: WindowId },
    Focus { window: WindowId },
    ListAdapters { request: RequestId, reply: RingName },
    ListWindows { request: RequestId, reply: RingName },
    /// What the renderer's pages draw GpuSurfaces with; the reply on `reply`
    /// is [`FromHelper::PageGraphics`].
    PageGraphics { request: RequestId, reply: RingName },
    /// Acts on the node named `name` as a person would: the event goes to
    /// the window's ring, and the outcome to `reply`.
    TestEvent { request: RequestId, reply: RingName, window: WindowId, name: String, kind: EventKind, value: Value },
    /// Gives the window `steps` of a person's input, a frame apart, through
    /// the renderer's own input ([`test_input`]). As each step begins, a
    /// [`FromHelper::InputStep`] on `reply` says so. The reply on `reply`, a
    /// [`FromHelper::TestEventDone`], comes once the frame after the last
    /// step has run. It refuses a window that is not open, a point outside
    /// its inside, and steps left when it closes.
    TestInput { request: RequestId, reply: RingName, window: WindowId, steps: Vec<test_input::Step> },
    /// Ends the test's input of request `request` that window `window` is
    /// taking, which the session no longer waits for: the steps not yet
    /// given are dropped, with no reply.
    StopInput { window: WindowId, request: RequestId },
    /// The window's tree and state, and with `pixels` its rendered image,
    /// of the whole window or of the node named `name`.
    Snapshot { request: RequestId, reply: RingName, window: WindowId, name: Option<String>, pixels: bool },
    /// The reply ring `reply` is gone; the renderer forgets it.
    ForgetReply { reply: RingName },
    /// A value for the `window.pwrs.on` handlers of a window's page, as
    /// JSON; the reply on `reply` is [`FromHelper::BridgeDone`].
    Bridge { request: RequestId, reply: RingName, window: WindowId, json: String },
    /// The reply to the page's `window.pwrs.send` numbered `call`, as JSON,
    /// which that send's Promise resolves with.
    BridgeReply { window: WindowId, call: u32, json: String },
    /// Closes every window and ends the renderer.
    Shutdown,
    /// Shows a native dialog titled `title`; the reply on `reply`,
    /// [`FromHelper::DialogResult`], comes once it closes, and `request`
    /// names the dialog while it is open.
    Dialog { request: RequestId, reply: RingName, title: String, kind: DialogKind },
    /// Closes the dialog `dialog`, whose command stopped, the platform's
    /// own way; the dialog's result is [`DialogAnswer::Canceled`].
    CloseDialog { dialog: RequestId },
    /// The dialogs open now; the reply is [`FromHelper::Dialogs`].
    ListDialogs { request: RequestId, reply: RingName },
    /// Closes the dialog `dialog` as [`ToHelper::CloseDialog`] does, its
    /// command getting `answer` as a person's answer; the outcome goes to
    /// `reply` as [`FromHelper::TestEventDone`].
    AnswerDialog { request: RequestId, reply: RingName, dialog: RequestId, answer: DialogAnswer },
    /// The answer of file chooser `node`'s dialog in window `window`, which
    /// the window's command had another renderer draw, as
    /// [`FromHelper::PickerWanted`] requested; the chooser takes it as a
    /// person's choice.
    PickerAnswer { window: WindowId, node: NodeId, outcome: Result<DialogAnswer, DialogFailure> },
    /// A call on the renderer's GPU layer, run on its GPU thread, the reply
    /// on `reply` a [`FromHelper::GpuResult`]; only the egui renderer has
    /// one.
    Gpu { request: RequestId, reply: RingName, op: gpu::GpuOp },
    /// Frees GPU buffer `buffer`, whose object the session has let go of,
    /// with no reply; a buffer the layer does not have is reported to the
    /// renderer's log, and a surface drawing with it goes on using it until
    /// its draws change.
    GpuFree { buffer: u64 },
    /// Frees render pipeline `pipeline`, whose object the session has let go
    /// of, with no reply, as [`ToHelper::GpuFree`] frees a buffer; a surface
    /// drawing with it goes on using it until its draws change.
    GpuFreePipeline { pipeline: u64 },
    /// Frees texture `texture`, whose object the session has let go of, as
    /// [`ToHelper::GpuFree`] frees a buffer.
    GpuFreeTexture { texture: u64 },
    /// Frees sampler `sampler`, whose object the session has let go of, as
    /// [`ToHelper::GpuFree`] frees a buffer.
    GpuFreeSampler { sampler: u64 },
    /// A call on MediaPlayer `node` of window `window`; the reply on
    /// `reply` is [`FromHelper::MediaDone`], the player's state once the
    /// call is made, or why it could not be.
    Media { request: RequestId, reply: RingName, window: WindowId, node: NodeId, op: media::MediaOp },
    /// Reads the headers of the clip at `source`, a file's path or an https
    /// address, through the renderer's loader, before a window shows it;
    /// the reply on `reply` is [`FromHelper::Probed`].
    Probe { request: RequestId, reply: RingName, source: String },
}

/// From a renderer to a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FromHelper {
    /// The renderer has started. `notes` are what the person should hear
    /// about its start: a setting this platform cannot honor, a monitor
    /// that is not connected, or data of an ended process that could not
    /// be removed.
    Hello { protocol: u32, build: String, renderer: String, capabilities: Capabilities, notes: Vec<String> },
    /// The renderer could not start; on the start ring in place of `Hello`.
    StartFailed { text: String },
    /// After a window's first frame.
    Opened { window: WindowId },
    /// `at` is when the renderer took the input, in nanoseconds on
    /// [`clock::now_ns`].
    Event { window: WindowId, node: NodeId, kind: EventKind, value: Value, at: u64 },
    Closed { window: WindowId, reason: CloseReason },
    /// A value a window's page sent with `window.pwrs.send`, as JSON. The
    /// session replies to every one with a [`ToHelper::BridgeReply`] that
    /// gives `call`, the page's number for the send.
    BridgeMessage { window: WindowId, call: u32, json: String },
    /// A window's page closed it with `window.pwrs.close(value)`; `json`
    /// is the value, `null` when it gave none. The window's last message,
    /// in place of `Closed`.
    PageClosed { window: WindowId, json: String },
    Adapters { request: RequestId, adapters: Vec<Adapter> },
    Windows { request: RequestId, windows: Vec<WindowSummary> },
    /// What the renderer's pages draw GpuSurfaces with, as the last page to
    /// settle it reported; `None` before any has, and from a renderer that
    /// shows no pages.
    PageGraphics { request: RequestId, graphics: Option<WebGraphics> },
    /// The value the widget took, or why the event could not be raised.
    TestEventDone { request: RequestId, outcome: Result<Value, String> },
    /// The test's input of [`ToHelper::TestInput`] request `request` begins
    /// its step `step`, counted from 1, the steps before it taken and seen.
    InputStep { request: RequestId, step: u32 },
    /// Whether a [`ToHelper::Bridge`] value went to the page: refused for a
    /// window that is not open or shows no page of its own, or a value that
    /// is not JSON.
    BridgeDone { request: RequestId, outcome: Result<(), String> },
    /// Whether the window a [`ToHelper::CloseOpen`] names was open and is
    /// closing: refused for a number the renderer has no open window of.
    CloseDone { request: RequestId, outcome: Result<(), String> },
    /// Whether the widget a [`ToHelper::FocusNode`] names was given the
    /// focus, or why the renderer refused it.
    FocusDone { request: RequestId, outcome: Result<(), String> },
    /// The window's snapshot, or why it could not be read.
    Snapshot { request: RequestId, outcome: Result<Snapshot, String> },
    Log { level: LogLevel, text: String },
    /// Something the renderer could not do; `window` names the window it
    /// concerns, so it reaches the command waiting on that window.
    Error { window: Option<WindowId>, context: String, text: String },
    /// A dialog closed: how it was answered, or why it gave no answer.
    DialogResult { request: RequestId, outcome: Result<DialogAnswer, DialogFailure> },
    Dialogs { request: RequestId, dialogs: Vec<DialogSummary> },
    /// File chooser `node` of window `window` needs a dialog this renderer
    /// cannot show, the system having none of its kind; the window's
    /// command has the egui renderer draw it and sends the person's answer
    /// back with [`ToHelper::PickerAnswer`].
    PickerWanted { window: WindowId, node: NodeId, title: String, kind: DialogKind },
    /// The reply to a [`ToHelper::Gpu`] call, or why it failed.
    GpuResult { request: RequestId, outcome: Result<gpu::GpuValue, gpu::GpuFailure> },
    /// The renderer's GPU device was lost for `reason`; the renderer goes
    /// on with a new one, its buffers filled again from what the session
    /// wrote into them.
    DeviceLost { reason: String },
    /// A file window `window` watches changed: `file` is its place among
    /// the files the window watches, from 0, and `length` its length in
    /// bytes, none when it is gone. The window's own session sends it into
    /// the window's events from its file watch; no renderer sends it.
    FileChanged { window: WindowId, file: u32, length: Option<u64> },
    /// The reply to a [`ToHelper::Media`] call: the player's state once the
    /// call is made, or why it could not be.
    MediaDone { request: RequestId, outcome: Result<media::MediaState, String> },
    /// The reply to a [`ToHelper::Probe`]: the clip's format as its
    /// headers name it, or why they name none.
    Probed { request: RequestId, outcome: Result<media::probe::Format, String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Verbose,
    Warning,
}

impl FromHelper {
    /// A line for a log: a `Log` as its level and text, an `Error` as its
    /// context and text, and any other message in its debug form.
    pub fn log_line(&self) -> String {
        match self {
            FromHelper::Log { level: LogLevel::Verbose, text } => format!("verbose: {text}"),
            FromHelper::Log { level: LogLevel::Warning, text } => format!("warning: {text}"),
            FromHelper::Error { context, text, .. } => format!("error in {context}: {text}"),
            other => format!("{other:?}"),
        }
    }

    /// Whether the message, sent on a request's reply ring, is the reply
    /// that ends the request's wait: every one is but
    /// [`FromHelper::InputStep`], which reports on a test's input under way.
    pub fn ends_request(&self) -> bool {
        !matches!(self, FromHelper::InputStep { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shortcut_reads_its_modifiers_in_any_order_and_writes_them_in_one() {
        let s = Shortcut::parse("shift+CTRL+s").expect("a shortcut");
        assert_eq!((s.ctrl, s.shift, s.alt, s.key.as_str()), (true, true, false, "S"));
        assert_eq!(s.written(), "Ctrl+Shift+S");
        assert_eq!(Shortcut::parse("f5").expect("an F key").written(), "F5");
        assert_eq!(Shortcut::parse("Ctrl+f12").expect("an F key").written(), "Ctrl+F12");
        assert_eq!(Shortcut::parse("Alt+arrowup").expect("a named key").written(), "Alt+ArrowUp");
        assert_eq!(Shortcut::parse("ctrl+plus").expect("the key typing +").written(), "Ctrl+Plus");
        assert_eq!(Shortcut::parse("Minus").expect("the key typing -").written(), "Minus");
        assert!(Shortcut::parse("Ctrl+Meta+S").is_err());
        assert!(Shortcut::parse("Ctrl+F25").is_err());
        let refused = Shortcut::parse("F01").expect_err("F01 names no key");
        assert!(refused.contains("\"F01\""), "{refused}");
    }

    #[test]
    fn a_text_field_keeps_its_editing_keys_and_no_others() {
        let keeps = |written: &str| Shortcut::parse(written).expect("a shortcut").field_keeps();
        for kept in ["A", "Shift+A", "7", "Space", "Delete", "Enter", "Escape", "ArrowLeft", "Ctrl+A", "Ctrl+C", "Ctrl+V", "Ctrl+Shift+Z", "Ctrl+ArrowLeft", "Ctrl+Backspace"] {
            assert!(keeps(kept), "a focused field keeps {kept}");
        }
        for acts in ["Ctrl+S", "Ctrl+Shift+N", "Ctrl+Enter", "F5", "Shift+F10", "Alt+N", "Ctrl+Alt+A"] {
            assert!(!keeps(acts), "{acts} acts from a focused field");
        }
    }

    #[test]
    fn each_widget_keeps_the_keys_that_work_it_and_raises_the_rest() {
        let keeps = |written: &str, kind: Kind, typed: bool| Shortcut::parse(written).expect("a shortcut").widget_keeps(kind, typed);
        for (written, kind) in [("Tab", Kind::Button), ("Shift+Tab", Kind::Slider), ("Space", Kind::Button), ("Enter", Kind::CheckBox), ("Shift+ArrowDown", Kind::ListBox), ("Ctrl+A", Kind::Table), ("Ctrl+C", Kind::Table), ("PageUp", Kind::Slider), ("ArrowRight", Kind::RadioGroup), ("Escape", Kind::ComboBox), ("Escape", Kind::DatePicker)] {
            assert!(keeps(written, kind, false), "a {kind:?} keeps {written}");
        }
        for (written, kind) in [("A", Kind::Button), ("Ctrl+K", Kind::ListBox), ("Ctrl+C", Kind::ListBox), ("Ctrl+Shift+C", Kind::Table), ("F5", Kind::TreeView), ("Delete", Kind::Table), ("Ctrl+ArrowUp", Kind::Slider), ("Escape", Kind::Button), ("Shift+Space", Kind::CheckBox), ("Ctrl+Tab", Kind::Button)] {
            assert!(!keeps(written, kind, false), "a {kind:?} raises {written}");
        }
        assert!(keeps("A", Kind::TextBox, true), "a text field keeps what it types");
        assert!(!keeps("Ctrl+K", Kind::NumberBox, true), "a number field raises a key it does not edit with");
    }

    #[test]
    fn a_password_field_keeps_alt_f8_which_shows_its_password() {
        let peek = Shortcut::parse("Alt+F8").expect("a shortcut");
        let password = Node::new(1, Kind::TextBox).prop("password", Value::Bool(true));
        let plain = Node::new(2, Kind::TextBox).prop("password", Value::Bool(false));
        assert!(peek.is_peek() && password.is_password() && !plain.is_password());
        assert!(peek.node_keeps(&password, true), "a password field keeps Alt+F8");
        assert!(!peek.node_keeps(&plain, true), "a field that is no password field raises Alt+F8");
        assert!(!Shortcut::parse("Alt+Shift+F8").expect("a shortcut").node_keeps(&password, true), "Alt+F8 alone shows the password");
        assert!(Shortcut::parse("A").expect("a shortcut").node_keeps(&password, true), "a password field keeps what it types");
    }

    fn tree() -> Node {
        Node::new(1, Kind::Column)
            .child(Node::new(2, Kind::Text).prop("text", Value::Str("Pick one".into())))
            .child(
                Node::new(3, Kind::ListBox)
                    .named("Choice")
                    .prop("items", Value::List(vec![Value::Str("a".into()), Value::Str("b".into())])),
            )
            .child(Node::new(4, Kind::Row).child(Node::new(5, Kind::Button).named("Ok")))
    }

    #[test]
    fn nodes_are_found_by_id_and_by_name_without_regard_to_case() {
        let t = tree();
        assert_eq!(t.find(3).map(|n| n.kind), Some(Kind::ListBox));
        assert_eq!(t.find_named("choice").map(|n| n.id), Some(3));
        assert!(t.find(9).is_none());
    }

    #[test]
    fn a_tree_lists_its_names_in_tree_order() {
        assert_eq!(tree().names(), vec!["Choice".to_string(), "Ok".to_string()]);
    }

    #[test]
    fn rows_taken_move_the_rest_up_and_say_where_each_went() {
        let mut rows = vec!["a", "b", "c", "d", "e"];
        let moved = remove_rows(&mut rows, &[3, 1, 9]);
        assert_eq!(rows, ["a", "c", "e"]);
        assert_eq!(moved, [Some(0), None, Some(1), None, Some(2)]);
        let mut none: Vec<u8> = Vec::new();
        assert!(remove_rows(&mut none, &[0]).is_empty());
    }

    #[test]
    fn base64_reads_both_alphabets_with_or_without_padding() {
        assert_eq!(base64_bytes("+/8A"), Ok(vec![0xFB, 0xFF, 0x00]));
        assert_eq!(base64_bytes("-_8A"), Ok(vec![0xFB, 0xFF, 0x00]), "the URL-safe alphabet");
        assert_eq!(base64_bytes("TWE="), base64_bytes("TWE"), "padding is optional");
        assert_eq!(base64_bytes("TW\nE="), Ok(b"Ma".to_vec()), "white space is passed over");
        assert_eq!(base64_bytes("TW*E"), Err('*'));
    }

    #[test]
    fn messages_survive_postcard() {
        let sent = vec![
            ToHelper::OpenWindow {
                window: 7,
                ring: "r".into(),
                options: WindowOptions::titled("t"),
                tree: tree(),
                rows: vec![(3, vec![Row { cells: vec![Cell { text: "a".into(), sort: SortKey::Text("a".into()) }] }])],
            },
            ToHelper::Rows {
                window: 7,
                node: 3,
                start: 0,
                rows: vec![Row { cells: vec![Cell { text: "1.5".into(), sort: SortKey::Number(1.5) }] }],
                added: true,
            },
            ToHelper::Patch { window: 7, ops: vec![PatchOp::RemoveRows { node: 3, rows: vec![0, 2] }] },
            ToHelper::FocusNode { request: 5, reply: "q".into(), window: 7, node: 3 },
            ToHelper::TestInput {
                request: 6,
                reply: "q".into(),
                window: 7,
                steps: vec![
                    test_input::Step::Press { button: test_input::Button::Right, at: [40.0, 12.5], keys: test_input::Keys { shift: false, ctrl: true, alt: false } },
                    test_input::Step::Wait { seconds: 0.9 },
                    test_input::Step::Key { key: String::from(test_input::MENU), keys: test_input::Keys::default() },
                ],
            },
            ToHelper::StopInput { window: 7, request: 6 },
            ToHelper::Shutdown,
        ];
        for m in &sent {
            let bytes = postcard::to_stdvec(m).expect("encoding a message");
            let back: ToHelper = postcard::from_bytes(&bytes).expect("decoding a message");
            assert_eq!(&back, m);
        }
        let done = FromHelper::TestEventDone { request: 3, outcome: Err("no node named X".into()) };
        let bytes = postcard::to_stdvec(&done).expect("encoding a reply");
        assert_eq!(postcard::from_bytes::<FromHelper>(&bytes).expect("decoding a reply"), done);
        let begun = FromHelper::InputStep { request: 6, step: 2 };
        let bytes = postcard::to_stdvec(&begun).expect("encoding a step begun");
        assert_eq!(postcard::from_bytes::<FromHelper>(&bytes).expect("decoding a step begun"), begun);
        let graphics = FromHelper::PageGraphics { request: 4, graphics: Some(WebGraphics::WebGl2) };
        let bytes = postcard::to_stdvec(&graphics).expect("encoding a reply");
        assert_eq!(postcard::from_bytes::<FromHelper>(&bytes).expect("decoding a reply"), graphics);
        let focused = FromHelper::FocusDone { request: 5, outcome: Err("the widget is disabled".into()) };
        let bytes = postcard::to_stdvec(&focused).expect("encoding a reply");
        assert_eq!(postcard::from_bytes::<FromHelper>(&bytes).expect("decoding a reply"), focused);
        for length in [Some(4096), None] {
            let changed = FromHelper::FileChanged { window: 6, file: 1, length };
            let bytes = postcard::to_stdvec(&changed).expect("encoding a file change");
            assert_eq!(postcard::from_bytes::<FromHelper>(&bytes).expect("decoding a file change"), changed);
        }
        let call = ToHelper::Media { request: 8, reply: "q".into(), window: 7, node: 2, op: media::MediaOp::Seek { seconds: 1.5 } };
        let bytes = postcard::to_stdvec(&call).expect("encoding a media call");
        assert_eq!(postcard::from_bytes::<ToHelper>(&bytes).expect("decoding a media call"), call);
        let state = media::MediaState { phase: media::Phase::Paused, position: 1.5, duration: Some(2.0), rate: 1.0, volume: 0.5, muted: false, looped: true, video: Some([320, 240]), audio: true, error: None };
        let done = FromHelper::MediaDone { request: 8, outcome: Ok(state) };
        let bytes = postcard::to_stdvec(&done).expect("encoding a media reply");
        assert_eq!(postcard::from_bytes::<FromHelper>(&bytes).expect("decoding a media reply"), done);
    }

    #[test]
    fn the_focus_goes_to_inputs_buttons_links_and_widgets_taking_keys() {
        let link = |style: &str| Node::new(1, Kind::Text).prop("style", Value::Str(style.into()));
        assert!(link("link").takes_focus());
        assert!(!link("heading").takes_focus(), "a Text takes the focus only as a link");
        assert!(Node::new(1, Kind::Table).takes_focus());
        assert!(!Node::new(1, Kind::Row).takes_focus());
        assert!(!Node::new(1, Kind::GpuSurface).takes_focus(), "a GpuSurface takes the focus only with keys");
        assert!(Node::new(1, Kind::GpuSurface).prop("keys", Value::Bool(true)).takes_focus());
        assert!(Node::new(1, Kind::Button).prop("enabled", Value::Bool(false)).disabled());
        assert!(!Node::new(1, Kind::Button).disabled(), "a widget is enabled unless it says otherwise");
        assert_eq!(Node::new(1, Kind::Button).focus_refusal(), None);
        assert_eq!(Node::new(1, Kind::Button).prop("enabled", Value::Bool(false)).focus_refusal().as_deref(), Some("the widget is disabled"));
        assert_eq!(Node::new(1, Kind::ProgressBar).focus_refusal().as_deref(), Some("a ProgressBar takes no focus"));
        assert_eq!(link("body").focus_refusal().as_deref(), Some("a Text takes the focus only when its style is link"));
    }

    #[test]
    fn the_focus_holder_is_the_innermost_widget_taking_the_focus() {
        let t = tree();
        assert_eq!(t.focus_holder(5), Some(5), "the button inside the row");
        assert_eq!(t.focus_holder(3), Some(3));
        assert_eq!(t.focus_holder(4), None, "a row takes no focus, and neither does the column around it");
        assert_eq!(t.focus_holder(2), None, "a Text that is not a link");
        assert_eq!(t.focus_holder(9), None, "the tree has no node 9");
        let items = Node::new(1, Kind::Column).child(Node::new(2, Kind::TreeView).child(Node::new(3, Kind::TreeItem)));
        assert_eq!(items.focus_holder(3), Some(2), "a tree's item gives the focus to its tree");
    }

    #[test]
    fn a_page_s_graphics_read_back_from_the_names_pages_report() {
        for g in [WebGraphics::WebGpu, WebGraphics::WebGl2] {
            assert_eq!(WebGraphics::parse(g.name()), Some(g));
        }
        assert_eq!(WebGraphics::WebGpu.name(), "WebGPU");
        assert_eq!(WebGraphics::parse("webgpu"), None, "pages report the names exactly");
    }

    #[test]
    fn a_log_line_is_its_level_or_context_and_its_text() {
        let warning = FromHelper::Log { level: LogLevel::Warning, text: "window 3's page was loaded again".into() };
        assert_eq!(warning.log_line(), "warning: window 3's page was loaded again");
        let error = FromHelper::Error { window: None, context: "attach".into(), text: "no ring".into() };
        assert_eq!(error.log_line(), "error in attach: no ring");
        assert_eq!(FromHelper::Opened { window: 2 }.log_line(), "Opened { window: 2 }");
    }

    #[test]
    fn a_window_fits_its_content_within_its_bounds_and_the_work_area() {
        let screen = Some([1920.0, 1040.0]);
        let dialog = WindowOptions { fit: Fit::DIALOG, ..WindowOptions::titled("d") };
        assert_eq!(dialog.fitted([200.0, 120.0], screen), [320.0, 120.0], "a small dialog takes the least width");
        assert_eq!(dialog.fitted([700.0, 900.0], screen), [548.0, 756.0], "a large one its most");
        assert_eq!(dialog.fitted([700.0, 900.0], Some([1280.0, 720.0])), [548.0, 576.0], "never past 80 percent of the work area down");
        let data = WindowOptions::titled("g");
        assert_eq!(data.fitted([2400.0, 300.0], screen), [1728.0, 300.0], "data takes up to 90 percent across");
        assert_eq!(data.fitted([2400.0, 300.0], None), [2400.0, 300.0], "with no work area read, only the fit's bounds limit it");
        assert_eq!(data.fitted([100.0, 40.0], screen), [320.0, 40.0], "a window of widgets is 320 across at least");
        let sized = WindowOptions { width: Some(380.0), ..WindowOptions::titled("p") };
        assert_eq!(sized.fitted([200.0, 90.0], screen), [380.0, 90.0], "a side given stays as given");
        assert!(sized.fits() && !WindowOptions { height: Some(1.0), ..sized.clone() }.fits());
        let huge = WindowOptions { width: Some(5000.0), height: Some(5000.0), ..WindowOptions::titled("h") };
        assert_eq!(huge.fitted([1.0, 1.0], screen), [1920.0, 1040.0], "and cut to the work area");
        assert_eq!(WindowOptions::titled("page").page_size(screen), PAGE_SIZE);
        assert_eq!(WindowOptions::titled("page").page_size(Some([640.0, 480.0])), [640.0, 480.0]);
    }

    #[test]
    fn align_reads_its_four_words_and_refuses_any_other() {
        let with = |v: Value| Node::new(1, Kind::Column).prop("align", v);
        assert_eq!(Align::of(&Node::new(1, Kind::Row)), Ok(Align::Start));
        assert_eq!(Align::of(&with(Value::Str("stretch".into()))), Ok(Align::Stretch));
        assert_eq!(Align::of(&with(Value::Str("end".into()))), Ok(Align::End));
        assert!(Align::of(&with(Value::Str("right".into()))).is_err());
        assert!(Align::of(&with(Value::Int(2))).is_err());
    }

    #[test]
    fn lists_unsized_views_and_their_holders_fill_down_and_a_row_s_spacer_does_not() {
        let list = Node::new(2, Kind::ListBox);
        let text = Node::new(3, Kind::Text);
        let fill = Node::new(4, Kind::Spacer).prop("fill", Value::Bool(true));
        assert!(fills_down(&list));
        assert!(!fills_down(&text));
        assert!(fills_down(&fill));
        assert!(fills_down(&Node::new(1, Kind::Column).child(text.clone()).child(list.clone())));
        assert!(!fills_down(&Node::new(1, Kind::Row).child(text.clone()).child(fill)), "a Row's Spacer fills across");
        assert!(!fills_down(&Node::new(5, Kind::LogView).prop("height", Value::Float(60.0))), "a log of its own height");
        assert!(fills_down(&Node::new(5, Kind::LogView)));
        assert!(fills_down(&Node::new(6, Kind::Split).prop("orientation", Value::Str("vertical".into()))));
        assert!(!fills_down(&Node::new(6, Kind::Split).child(text.clone()).child(text)));
        assert!(fills_down(&Node::new(7, Kind::MediaPlayer)), "a player's picture takes the height left");
        assert!(!fills_down(&Node::new(7, Kind::MediaPlayer).prop("audio_only", Value::Bool(true))), "a player of sound alone shows no picture");
        assert!(!fills_down(&Node::new(7, Kind::MediaPlayer).prop("height", Value::Float(180.0))));
    }

    #[test]
    fn the_bars_and_context_menus_take_no_room_where_they_stand() {
        for kind in [Kind::MenuBar, Kind::StatusBar, Kind::ButtonBar, Kind::ContextMenu] {
            assert!(!in_flow(&Node::new(1, kind)), "{kind:?}");
        }
        for kind in [Kind::Text, Kind::ListBox, Kind::Spacer, Kind::Column, Kind::Button] {
            assert!(in_flow(&Node::new(1, kind)), "{kind:?}");
        }
    }
}
