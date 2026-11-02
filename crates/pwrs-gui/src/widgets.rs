//! The widgets of windows described in PowerShell: the `New-Gui<Widget>`
//! commands, each writing a description of one widget, the properties each
//! widget takes, under the names its command's parameters give them and the
//! renderer's, and the window's tree built from the descriptions.

use crate::canvas::TICKS_PER_SECOND;
use crate::values::ordered;
use pwrs::prelude::*;
use pwrs::sys::{PS_TYPE_DATETIME, PS_TYPE_OBJECT, PS_TYPE_STRING, PS_TYPE_TIMESPAN};
use pwrs_gui_client::proto::canvas::Board;
use pwrs_gui_client::proto::source::{RingKind, RingSource};
use pwrs_gui_client::proto::{EventKind, Kind, Node, NodeId, Row, Shortcut, Value, date, markdown, media, picture};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Each widget the commands describe, by the name in its command and its
/// short name.
pub const WIDGETS: &[&str] = &[
    "Column",
    "Row",
    "Grid",
    "Group",
    "Tabs",
    "Tab",
    "Split",
    "Scroll",
    "Text",
    "Image",
    "Markdown",
    "ProgressBar",
    "Spinner",
    "LogView",
    "Chart",
    "GpuSurface",
    "Canvas",
    "MediaPlayer",
    "Button",
    "ListBox",
    "Table",
    "TreeView",
    "CheckBox",
    "TextBox",
    "NumberBox",
    "Slider",
    "ComboBox",
    "RadioGroup",
    "ColorPicker",
    "DatePicker",
    "FilePicker",
    "MenuBar",
    "Menu",
    "MenuItem",
    "ContextMenu",
    "StatusBar",
    "ButtonBar",
    "Separator",
    "Spacer",
];

/// The kind the widget named `name` is drawn as.
fn kind_named(name: &str) -> Option<Kind> {
    [
        Kind::Column,
        Kind::Row,
        Kind::Grid,
        Kind::Group,
        Kind::Tabs,
        Kind::Tab,
        Kind::Split,
        Kind::Scroll,
        Kind::Text,
        Kind::Image,
        Kind::Markdown,
        Kind::ProgressBar,
        Kind::Spinner,
        Kind::LogView,
        Kind::Chart,
        Kind::GpuSurface,
        Kind::Canvas,
        Kind::MediaPlayer,
        Kind::Button,
        Kind::ListBox,
        Kind::Table,
        Kind::TreeView,
        Kind::CheckBox,
        Kind::TextBox,
        Kind::NumberBox,
        Kind::Slider,
        Kind::ComboBox,
        Kind::RadioGroup,
        Kind::ColorPicker,
        Kind::DatePicker,
        Kind::FilePicker,
        Kind::MenuBar,
        Kind::Menu,
        Kind::MenuItem,
        Kind::ContextMenu,
        Kind::StatusBar,
        Kind::ButtonBar,
        Kind::Separator,
        Kind::Spacer,
    ]
    .into_iter()
    .find(|k| widget_name(*k).eq_ignore_ascii_case(name))
}

/// The name of a widget of `kind`.
pub fn widget_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Column => "Column",
        Kind::Row => "Row",
        Kind::Grid => "Grid",
        Kind::Group => "Group",
        Kind::Tabs => "Tabs",
        Kind::Tab => "Tab",
        Kind::Split => "Split",
        Kind::Scroll => "Scroll",
        Kind::Text => "Text",
        Kind::Image => "Image",
        Kind::Markdown => "Markdown",
        Kind::ProgressBar => "ProgressBar",
        Kind::Spinner => "Spinner",
        Kind::LogView => "LogView",
        Kind::Chart => "Chart",
        Kind::GpuSurface => "GpuSurface",
        Kind::Canvas => "Canvas",
        Kind::MediaPlayer => "MediaPlayer",
        Kind::Button => "Button",
        Kind::ListBox => "ListBox",
        Kind::Separator => "Separator",
        Kind::Spacer => "Spacer",
        Kind::Table => "Table",
        Kind::TreeView => "TreeView",
        Kind::TreeItem => "TreeItem",
        Kind::CheckBox => "CheckBox",
        Kind::TextBox => "TextBox",
        Kind::NumberBox => "NumberBox",
        Kind::Slider => "Slider",
        Kind::ComboBox => "ComboBox",
        Kind::RadioGroup => "RadioGroup",
        Kind::ColorPicker => "ColorPicker",
        Kind::DatePicker => "DatePicker",
        Kind::FilePicker => "FilePicker",
        Kind::MenuBar => "MenuBar",
        Kind::Menu => "Menu",
        Kind::MenuItem => "MenuItem",
        Kind::ContextMenu => "ContextMenu",
        Kind::StatusBar => "StatusBar",
        Kind::ButtonBar => "ButtonBar",
        Kind::Html => "Html",
    }
}

/// Whether a widget of `kind` lists items, which Add-GuiItem adds to and,
/// but for a log's lines and a chart's points, Remove-GuiItem takes away;
/// a table's items are its rows, a tree's its nodes and a canvas's what it
/// draws.
pub fn lists_items(kind: Kind) -> bool {
    matches!(kind, Kind::ListBox | Kind::ComboBox | Kind::RadioGroup | Kind::LogView | Kind::Chart | Kind::Table | Kind::TreeView | Kind::Canvas)
}

/// The points Add-GuiItem adds to a chart, as the renderer appends them to
/// its `points`: each a hashtable of the Series it joins, its Y, and its X,
/// a number or a date, which a point without one takes from its series'
/// count of points. `dates` is whether the chart's X are dates, which the
/// first point settles when nothing has; the points must agree with it, so
/// on a chart of dates each point gives its date.
pub fn chart_points(given: &[PsObject], dates: &mut Option<bool>) -> Result<Vec<Value>, String> {
    let mut out = Vec::with_capacity(given.len());
    for (i, p) in given.iter().enumerate() {
        out.push(chart_point_of(p, dates).map_err(|e| format!("point {}: {e}", i + 1))?);
    }
    Ok(out)
}

/// One point Add-GuiItem adds to a chart, from its hashtable.
fn chart_point_of(given: &PsObject, dates: &mut Option<bool>) -> Result<Value, String> {
    let message = |e: PsError| e.message;
    let table = PsReadOnlyTable::over(given).map_err(|e| format!("a chart's point is a hashtable of Series, X and Y: {}", e.message))?;
    let mut out = BTreeMap::new();
    let mut is_date = false;
    for key in table.keys().map_err(message)? {
        let v = table.get(&key).map_err(message)?;
        match key.to_ascii_lowercase().as_str() {
            "series" => {
                out.insert(String::from("series"), Value::Str(String::from_ps(&v).map_err(|e| format!("Series: {}", e.message))?));
            }
            "y" => {
                let y = f64::from_ps(&v).map_err(|e| format!("Y: {}", e.message))?;
                if !y.is_finite() {
                    return Err(format!("Y is {y}; it must be a finite number"));
                }
                out.insert(String::from("y"), Value::Float(y));
            }
            "x" => {
                if v.type_tag().map_err(message)? == PS_TYPE_DATETIME {
                    let when = PsDateTime::from_ps(&v).map_err(|e| format!("X: {}", e.message))?;
                    out.insert(String::from("x"), Value::Str(date::full(when.ticks)));
                    is_date = true;
                } else {
                    let x = f64::from_ps(&v).map_err(|e| format!("X: the X of a point is a number or a date: {}", e.message))?;
                    if !x.is_finite() {
                        return Err(format!("X is {x}; it must be a finite number"));
                    }
                    out.insert(String::from("x"), Value::Float(x));
                }
            }
            _ => return Err(format!("a chart's point takes Series, X and Y; it was given {key}")),
        }
    }
    if !out.contains_key("series") {
        return Err(String::from("a point needs the Series it joins"));
    }
    if !out.contains_key("y") {
        return Err(String::from("a point needs its Y"));
    }
    if *dates.get_or_insert(is_date) != is_date {
        return Err(String::from("a chart's X are numbers or dates, every point alike, and a point without X counts in numbers"));
    }
    Ok(Value::Map(out))
}

/// How a property's PowerShell value becomes the renderer's.
#[derive(Clone, Copy, Debug)]
enum Conv {
    /// Text.
    Text,
    /// True or false.
    Flag,
    /// True or false, set when the window opens, which Update-GuiWidget
    /// refuses.
    FixedFlag,
    /// A size in points.
    Points,
    /// A finite number.
    Number,
    /// One item by its text, as the renderer's row number; null for none.
    Row,
    /// A whole number above 0.
    Count,
    /// How a split places its widgets, Horizontal or Vertical, as the
    /// renderer's lower case.
    Orientation,
    /// A share from 0 to 1.
    Share,
    /// A color written `#rrggbb`, as the renderer's lower case.
    Color,
    /// A keyboard shortcut, as `Ctrl+S`, written one way.
    Shortcut,
    /// A list of texts.
    Texts,
    /// A text style, Body, Heading, Mono, Link or Instruction, as the
    /// renderer's lower case.
    Style,
    /// Where a Row, Column or Grid places its widgets across, Start, Center,
    /// End or Stretch, as the renderer's lower case.
    Align,
    /// Items of the list by their text, as the renderer's row numbers.
    Rows,
    /// How many rows can be selected, None, Single or Multiple, as the
    /// renderer's lower case.
    Selection,
    /// Data the module builds the widget from when the window opens, which
    /// is not sent as a property: a table's objects and its columns, a
    /// tree's nodes and how they give their text and say they have
    /// children.
    Data,
    /// A picture's bytes, as the renderer's map of them and their format.
    Picture,
    /// A picture's file, which the module reads, resolved as PowerShell
    /// resolves a path, as the renderer's map of its bytes and their format.
    PictureFile,
    /// How a picture fills the box its width and height make, Contain,
    /// Cover, Fill or None, as the renderer's lower case.
    Fit,
    /// A Markdown document's text, its images found from the current
    /// location, as the renderer's map of the text and the pictures read.
    Document,
    /// A Markdown document's file, which the module reads, its images found
    /// from the file's folder, as the renderer's map of the text and the
    /// pictures read.
    DocumentFile,
    /// A chart's series, hashtables of a Name, Y numbers, X numbers or
    /// dates, and a Type, as the renderer's list of maps.
    Series,
    /// How a chart draws a series, Line, Bar or Points, as the renderer's
    /// lower case.
    ChartType,
    /// A date and time, as the renderer's ISO 8601 to the minute.
    DateTime,
    /// The kinds of files a file dialog offers: `Name|*.txt;*.md` strings,
    /// or an ordered dictionary from each name to its extensions, as the
    /// renderer's list of maps of a name and its extensions.
    Filter,
    /// Paths, as the renderer's list of texts.
    Paths,
    /// A SubEtha ring another process writes, set when the window opens,
    /// as the renderer's map of its kind, path and sizes.
    Ring,
    /// A canvas's shared memory another process writes, set when the window
    /// opens, as the renderer's map of its store and keys, which
    /// [`crate::shared::canvas_shared`] makes.
    Shared,
    /// A canvas's items, hashtables of each one's Kind, Name and keys, as
    /// the renderer's list of item maps, which [`crate::canvas::items`]
    /// makes.
    Items,
    /// A clip's file, resolved as PowerShell resolves a path, as the
    /// renderer's full path to it, a path naming no file refused; or an
    /// https address, as given, which the renderer's loader fetches.
    MediaFile,
    /// A length of time, seconds as a number or a [TimeSpan], above 0, as
    /// the renderer's seconds.
    Seconds,
    /// A clip's subtitles: a SubRip or WebVTT file, resolved as PowerShell
    /// resolves a path, which the module reads, as the renderer's list of
    /// cues.
    SubtitleFile,
}

/// A property: the name PowerShell gives it, the renderer's, and how the
/// value converts.
struct Prop {
    name: &'static str,
    wire: &'static str,
    conv: Conv,
}

const TEXT_PROPS: &[Prop] = &[Prop { name: "Text", wire: "text", conv: Conv::Text }, Prop { name: "Style", wire: "style", conv: Conv::Style }];
const IMAGE_PROPS: &[Prop] = &[
    Prop { name: "Path", wire: "image", conv: Conv::PictureFile },
    Prop { name: "Bytes", wire: "image", conv: Conv::Picture },
    Prop { name: "Width", wire: "width", conv: Conv::Points },
    Prop { name: "Height", wire: "height", conv: Conv::Points },
    Prop { name: "Fit", wire: "fit", conv: Conv::Fit },
    Prop { name: "AltText", wire: "alt", conv: Conv::Text },
];
const MARKDOWN_PROPS: &[Prop] = &[Prop { name: "Text", wire: "document", conv: Conv::Document }, Prop { name: "Path", wire: "document", conv: Conv::DocumentFile }];
const BUTTON_PROPS: &[Prop] = &[
    Prop { name: "Text", wire: "text", conv: Conv::Text },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
    Prop { name: "Default", wire: "default", conv: Conv::FixedFlag },
    Prop { name: "Cancel", wire: "cancel", conv: Conv::FixedFlag },
];
const LIST_PROPS: &[Prop] = &[
    Prop { name: "Item", wire: "items", conv: Conv::Texts },
    Prop { name: "Multiple", wire: "multiple", conv: Conv::Flag },
    Prop { name: "Selected", wire: "selected", conv: Conv::Rows },
];
const SPACER_PROPS: &[Prop] = &[Prop { name: "Size", wire: "size", conv: Conv::Points }, Prop { name: "Fill", wire: "fill", conv: Conv::Flag }];
const ALIGN_PROPS: &[Prop] = &[Prop { name: "Align", wire: "align", conv: Conv::Align }];
const TABLE_PROPS: &[Prop] = &[
    Prop { name: "Selection", wire: "selection", conv: Conv::Selection },
    Prop { name: "Filter", wire: "filter", conv: Conv::Flag },
    Prop { name: "Item", wire: "", conv: Conv::Data },
    Prop { name: "Property", wire: "", conv: Conv::Data },
];
const TREE_PROPS: &[Prop] = &[
    Prop { name: "Node", wire: "", conv: Conv::Data },
    Prop { name: "Text", wire: "", conv: Conv::Data },
    Prop { name: "HasChildren", wire: "", conv: Conv::Data },
    Prop { name: "Multiple", wire: "multiple", conv: Conv::Flag },
];
const CHECK_PROPS: &[Prop] = &[
    Prop { name: "Text", wire: "text", conv: Conv::Text },
    Prop { name: "Checked", wire: "checked", conv: Conv::Flag },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
];
const TEXT_BOX_PROPS: &[Prop] = &[
    Prop { name: "Text", wire: "text", conv: Conv::Text },
    Prop { name: "Multiline", wire: "multiline", conv: Conv::Flag },
    Prop { name: "Password", wire: "password", conv: Conv::FixedFlag },
    Prop { name: "Placeholder", wire: "placeholder", conv: Conv::Text },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
];
const NUMBER_PROPS: &[Prop] = &[
    Prop { name: "Value", wire: "value", conv: Conv::Number },
    Prop { name: "Minimum", wire: "minimum", conv: Conv::Number },
    Prop { name: "Maximum", wire: "maximum", conv: Conv::Number },
    Prop { name: "Step", wire: "step", conv: Conv::Number },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
];
const CHOICE_PROPS: &[Prop] = &[
    Prop { name: "Choice", wire: "items", conv: Conv::Texts },
    Prop { name: "Selected", wire: "selected", conv: Conv::Row },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
];
const COMBO_PROPS: &[Prop] = &[
    Prop { name: "Choice", wire: "items", conv: Conv::Texts },
    Prop { name: "Selected", wire: "selected", conv: Conv::Row },
    Prop { name: "Editable", wire: "editable", conv: Conv::FixedFlag },
    Prop { name: "Text", wire: "text", conv: Conv::Text },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
];
const COLOR_PROPS: &[Prop] = &[Prop { name: "Color", wire: "color", conv: Conv::Color }, Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag }];
const MENU_ITEM_PROPS: &[Prop] = &[
    Prop { name: "Text", wire: "text", conv: Conv::Text },
    Prop { name: "Shortcut", wire: "shortcut", conv: Conv::Shortcut },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
];
const CONTEXT_PROPS: &[Prop] = &[Prop { name: "For", wire: "for", conv: Conv::Text }];

/// A color written `#rrggbb`, in either case, in lower case; `None` for
/// anything else.
fn color(given: &str) -> Option<String> {
    let digits = given.strip_prefix('#')?;
    if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()) { Some(given.to_ascii_lowercase()) } else { None }
}

/// A picture's `bytes` as the renderer takes them, as the protocol crate
/// reads them for the module and every renderer ([`picture::value`]).
pub(crate) fn picture(bytes: Vec<u8>) -> Result<Value, String> {
    picture::value(bytes)
}

/// The bytes of the file `path` names, resolved from the current location
/// as PowerShell resolves a path, wildcards taken literally.
pub(crate) fn picture_file(ps: &Pipeline<'_>, path: &str) -> Result<Vec<u8>, String> {
    let resolved = ps.resolve_path(path, true).map_err(|e| e.message)?;
    let full = match resolved.as_slice() {
        [one] => std::path::PathBuf::from(one),
        [] => return Err(format!("{path} names no file")),
        many => return Err(format!("{path} names {} files; it must name one", many.len())),
    };
    std::fs::read(&full).map_err(|e| format!("{} could not be read: {e}", full.display()))
}

/// The format of a picture's `bytes`, as the protocol crate tells it for
/// the module and every renderer ([`picture::format`]).
pub fn picture_format(bytes: &[u8]) -> Result<&'static str, String> {
    picture::format(bytes)
}

/// A Markdown document as the renderer takes it: its text, and a picture
/// for each image the text names that the module reads: the data in its
/// address, or a file found from `base`, the document's folder, unless its
/// path is absolute. An https image is left to the renderer. An image the
/// module cannot read, or whose address names no source it reads from, is
/// written as a warning naming it and why, and shows its alt text.
fn document(ps: &Pipeline<'_>, text: String, base: Option<&std::path::Path>) -> Result<Value, String> {
    let mut pictures = BTreeMap::new();
    for address in markdown::image_addresses(&text) {
        let read = match markdown::source(&address) {
            markdown::Source::Web(_) => continue,
            markdown::Source::Data(bytes) => bytes,
            markdown::Source::File(path) => document_picture(&path, base),
            markdown::Source::Refused(why) => Err(why.to_string()),
        };
        match read.and_then(picture) {
            Ok(shown) => {
                pictures.insert(address, shown);
            }
            Err(why) => ps.warning(&format!("the image {address} shows its alt text: {why}")).map_err(|e| e.message)?,
        }
    }
    Ok(Value::Map(BTreeMap::from([(String::from("text"), Value::Str(text)), (String::from("pictures"), Value::Map(pictures))])))
}

/// The bytes of a document's image file `path`: as it is when absolute,
/// else found from `base`, the document's folder; a path that names a
/// share once joined to the folder is refused, as one written so is.
fn document_picture(path: &str, base: Option<&std::path::Path>) -> Result<Vec<u8>, String> {
    let given = std::path::Path::new(path);
    let full = if given.is_absolute() || given.has_root() {
        given.to_path_buf()
    } else {
        match base {
            Some(folder) => folder.join(given),
            None => return Err(String::from("its path is relative, and the current location is no folder of files to find it in")),
        }
    };
    if markdown::names_a_share(&full.to_string_lossy()) {
        return Err(markdown::SHARED.to_string());
    }
    std::fs::read(&full).map_err(|e| format!("{} could not be read: {e}", full.display()))
}

/// The folder of files the current location is, which a document's
/// relative images are found from; none when the location belongs to
/// another provider, as the registry's does.
fn current_folder(ps: &Pipeline<'_>) -> Result<Option<std::path::PathBuf>, String> {
    let here = ps.resolve_path(".", true).map_err(|e| e.message)?;
    Ok(here.first().map(std::path::PathBuf::from).filter(|p| p.is_dir()))
}

/// `path` as a full path, a relative one taken from the current location,
/// whether or not anything is there yet, as a file to save need not be.
pub(crate) fn full_path(ps: &Pipeline<'_>, path: &str) -> PsResult<String> {
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadPath", text).terminating();
    let given = std::path::Path::new(path);
    let joined = if given.is_absolute() {
        given.to_path_buf()
    } else {
        match current_folder(ps).map_err(refuse)? {
            Some(folder) => folder.join(given),
            None => return Err(refuse(format!("{path} is relative, and the current location is no folder of files to find it in"))),
        }
    };
    let full = std::path::absolute(&joined).map_err(|e| refuse(format!("{path} is no path: {e}")))?;
    Ok(full.to_string_lossy().into_owned())
}

/// The text of the Markdown file `path` names, resolved from the current
/// location as PowerShell resolves a path, and the file's full path. The
/// text is UTF-8, or UTF-16 when a byte order mark says so, as Windows
/// PowerShell writes files.
fn document_file(ps: &Pipeline<'_>, path: &str) -> Result<(String, std::path::PathBuf), String> {
    let resolved = ps.resolve_path(path, true).map_err(|e| e.message)?;
    let full = match resolved.as_slice() {
        [one] => std::path::PathBuf::from(one),
        [] => return Err(format!("{path} names no file")),
        many => return Err(format!("{path} names {} files; it must name one", many.len())),
    };
    let bytes = std::fs::read(&full).map_err(|e| format!("{} could not be read: {e}", full.display()))?;
    let text = text_of(&bytes).map_err(|e| format!("{}: {e}", full.display()))?;
    Ok((text, full))
}

/// A file's bytes as text: UTF-8, its byte order mark dropped, or UTF-16
/// in either byte order when its byte order mark says so.
fn text_of(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |rest: &[u8], word: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.chunks(2).map(|c| word([c[0], *c.get(1).unwrap_or(&0)])).collect();
        String::from_utf16(&units).map_err(|e| format!("its UTF-16 text cannot be read: {e}"))
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    let rest = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(rest.to_vec()).map_err(|e| format!("its text is neither UTF-8 nor marked as UTF-16: {e}"))
}

/// Whether `node` is an editable drop-down, whose value is the text typed
/// or picked rather than a row chosen.
pub fn is_editable(node: &Node) -> bool {
    node.kind == Kind::ComboBox && matches!(node.props.get("editable"), Some(Value::Bool(true)))
}
const GRID_PROPS: &[Prop] = &[Prop { name: "Columns", wire: "columns", conv: Conv::Count }, Prop { name: "Align", wire: "align", conv: Conv::Align }];
const TITLE_PROPS: &[Prop] = &[Prop { name: "Title", wire: "title", conv: Conv::Text }];
const TABS_PROPS: &[Prop] = &[Prop { name: "Selected", wire: "selected", conv: Conv::Row }];
const SPLIT_PROPS: &[Prop] = &[Prop { name: "Orientation", wire: "orientation", conv: Conv::Orientation }, Prop { name: "Position", wire: "position", conv: Conv::Share }];
const SCROLL_PROPS: &[Prop] = &[Prop { name: "Height", wire: "height", conv: Conv::Points }];
const PROGRESS_PROPS: &[Prop] = &[
    Prop { name: "Value", wire: "value", conv: Conv::Number },
    Prop { name: "Maximum", wire: "maximum", conv: Conv::Number },
    Prop { name: "Text", wire: "text", conv: Conv::Text },
    Prop { name: "Ring", wire: "ring", conv: Conv::Ring },
];
const LOG_PROPS: &[Prop] = &[
    Prop { name: "Line", wire: "lines", conv: Conv::Texts },
    Prop { name: "Width", wire: "width", conv: Conv::Points },
    Prop { name: "Height", wire: "height", conv: Conv::Points },
    Prop { name: "Ring", wire: "ring", conv: Conv::Ring },
];
const CHART_PROPS: &[Prop] = &[
    Prop { name: "Series", wire: "series", conv: Conv::Series },
    Prop { name: "Type", wire: "type", conv: Conv::ChartType },
    Prop { name: "XLabel", wire: "x_label", conv: Conv::Text },
    Prop { name: "YLabel", wire: "y_label", conv: Conv::Text },
    Prop { name: "Height", wire: "height", conv: Conv::Points },
    Prop { name: "Ring", wire: "ring", conv: Conv::Ring },
];
const GPU_SURFACE_PROPS: &[Prop] = &[Prop { name: "Width", wire: "width", conv: Conv::Points }, Prop { name: "Height", wire: "height", conv: Conv::Points }];
const MEDIA_PROPS: &[Prop] = &[
    Prop { name: "Path", wire: "source", conv: Conv::MediaFile },
    Prop { name: "AudioOnly", wire: "audio_only", conv: Conv::FixedFlag },
    Prop { name: "AutoPlay", wire: "autoplay", conv: Conv::FixedFlag },
    Prop { name: "Loop", wire: "loop", conv: Conv::Flag },
    Prop { name: "Muted", wire: "muted", conv: Conv::Flag },
    Prop { name: "Volume", wire: "volume", conv: Conv::Number },
    Prop { name: "Rate", wire: "rate", conv: Conv::Number },
    Prop { name: "PositionInterval", wire: "position_interval", conv: Conv::Seconds },
    Prop { name: "NoControls", wire: "no_controls", conv: Conv::Flag },
    Prop { name: "Subtitles", wire: "subtitles", conv: Conv::SubtitleFile },
    Prop { name: "EngineFetch", wire: "engine_fetch", conv: Conv::Flag },
    Prop { name: "Width", wire: "width", conv: Conv::Points },
    Prop { name: "Height", wire: "height", conv: Conv::Points },
];
const CANVAS_PROPS: &[Prop] = &[
    Prop { name: "Item", wire: "items", conv: Conv::Items },
    Prop { name: "Width", wire: "width", conv: Conv::Points },
    Prop { name: "Height", wire: "height", conv: Conv::Points },
    Prop { name: "Ring", wire: "ring", conv: Conv::Ring },
    Prop { name: "SharedMemory", wire: "shared", conv: Conv::Shared },
];
const DATE_PROPS: &[Prop] = &[
    Prop { name: "Value", wire: "value", conv: Conv::DateTime },
    Prop { name: "Minimum", wire: "minimum", conv: Conv::DateTime },
    Prop { name: "Maximum", wire: "maximum", conv: Conv::DateTime },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
];
const FILE_PROPS: &[Prop] = &[
    Prop { name: "Path", wire: "paths", conv: Conv::Paths },
    Prop { name: "Save", wire: "save", conv: Conv::FixedFlag },
    Prop { name: "Folder", wire: "folder", conv: Conv::FixedFlag },
    Prop { name: "Multiple", wire: "multiple", conv: Conv::FixedFlag },
    Prop { name: "Filter", wire: "filter", conv: Conv::Filter },
    Prop { name: "Title", wire: "title", conv: Conv::Text },
    Prop { name: "Enabled", wire: "enabled", conv: Conv::Flag },
];

/// The properties a widget of `kind` takes.
fn props_of(kind: Kind) -> &'static [Prop] {
    match kind {
        Kind::Grid => GRID_PROPS,
        Kind::Group | Kind::Tab | Kind::Menu => TITLE_PROPS,
        Kind::MenuItem => MENU_ITEM_PROPS,
        Kind::ContextMenu => CONTEXT_PROPS,
        Kind::MenuBar | Kind::StatusBar | Kind::ButtonBar => &[],
        Kind::Tabs => TABS_PROPS,
        Kind::Split => SPLIT_PROPS,
        Kind::Scroll => SCROLL_PROPS,
        Kind::Text => TEXT_PROPS,
        Kind::Image => IMAGE_PROPS,
        Kind::Markdown => MARKDOWN_PROPS,
        Kind::ProgressBar => PROGRESS_PROPS,
        Kind::LogView => LOG_PROPS,
        Kind::Chart => CHART_PROPS,
        Kind::GpuSurface => GPU_SURFACE_PROPS,
        Kind::Canvas => CANVAS_PROPS,
        Kind::MediaPlayer => MEDIA_PROPS,
        Kind::DatePicker => DATE_PROPS,
        Kind::FilePicker => FILE_PROPS,
        Kind::Spinner => &[],
        Kind::Button => BUTTON_PROPS,
        Kind::ListBox => LIST_PROPS,
        Kind::Spacer => SPACER_PROPS,
        Kind::Table => TABLE_PROPS,
        Kind::TreeView => TREE_PROPS,
        Kind::TreeItem => &[],
        Kind::CheckBox => CHECK_PROPS,
        Kind::TextBox => TEXT_BOX_PROPS,
        Kind::NumberBox | Kind::Slider => NUMBER_PROPS,
        Kind::ComboBox => COMBO_PROPS,
        Kind::RadioGroup => CHOICE_PROPS,
        Kind::ColorPicker => COLOR_PROPS,
        Kind::Column | Kind::Row => ALIGN_PROPS,
        Kind::Separator | Kind::Html => &[],
    }
}

/// The events a widget of `kind` raises, by the names its parameters give
/// them.
fn events_of(kind: Kind) -> &'static [(&'static str, EventKind)] {
    match kind {
        Kind::Button | Kind::Text | Kind::MenuItem | Kind::Markdown => &[("Click", EventKind::Click)],
        Kind::MenuBar | Kind::Menu | Kind::ContextMenu | Kind::StatusBar | Kind::ButtonBar => &[],
        Kind::ListBox | Kind::Table => &[("Select", EventKind::Select), ("Activate", EventKind::Activate)],
        Kind::TreeView => &[("Select", EventKind::Select), ("Activate", EventKind::Activate), ("Expand", EventKind::Expand)],
        Kind::TreeItem => &[],
        Kind::TextBox => &[("Change", EventKind::Change), ("Submit", EventKind::Submit)],
        Kind::CheckBox | Kind::NumberBox | Kind::Slider | Kind::ComboBox | Kind::RadioGroup | Kind::Tabs | Kind::ColorPicker | Kind::DatePicker | Kind::FilePicker => &[("Change", EventKind::Change)],
        Kind::Chart => &[("Select", EventKind::Select)],
        Kind::GpuSurface => &[("PointerDown", EventKind::PointerDown), ("PointerUp", EventKind::PointerUp), ("PointerMove", EventKind::PointerMove), ("Wheel", EventKind::Wheel)],
        Kind::Canvas => &[
            ("Click", EventKind::Click),
            ("DoubleClick", EventKind::DoubleClick),
            ("PointerEnter", EventKind::PointerEnter),
            ("PointerLeave", EventKind::PointerLeave),
            ("DragStart", EventKind::DragStart),
            ("DragMove", EventKind::DragMove),
            ("DragEnd", EventKind::DragEnd),
            ("PointerDown", EventKind::PointerDown),
            ("PointerUp", EventKind::PointerUp),
            ("PointerMove", EventKind::PointerMove),
            ("Wheel", EventKind::Wheel),
        ],
        Kind::MediaPlayer => &[
            ("Loaded", EventKind::Loaded),
            ("Playing", EventKind::Playing),
            ("Paused", EventKind::Paused),
            ("Ended", EventKind::Ended),
            ("Position", EventKind::Position),
            ("Error", EventKind::Error),
        ],
        Kind::Column
        | Kind::Row
        | Kind::Grid
        | Kind::Group
        | Kind::Tab
        | Kind::Split
        | Kind::Scroll
        | Kind::Image
        | Kind::ProgressBar
        | Kind::Spinner
        | Kind::LogView
        | Kind::Separator
        | Kind::Spacer
        | Kind::Html => &[],
    }
}

/// The event a widget of `kind` raises under the name `name`, without
/// regard to case: one of [`events_of`], Drop for any widget drawn among
/// the window's widgets, and Key for one that takes the focus.
fn event_named(kind: Kind, name: &str) -> Option<EventKind> {
    if let Some((_, event)) = events_of(kind).iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return Some(*event);
    }
    if name.eq_ignore_ascii_case("Drop") && takes_drops(kind) {
        return Some(EventKind::Drop);
    }
    if name.eq_ignore_ascii_case("Key") && takes_keys(kind) {
        return Some(EventKind::Key);
    }
    None
}

/// Whether files or text can be dropped on a widget of `kind`: any drawn
/// among the window's widgets that shows something or contains widgets, which
/// a menu, a button bar, a tree's item, a page, a rule and empty space do
/// not.
fn takes_drops(kind: Kind) -> bool {
    !matches!(kind, Kind::MenuBar | Kind::Menu | Kind::MenuItem | Kind::ContextMenu | Kind::ButtonBar | Kind::TreeItem | Kind::Html | Kind::Separator | Kind::Spacer)
}

/// Whether a widget of `kind` takes the focus, so the keys pressed reach it.
fn takes_keys(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Button
            | Kind::ListBox
            | Kind::Table
            | Kind::TreeView
            | Kind::CheckBox
            | Kind::TextBox
            | Kind::NumberBox
            | Kind::Slider
            | Kind::ComboBox
            | Kind::RadioGroup
            | Kind::ColorPicker
            | Kind::DatePicker
            | Kind::FilePicker
            | Kind::GpuSurface
            | Kind::Canvas
    )
}

/// The renderer's property of `node` that records what a person chose or
/// entered, which State reports; `None` for a widget that takes no input.
pub fn value_prop(node: &Node) -> Option<&'static str> {
    match node.kind {
        Kind::ComboBox if is_editable(node) => Some("text"),
        Kind::ListBox | Kind::Table | Kind::TreeView | Kind::ComboBox | Kind::RadioGroup | Kind::Tabs => Some("selected"),
        Kind::CheckBox => Some("checked"),
        Kind::TextBox => Some("text"),
        Kind::NumberBox | Kind::Slider | Kind::DatePicker => Some("value"),
        Kind::ColorPicker => Some("color"),
        Kind::FilePicker => Some("paths"),
        Kind::MenuBar | Kind::Menu | Kind::MenuItem | Kind::ContextMenu | Kind::StatusBar | Kind::ButtonBar | Kind::TreeItem => None,
        Kind::Column
        | Kind::Row
        | Kind::Grid
        | Kind::Group
        | Kind::Tab
        | Kind::Split
        | Kind::Scroll
        | Kind::Text
        | Kind::Image
        | Kind::Markdown
        | Kind::ProgressBar
        | Kind::Spinner
        | Kind::LogView
        | Kind::Chart
        | Kind::GpuSurface
        | Kind::Canvas
        | Kind::MediaPlayer
        | Kind::Button
        | Kind::Separator
        | Kind::Spacer
        | Kind::Html => None,
    }
}

/// The renderer's properties for `given`, a dictionary by the PowerShell
/// names of `kind`'s properties: each name must be one of them, without
/// regard to case, and no two may set the same renderer property, as an
/// Image's Path and Bytes do. `items` are the list's items, which a
/// selection names by their text; `given` may change them first. Data the
/// module builds the widget from is passed over when `building` and refused
/// otherwise.
pub fn wire_props(ps: &Pipeline<'_>, kind: Kind, given: &PsObject, items: &[String], building: bool) -> PsResult<BTreeMap<String, Value>> {
    let table = PsReadOnlyTable::over(given)?;
    let known = props_of(kind);
    let mut entries: Vec<(&Prop, PsObject)> = Vec::new();
    for key in table.keys()? {
        let Some(prop) = known.iter().find(|p| p.name.eq_ignore_ascii_case(&key)) else {
            let names: Vec<&str> = known.iter().map(|p| p.name).collect();
            let text = if names.is_empty() {
                format!("a {} takes no property; it was given {key}", widget_name(kind))
            } else {
                format!("a {} takes {}; it was given {key}", widget_name(kind), names.join(", "))
            };
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiUnknownProperty", text).terminating());
        };
        if let Some((other, _)) = entries.iter().find(|(p, _)| !p.wire.is_empty() && p.wire == prop.wire) {
            let text = format!("a {} takes {} or {}, not both", widget_name(kind), other.name, prop.name);
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
        }
        // Read from `given` itself: the read-only view returns a nested
        // dictionary back as a view of its own, which hides whether the
        // caller's was ordered.
        entries.push((prop, given.call("get_Item", &[key.as_str().into_ps()?])?));
    }
    let mut items: Vec<String> = items.to_vec();
    for (prop, value) in &entries {
        if prop.wire == "items" && matches!(prop.conv, Conv::Texts) {
            items = strings(ps, value)?;
        }
    }
    let mut out = BTreeMap::new();
    for (prop, value) in entries {
        if matches!(prop.conv, Conv::Data | Conv::FixedFlag | Conv::Ring | Conv::Shared) && !building {
            let text = format!("a {}'s {} is taken when the window opens", widget_name(kind), prop.name);
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiOpeningOnly", text).terminating());
        }
        if let Conv::Data = prop.conv {
            continue;
        }
        let wire = convert(ps, prop, &value, &items).map_err(|e| PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", e).terminating())?;
        out.insert(prop.wire.to_string(), wire);
    }
    Ok(out)
}

/// One property's value as the renderer takes it.
fn convert(ps: &Pipeline<'_>, prop: &Prop, value: &PsObject, items: &[String]) -> Result<Value, String> {
    let name = prop.name;
    let shown = |e: PsError| format!("{name}: {}", e.message);
    match prop.conv {
        Conv::Picture => Vec::<u8>::from_ps(value).map_err(shown).and_then(|bytes| picture(bytes).map_err(|e| format!("{name}: {e}"))),
        Conv::PictureFile => {
            let path = String::from_ps(value).map_err(shown)?;
            picture(picture_file(ps, &path).map_err(|e| format!("{name}: {e}"))?).map_err(|e| format!("{name} {path}: {e}"))
        }
        Conv::Fit => {
            let given = String::from_ps(value).map_err(shown)?;
            match given.to_ascii_lowercase().as_str() {
                f @ ("contain" | "cover" | "fill" | "none") => Ok(Value::Str(f.to_string())),
                _ => Err(format!("{name} is {given}; it must be Contain, Cover, Fill or None")),
            }
        }
        Conv::Document => {
            let text = String::from_ps(value).map_err(shown)?;
            let base = current_folder(ps).map_err(|e| format!("{name}: {e}"))?;
            document(ps, text, base.as_deref()).map_err(|e| format!("{name}: {e}"))
        }
        Conv::DocumentFile => {
            let path = String::from_ps(value).map_err(shown)?;
            let (text, full) = document_file(ps, &path).map_err(|e| format!("{name}: {e}"))?;
            document(ps, text, full.parent()).map_err(|e| format!("{name}: {e}"))
        }
        Conv::Series => {
            // A dictionary given alone is one series, where reading it as a
            // list would give its entries.
            let given = if value.is_null() {
                Vec::new()
            } else if matches!(shape(ps, value).map_err(shown)?.as_str(), "ordered" | "dictionary") {
                vec![value.clone()]
            } else {
                Vec::<PsObject>::from_ps(value).map_err(shown)?
            };
            let mut dated = None;
            let mut out = Vec::with_capacity(given.len());
            for (i, s) in given.iter().enumerate() {
                out.push(series_of(s, &mut dated).map_err(|e| format!("{name}: series {}: {e}", i + 1))?);
            }
            Ok(Value::List(out))
        }
        Conv::ChartType => {
            let given = String::from_ps(value).map_err(shown)?;
            chart_type(&given).map(|t| Value::Str(t.to_string())).map_err(|e| format!("{name}: {e}"))
        }
        Conv::DateTime => {
            if value.is_null() {
                return Ok(Value::Null);
            }
            let when = PsDateTime::from_ps(value).map_err(shown)?;
            Ok(Value::Str(date::minutes(when.ticks)))
        }
        Conv::Filter => filter_of(ps, value).map_err(|e| format!("{name}: {e}")),
        Conv::Ring => ring_of(value).map_err(|e| format!("{name}: {e}")),
        Conv::Shared => crate::shared::canvas_shared(ps, value).map_err(|e| format!("{name}: {e}")),
        Conv::Items => crate::canvas::items(ps, value).map_err(|e| format!("{name}: {e}")),
        Conv::MediaFile => {
            let path = String::from_ps(value).map_err(shown)?;
            if path.starts_with("https://") {
                return Ok(Value::Str(path));
            }
            let full = full_path(ps, &path).map_err(shown)?;
            if !std::path::Path::new(&full).is_file() {
                return Err(format!("{name} {path}: no file is at {full}"));
            }
            Ok(Value::Str(full))
        }
        Conv::Seconds => {
            let seconds = if value.type_tag().map_err(shown)? == PS_TYPE_TIMESPAN {
                PsTimeSpan::from_ps(value).map_err(shown)?.ticks as f64 / TICKS_PER_SECOND
            } else {
                f64::from_ps(value).map_err(|e| format!("{name} takes seconds or a [TimeSpan]: {}", e.message))?
            };
            if !(seconds.is_finite() && seconds > 0.0) {
                return Err(format!("{name} is {seconds}; it is a length of time in seconds, above 0"));
            }
            Ok(Value::Float(seconds))
        }
        Conv::SubtitleFile => {
            if value.is_null() {
                return Ok(Value::Null);
            }
            let path = String::from_ps(value).map_err(shown)?;
            let full = full_path(ps, &path).map_err(shown)?;
            let text = std::fs::read_to_string(&full).map_err(|e| format!("{name} {path}: {full} cannot be read as text: {e}"))?;
            let cues = media::subtitles::parse(&text).map_err(|e| format!("{name} {path}: {e}"))?;
            Ok(media::subtitles::value(&cues))
        }
        Conv::Paths => {
            let mut full = Vec::new();
            for p in strings(ps, value).map_err(shown)? {
                full.push(Value::Str(full_path(ps, &p).map_err(shown)?));
            }
            Ok(Value::List(full))
        }
        Conv::Text => String::from_ps(value).map(Value::Str).map_err(shown),
        Conv::Flag | Conv::FixedFlag => bool::from_ps(value).map(Value::Bool).map_err(shown),
        Conv::Points => f64::from_ps(value).map(Value::Float).map_err(shown),
        Conv::Number => {
            let n = f64::from_ps(value).map_err(shown)?;
            if n.is_finite() { Ok(Value::Float(n)) } else { Err(format!("{name} is {n}; it must be a finite number")) }
        }
        Conv::Row => {
            if value.is_null() {
                return Ok(Value::Null);
            }
            let chosen = String::from_ps(value).map_err(shown)?;
            let row = items.iter().position(|i| i.eq_ignore_ascii_case(&chosen)).ok_or_else(|| format!("{name} is {chosen}; it must be one of {}", items.join(", ")))?;
            i64::try_from(row).map(Value::Int).map_err(|e| format!("row {row} does not fit the protocol: {e}"))
        }
        Conv::Count => {
            let n = i64::from_ps(value).map_err(shown)?;
            if n >= 1 { Ok(Value::Int(n)) } else { Err(format!("{name} is {n}; it must be a whole number above 0")) }
        }
        Conv::Orientation => {
            let given = String::from_ps(value).map_err(shown)?;
            match given.to_ascii_lowercase().as_str() {
                o @ ("horizontal" | "vertical") => Ok(Value::Str(o.to_string())),
                _ => Err(format!("{name} is {given}; it must be Horizontal or Vertical")),
            }
        }
        Conv::Share => {
            let n = f64::from_ps(value).map_err(shown)?;
            if (0.0..=1.0).contains(&n) { Ok(Value::Float(n)) } else { Err(format!("{name} is {n}; it must be from 0 to 1")) }
        }
        Conv::Color => {
            let given = String::from_ps(value).map_err(shown)?;
            color(&given).ok_or_else(|| format!("{name} is {given}; it must be a color written #rrggbb")).map(Value::Str)
        }
        Conv::Shortcut => {
            let given = String::from_ps(value).map_err(shown)?;
            Shortcut::parse(&given).map(|s| Value::Str(s.written())).map_err(|e| format!("{name}: {e}"))
        }
        Conv::Texts => strings(ps, value).map(|v| Value::List(v.into_iter().map(Value::Str).collect())).map_err(shown),
        Conv::Style => {
            let style = String::from_ps(value).map_err(shown)?;
            match style.to_ascii_lowercase().as_str() {
                s @ ("body" | "heading" | "mono" | "link" | "instruction") => Ok(Value::Str(s.to_string())),
                _ => Err(format!("{name} is {style}; it must be Body, Heading, Mono, Link or Instruction")),
            }
        }
        Conv::Align => {
            let align = String::from_ps(value).map_err(shown)?;
            match align.to_ascii_lowercase().as_str() {
                a @ ("start" | "center" | "end" | "stretch") => Ok(Value::Str(a.to_string())),
                _ => Err(format!("{name} is {align}; it must be Start, Center, End or Stretch")),
            }
        }
        Conv::Selection => {
            let mode = String::from_ps(value).map_err(shown)?;
            match mode.to_ascii_lowercase().as_str() {
                m @ ("none" | "single" | "multiple") => Ok(Value::Str(m.to_string())),
                _ => Err(format!("{name} is {mode}; it must be None, Single or Multiple")),
            }
        }
        Conv::Data => Err(format!("{name} is taken when the window opens")),
        Conv::Rows => {
            let chosen = strings(ps, value).map_err(shown)?;
            let mut rows = Vec::with_capacity(chosen.len());
            for c in &chosen {
                let row = items.iter().position(|i| i.eq_ignore_ascii_case(c)).ok_or_else(|| format!("{name} names {c}, which is not one of the items"))?;
                rows.push(Value::Int(i64::try_from(row).map_err(|e| format!("row {row} does not fit the protocol: {e}"))?));
            }
            Ok(Value::List(rows))
        }
    }
}

fn bad_number(text: String) -> PsError {
    PsError::new(ErrorCategory::InvalidArgument, "GuiBadNumber", text).terminating()
}

fn bad_date(text: String) -> PsError {
    PsError::new(ErrorCategory::InvalidArgument, "GuiBadDate", text).terminating()
}

/// Whether the X of a chart's `series`, as [`series_of`] writes them, are
/// dates: told by the first series with a point, none when no series has
/// one.
fn series_dates(series: &[Value]) -> Option<bool> {
    series.iter().filter_map(Value::as_map).find(|s| s.get("y").and_then(Value::as_list).is_some_and(|y| !y.is_empty())).map(|s| {
        let first_x = s.get("x").and_then(Value::as_list).and_then(<[Value]>::first);
        matches!(first_x, Some(Value::Str(_)))
    })
}

/// One series of a chart from its hashtable: its Name, its Y numbers, its
/// X, as many as Y, and its Type. A single number or date given for X or Y
/// is one point, as PowerShell binds one value to an array parameter.
/// `dated` is whether the chart's X are dates, which the first series
/// sets and every other must match; a series without X counts 0, 1, 2 and
/// on, which are numbers.
fn series_of(given: &PsObject, dated: &mut Option<bool>) -> Result<Value, String> {
    let message = |e: PsError| e.message;
    let table = PsReadOnlyTable::over(given).map_err(|e| format!("a series is a hashtable of Name, X, Y and Type: {}", e.message))?;
    let mut out = BTreeMap::new();
    let mut x_given = None;
    let mut y_count = None;
    for key in table.keys().map_err(message)? {
        let v = table.get(&key).map_err(message)?;
        match key.to_ascii_lowercase().as_str() {
            "name" => {
                out.insert(String::from("name"), Value::Str(String::from_ps(&v).map_err(|e| format!("Name: {}", e.message))?));
            }
            "y" => {
                let ys = Vec::<f64>::from_ps(&as_list(&v)?).map_err(|e| format!("Y: {}", e.message))?;
                if let Some(bad) = ys.iter().find(|y| !y.is_finite()) {
                    return Err(format!("Y has {bad}; each point's Y must be a finite number"));
                }
                y_count = Some(ys.len());
                out.insert(String::from("y"), Value::List(ys.into_iter().map(Value::Float).collect()));
            }
            "x" => x_given = Some(v),
            "type" => {
                let written = String::from_ps(&v).map_err(|e| format!("Type: {}", e.message))?;
                out.insert(String::from("type"), Value::Str(chart_type(&written)?.to_string()));
            }
            _ => return Err(format!("a series takes Name, X, Y and Type; it was given {key}")),
        }
    }
    if !out.contains_key("name") {
        return Err(String::from("a series needs a Name"));
    }
    let Some(count) = y_count else {
        return Err(String::from("a series needs Y, its points' values"));
    };
    let Some(x) = x_given else {
        settle_dates(dated, false, count)?;
        return Ok(Value::Map(out));
    };
    let (xs, is_dates) = x_of(&x)?;
    if xs.len() != count {
        let given = if xs.len() == 1 { String::from("1 value") } else { format!("{} values", xs.len()) };
        return Err(format!("X has {given} and Y {count}; they must be as many"));
    }
    settle_dates(dated, is_dates, count)?;
    out.insert(String::from("x"), Value::List(xs));
    Ok(Value::Map(out))
}

/// Settles whether a chart's X are dates from a series of `count` points
/// whose X are dates or not, refusing a series unlike the ones before it;
/// a series of no points settles nothing.
fn settle_dates(dated: &mut Option<bool>, is_dates: bool, count: usize) -> Result<(), String> {
    if count == 0 {
        return Ok(());
    }
    if *dated.get_or_insert(is_dates) != is_dates {
        return Err(String::from("a chart's X are numbers or dates, every series alike, and a series without X counts in numbers"));
    }
    Ok(())
}

/// A series' X and whether they are dates: numbers, or dates as
/// [`date::full`] writes them, told by the first value's type; every value
/// must be of that kind.
fn x_of(given: &PsObject) -> Result<(Vec<Value>, bool), String> {
    let given = &as_list(given)?;
    let items = Vec::<PsObject>::from_ps(given).map_err(|e| format!("X: {}", e.message))?;
    let Some(first) = items.first() else {
        return Ok((Vec::new(), false));
    };
    if first.type_tag().map_err(|e| e.message)? == PS_TYPE_DATETIME {
        let dates = Vec::<PsDateTime>::from_ps(given).map_err(|e| format!("X: the first is a date, so each must be one: {}", e.message))?;
        return Ok((dates.into_iter().map(|d| Value::Str(date::full(d.ticks))).collect(), true));
    }
    let numbers = Vec::<f64>::from_ps(given).map_err(|e| format!("X: the first is a number, so each must be one: {}", e.message))?;
    if let Some(bad) = numbers.iter().find(|x| !x.is_finite()) {
        return Err(format!("X has {bad}; each point's X must be a finite number"));
    }
    Ok((numbers.into_iter().map(Value::Float).collect(), false))
}

/// `value` as a list: a collection, or `$null`, as it is, and a single
/// value as a list of that one, as PowerShell binds one value to an array
/// parameter. The host tags every scalar with its own type and anything
/// else, a collection among them, as an object.
pub(crate) fn as_list(value: &PsObject) -> Result<PsObject, String> {
    if value.type_tag().map_err(|e| e.message)? == PS_TYPE_OBJECT {
        return Ok(value.clone());
    }
    vec![value.clone()].into_ps().map_err(|e| e.message)
}

/// The renderer's name for a chart type written Line, Bar or Points in any
/// case.
fn chart_type(written: &str) -> Result<&'static str, String> {
    match written.to_ascii_lowercase().as_str() {
        "line" => Ok("line"),
        "bar" => Ok("bar"),
        "points" => Ok("points"),
        _ => Err(format!("{written} is no chart type; it must be Line, Bar or Points")),
    }
}

/// What PowerShell's -is makes of a value, through any object wrapping it:
/// `string`, `ordered` for an ordered dictionary, `dictionary` for any other
/// dictionary, or `other`.
const SHAPE: &str = r#"
param($Value)
if ($Value -is [string]) { 'string' }
elseif ($Value -is [System.Collections.Specialized.OrderedDictionary]) { 'ordered' }
elseif ($Value -is [System.Collections.IDictionary]) { 'dictionary' }
else { 'other' }
"#;

/// The shape [`SHAPE`] gives `value`.
fn shape(ps: &Pipeline<'_>, value: &PsObject) -> PsResult<String> {
    let said = crate::window::script(SHAPE)?.call(ps, std::slice::from_ref(value))?;
    match said.first() {
        Some(s) => String::from_ps(s),
        None => Err(PsError::new(ErrorCategory::InvalidResult, "GuiNoShape", "the shape of a value went unread")),
    }
}

/// `value` as strings: a string is one, where reading it as a list would
/// give its characters, a list gives its items, and `$null` none.
pub(crate) fn strings(ps: &Pipeline<'_>, value: &PsObject) -> PsResult<Vec<String>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    if shape(ps, value)? == "string" { Ok(vec![String::from_ps(value)?]) } else { Vec::<String>::from_ps(value) }
}

/// The kinds of files a dialog offers, as the renderer's list of maps of a
/// `name` and its `extensions`: from `Name|*.txt;*.md` strings, or an
/// ordered dictionary from each name to its extensions. Any other
/// dictionary is refused, since it has no order and a dialog offers its
/// first kind first.
pub(crate) fn filter_of(ps: &Pipeline<'_>, given: &PsObject) -> Result<Value, String> {
    if given.is_null() {
        return Ok(Value::List(Vec::new()));
    }
    let message = |e: PsError| e.message;
    match shape(ps, given).map_err(message)?.as_str() {
        "dictionary" => {
            return Err(String::from("only an ordered dictionary has an order, and a dialog offers its first kind of file first; write [ordered]@{ 'Text files' = 'txt', 'md' } or 'Text files|*.txt;*.md'"));
        }
        "ordered" => {
            let table = PsReadOnlyTable::over(given).map_err(message)?;
            let mut out = Vec::new();
            for key in table.keys().map_err(message)? {
                let written = strings(ps, &table.get(&key).map_err(message)?).map_err(|e| format!("{key}: {}", e.message))?;
                let extensions = written.iter().map(|w| extension_of(w)).collect::<Result<Vec<String>, String>>()?;
                out.push(filter_entry(&key, extensions));
            }
            return Ok(Value::List(out));
        }
        _ => {}
    }
    let written = strings(ps, given).map_err(message)?;
    let mut out = Vec::with_capacity(written.len());
    for w in &written {
        let (name, patterns) = w.split_once('|').ok_or_else(|| format!("{w} is no kind of file; write 'Name|*.txt;*.md'"))?;
        let extensions = patterns.split(';').map(str::trim).filter(|p| !p.is_empty()).map(extension_of).collect::<Result<Vec<String>, String>>()?;
        out.push(filter_entry(name.trim(), extensions));
    }
    Ok(Value::List(out))
}

/// The ring a widget reads, from the object New-SubEtha<Noun> wrote, as
/// the renderer opens it again: by its path and the sizes it was made with.
/// It is a SubEtha.Ring, whose items travel as frames of any size, a
/// SubEtha.LocaleRing or a SubEtha.BroadcastRing, whose items each fit one
/// slot. A ring with order stamps is refused, since a reader would
/// take its stamps for the items' bytes, and a capacity ring, whose resizes
/// a reader in another process does not follow.
fn ring_of(value: &PsObject) -> Result<Value, String> {
    let message = |e: PsError| e.message;
    let count = |what: &str, v: PsObject| -> Result<i64, String> {
        let n = u64::from_ps(&v).map_err(|e| format!("its {what}: {}", e.message))?;
        i64::try_from(n).map_err(|e| format!("its {what}, {n}, is too large for a ring: {e}"))
    };
    let kind = value.type_name().map_err(message)?;
    let (kind, capacity, producers, consumers) = match kind.as_str() {
        "SubEtha.Ring" => {
            if !value.get("Stamps").map_err(message)?.is_null() {
                return Err(String::from("a ring made with -Stamps puts its order stamps in its items; make the ring without them"));
            }
            let capacity = count("capacity", value.call("Capacity", &[]).map_err(message)?)?;
            (RingKind::Ring, capacity, count("producer count", value.get("MaxProducers").map_err(message)?)?, count("consumer count", value.get("MaxConsumers").map_err(message)?)?)
        }
        "SubEtha.LocaleRing" => {
            if bool::from_ps(&value.get("Stamped").map_err(message)?).map_err(message)? {
                return Err(String::from("a ring made with -Stamped puts its order stamps in its items; make the ring without them"));
            }
            let capacity = count("capacity", value.get("Capacity").map_err(message)?)?;
            (RingKind::LocaleRing, capacity, count("producer count", value.get("MaxProducers").map_err(message)?)?, count("consumer count", value.get("MaxConsumers").map_err(message)?)?)
        }
        "SubEtha.BroadcastRing" => (RingKind::BroadcastRing, count("capacity", value.get("Capacity").map_err(message)?)?, 1, 0),
        "SubEtha.CapacityRing" => {
            return Err(String::from("a SubEtha.CapacityRing's resizes reach only the process that makes them, so a window could not follow it; take a SubEtha.Ring, LocaleRing or BroadcastRing"));
        }
        other => return Err(format!("it takes a SubEtha.Ring, SubEtha.LocaleRing or SubEtha.BroadcastRing; it was given a {other}")),
    };
    let path = String::from_ps(&value.get("Path").map_err(message)?).map_err(|e| format!("its path: {}", e.message))?;
    Ok(RingSource { kind, path, capacity, producers, consumers }.value())
}

/// One kind of file a dialog offers, as the renderer's map.
fn filter_entry(name: &str, extensions: Vec<String>) -> Value {
    Value::Map(BTreeMap::from([(String::from("name"), Value::Str(name.to_string())), (String::from("extensions"), Value::List(extensions.into_iter().map(Value::Str).collect()))]))
}

/// The extension a filter names, written `*.txt`, `.txt` or `txt`, without
/// its dot; `*` or `*.*` for any file is `*`.
fn extension_of(written: &str) -> Result<String, String> {
    let w = written.trim();
    if w == "*" || w == "*.*" {
        return Ok(String::from("*"));
    }
    let extension = match w.strip_prefix("*.") {
        Some(rest) => rest,
        None => w.strip_prefix('.').unwrap_or(w),
    };
    if extension.is_empty() || extension.contains(['*', '?', '/', '\\', ';', '|', ' ']) {
        return Err(format!("{written} is no file extension; write *.txt, .txt or txt"));
    }
    Ok(extension.to_string())
}

/// Checks a widget's renderer properties against each other: `before` are
/// those it has, `props` those being set, `current` its value. A number
/// field's or slider's Minimum must not be above its Maximum, its Step must
/// be above 0, and a Value being set must lie between them; a new range
/// with no Value moves `current` into it, to the nearer end, and the moved
/// value joins `props`. A date picker's dates follow the same rules,
/// compared as their text, which for `YYYY-MM-DDTHH:MM` is the order of the
/// dates. A password field is one line. A progress bar's Maximum must be
/// above 0 and its Value from 0 to its Maximum. A chart's new Series
/// replaces the points Add-GuiItem added and settles whether its X are
/// dates. A file chooser saves a file or chooses a folder, not both, a save
/// takes one path, a folder takes no filter, and without Multiple it takes
/// at most one path. An image's, a GPU surface's and a canvas's sides and a
/// chart's Height are above 0 points, and a scroll area's and a log's
/// Height and a spacer's Size are zero or more.
pub fn settle(kind: Kind, before: &BTreeMap<String, Value>, current: Option<&Value>, props: &mut BTreeMap<String, Value>) -> PsResult<()> {
    let name = widget_name(kind);
    match kind {
        Kind::NumberBox | Kind::Slider => {
            let number = |key: &str| match props.get(key) {
                Some(v) => v.as_number(),
                None => before.get(key).and_then(Value::as_number),
            };
            let (low, high, step) = (number("minimum"), number("maximum"), number("step"));
            if let (Some(l), Some(h)) = (low, high)
                && l > h
            {
                return Err(bad_number(format!("a {name}'s Minimum is {l}, above its Maximum {h}")));
            }
            if let Some(s) = step
                && s <= 0.0
            {
                return Err(bad_number(format!("a {name}'s Step is {s}; it must be above 0")));
            }
            let low = low.unwrap_or(f64::NEG_INFINITY);
            let high = high.unwrap_or(f64::INFINITY);
            match props.get("value").and_then(Value::as_number) {
                Some(v) if v < low => return Err(bad_number(format!("a {name}'s Value is {v}, below its Minimum {low}"))),
                Some(v) if v > high => return Err(bad_number(format!("a {name}'s Value is {v}, above its Maximum {high}"))),
                Some(_) => {}
                None => {
                    let ranged = props.contains_key("minimum") || props.contains_key("maximum");
                    if let Some(v) = current.and_then(Value::as_number)
                        && ranged
                        && (v < low || v > high)
                    {
                        props.insert(String::from("value"), Value::Float(v.clamp(low, high)));
                    }
                }
            }
        }
        Kind::TextBox => {
            let on = |key: &str| matches!(props.get(key).or_else(|| before.get(key)), Some(Value::Bool(true)));
            if on("password") && on("multiline") {
                let text = "a password field is one line; a TextBox takes Password or Multiline, not both";
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
            }
        }
        Kind::ProgressBar => {
            let number = |key: &str| match props.get(key) {
                Some(v) => v.as_number(),
                None => before.get(key).and_then(Value::as_number),
            };
            let maximum = number("maximum").unwrap_or(100.0);
            if maximum <= 0.0 {
                return Err(bad_number(format!("a ProgressBar's Maximum is {maximum}; it must be above 0")));
            }
            if let Some(v) = number("value")
                && !(0.0..=maximum).contains(&v)
            {
                return Err(bad_number(format!("a ProgressBar's Value is {v}; it must be from 0 to its Maximum {maximum}")));
            }
        }
        Kind::Image => {
            let number = |key: &str| match props.get(key) {
                Some(v) => v.as_number(),
                None => before.get(key).and_then(Value::as_number),
            };
            for (key, label) in [("width", "Width"), ("height", "Height")] {
                if let Some(n) = number(key)
                    && !(n.is_finite() && n > 0.0)
                {
                    return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadSize", format!("an Image's {label} is {n}; it must be above 0 points")).terminating());
                }
            }
            if props.contains_key("fit") && (number("width").is_none() || number("height").is_none()) {
                let text = "an Image's Fit says how it fills the box its Width and Height make, so it takes both";
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
            }
        }
        Kind::ComboBox => {
            let editable = matches!(props.get("editable").or_else(|| before.get("editable")), Some(Value::Bool(true)));
            let (refused, instead) = if editable { ("selected", "Text") } else { ("text", "Selected") };
            if props.contains_key(refused) {
                let text = if editable {
                    format!("an editable ComboBox's value is the text typed or picked, which {instead} sets; it takes no Selected")
                } else {
                    format!("a ComboBox that is not editable has the choice chosen as its value, which {instead} sets; it takes no Text")
                };
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
            }
        }
        Kind::GpuSurface | Kind::Canvas => {
            for (key, label) in [("width", "Width"), ("height", "Height")] {
                if let Some(n) = props.get(key).and_then(Value::as_number)
                    && !(n.is_finite() && n > 0.0)
                {
                    return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadSize", format!("a {name}'s {label} is {n}; it must be above 0 points")).terminating());
                }
            }
        }
        Kind::MediaPlayer => {
            for (key, label) in [("width", "Width"), ("height", "Height")] {
                if let Some(n) = props.get(key).and_then(Value::as_number)
                    && !(n.is_finite() && n > 0.0)
                {
                    return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadSize", format!("a MediaPlayer's {label} is {n}; it must be above 0 points")).terminating());
                }
            }
            if let Some(v) = props.get("volume").and_then(Value::as_number)
                && !(0.0..=1.0).contains(&v)
            {
                return Err(bad_number(format!("a MediaPlayer's Volume is {v}; it runs from 0 for silence to 1")));
            }
            if let Some(r) = props.get("rate").and_then(Value::as_number)
                && let Err(why) = media::check_rate(r)
            {
                return Err(bad_number(format!("a MediaPlayer's {why}")));
            }
        }
        Kind::Chart => {
            if let Some(h) = props.get("height").and_then(Value::as_number)
                && !(h.is_finite() && h > 0.0)
            {
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadSize", format!("a Chart's Height is {h}; it must be above 0 points")).terminating());
            }
            if let Some(series) = props.get("series").and_then(Value::as_list) {
                let dates = series_dates(series);
                props.insert(String::from("dates"), dates.map_or(Value::Null, Value::Bool));
                props.insert(String::from("points"), Value::List(Vec::new()));
            }
        }
        Kind::DatePicker => {
            let when = |key: &str| props.get(key).or_else(|| before.get(key)).and_then(Value::as_str).map(str::to_string);
            let (low, high) = (when("minimum"), when("maximum"));
            if let (Some(l), Some(h)) = (&low, &high)
                && l > h
            {
                return Err(bad_date(format!("a DatePicker's Minimum is {l}, after its Maximum {h}")));
            }
            match props.get("value").and_then(Value::as_str) {
                Some(v) => {
                    if let Some(l) = &low
                        && v < l.as_str()
                    {
                        return Err(bad_date(format!("a DatePicker's Value is {v}, before its Minimum {l}")));
                    }
                    if let Some(h) = &high
                        && v > h.as_str()
                    {
                        return Err(bad_date(format!("a DatePicker's Value is {v}, after its Maximum {h}")));
                    }
                }
                None => {
                    let ranged = props.contains_key("minimum") || props.contains_key("maximum");
                    if let Some(v) = current.and_then(Value::as_str)
                        && ranged
                        && !props.contains_key("value")
                    {
                        let moved = match (&low, &high) {
                            (Some(l), _) if v < l.as_str() => Some(l.clone()),
                            (_, Some(h)) if v > h.as_str() => Some(h.clone()),
                            _ => None,
                        };
                        if let Some(m) = moved {
                            props.insert(String::from("value"), Value::Str(m));
                        }
                    }
                }
            }
        }
        Kind::FilePicker => {
            let on = |key: &str| matches!(props.get(key).or_else(|| before.get(key)), Some(Value::Bool(true)));
            let refuse = |text: String| Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
            if on("save") && on("folder") {
                return refuse(String::from("a FilePicker chooses a file to save or a folder, not both; it takes Save or Folder"));
            }
            if on("save") && on("multiple") {
                return refuse(String::from("a save dialog chooses one file; a FilePicker takes Save or Multiple, not both"));
            }
            let filtered = props.get("filter").or_else(|| before.get("filter")).and_then(Value::as_list).is_some_and(|f| !f.is_empty());
            if on("folder") && filtered {
                return refuse(String::from("a folder dialog shows no files to filter; a FilePicker takes Folder or Filter, not both"));
            }
            let paths = props.get("paths").or_else(|| before.get("paths")).and_then(Value::as_list).map_or(0, <[Value]>::len);
            if paths > 1 && !on("multiple") {
                return refuse(format!("a FilePicker without Multiple takes one path; it was given {paths}"));
            }
        }
        Kind::Scroll | Kind::LogView | Kind::Spacer => {
            let sizes: &[(&str, &str)] = match kind {
                Kind::Spacer => &[("size", "Size")],
                Kind::LogView => &[("width", "Width"), ("height", "Height")],
                _ => &[("height", "Height")],
            };
            for &(key, label) in sizes {
                if let Some(n) = props.get(key).and_then(Value::as_number)
                    && (n.is_nan() || n < 0.0)
                {
                    return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadSize", format!("a {name}'s {label} is {n}; it must be zero or more points")).terminating());
                }
            }
        }
        Kind::Spinner
        | Kind::ColorPicker
        | Kind::MenuBar
        | Kind::Menu
        | Kind::MenuItem
        | Kind::ContextMenu
        | Kind::StatusBar
        | Kind::ButtonBar
        | Kind::TreeView
        | Kind::TreeItem
        | Kind::Markdown => {}
        Kind::Column
        | Kind::Row
        | Kind::Grid
        | Kind::Group
        | Kind::Tabs
        | Kind::Tab
        | Kind::Split
        | Kind::Text
        | Kind::Button
        | Kind::ListBox
        | Kind::Table
        | Kind::CheckBox
        | Kind::RadioGroup
        | Kind::Separator
        | Kind::Html => {}
    }
    Ok(())
}

/// Checks a number field's or slider's numbers as Show-GuiWindow would, so
/// the command describing it refuses them.
fn check_numbers(kind: Kind, value: f64, minimum: Option<f64>, maximum: Option<f64>, step: Option<f64>) -> PsResult<()> {
    let mut props = BTreeMap::new();
    for (label, key, n) in [("Value", "value", Some(value)), ("Minimum", "minimum", minimum), ("Maximum", "maximum", maximum), ("Step", "step", step)] {
        if let Some(n) = n {
            if !n.is_finite() {
                return Err(bad_number(format!("a {}'s {label} is {n}; it must be a finite number", widget_name(kind))));
            }
            props.insert(key.to_string(), Value::Float(n));
        }
    }
    settle(kind, &BTreeMap::new(), None, &mut props)
}

/// `v`, or the nearer of `minimum` and `maximum` when it is outside them.
fn nearest(v: f64, minimum: Option<f64>, maximum: Option<f64>) -> f64 {
    match (minimum, maximum) {
        (Some(low), _) if v < low => low,
        (_, Some(high)) if v > high => high,
        _ => v,
    }
}

/// Fails when `selected` is not one of `choices`, compared without regard
/// to case.
fn check_selected(selected: Option<&String>, choices: &[String]) -> PsResult<()> {
    match selected {
        Some(s) if !choices.iter().any(|c| c.eq_ignore_ascii_case(s)) => {
            let text = format!("-Selected is {s}, which is not one of the choices: {}", choices.join(", "));
            Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating())
        }
        _ => Ok(()),
    }
}

/// One widget of a window described in PowerShell, as a New-Gui<Widget>
/// command writes it and Show-GuiWindow -Content takes it.
#[psclass(name = "PwrsGui.Widget")]
#[derive(Clone)]
pub struct GuiWidget {
    /// What the widget is, by its command's name, such as Text or
    /// ListBox.
    pub kind: String,
    /// The name it goes by in State, the handlers' events and the update
    /// commands; null for none.
    pub name: Option<String>,
    /// Its properties, by the names its command's parameters give them.
    pub properties: PsObject,
    /// The widgets inside it, in order.
    pub children: Vec<PsObject>,
    /// Its handlers, by the event each runs for: Click, Change, Select,
    /// Activate, Submit, Expand, Drop or Key.
    pub handlers: PsObject,
    /// For a button, the window's result when it is pressed, which closes
    /// the window; null for none.
    pub result: Option<PsObject>,
}

/// A widget's description: its properties and the handlers given, each by
/// its name.
fn describe(kind: Kind, name: Option<String>, props: Vec<(&str, PsObject)>, children: Vec<PsObject>, handlers: Vec<(&str, Option<&PsScriptBlock>)>, result: Option<PsObject>) -> PsResult<GuiWidget> {
    let properties = ordered(props.into_iter().map(|(k, v)| (k.to_string(), v)).collect())?;
    let mut given = Vec::new();
    for (event, handler) in handlers {
        if let Some(h) = handler {
            given.push((event.to_string(), h.0.clone()));
        }
    }
    let handlers = ordered(given)?;
    Ok(GuiWidget { kind: widget_name(kind).to_string(), name, properties, children, handlers, result })
}

/// The widgets `content` writes, in order, for the widget `kind` that
/// contains them; anything else it writes is an error, and `$null` is passed over.
fn children_of(ps: &Pipeline<'_>, kind: Kind, content: Option<&PsScriptBlock>) -> PsResult<Vec<PsObject>> {
    let Some(block) = content else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for o in block.call(ps, &[])? {
        if o.is_null() {
            continue;
        }
        let type_name = o.type_name()?;
        if type_name != "PwrsGui.Widget" {
            let text = format!(
                "the block of a {} wrote a {type_name}, which is not a widget; the widgets inside come from the short names, such as Text and Button, or from New-Gui<Widget>",
                widget_name(kind)
            );
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotAWidget", text).terminating());
        }
        out.push(o);
    }
    Ok(out)
}

/// The text style of a Text widget.
#[psenum(name = "PwrsGui.TextStyle")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiTextStyle {
    /// The body text.
    #[default]
    Body,
    /// A heading.
    Heading,
    /// A monospaced font.
    Mono,
    /// A link, which runs -OnClick when clicked.
    Link,
    /// A dialog's main line, larger than the body text and in its strong
    /// color.
    Instruction,
}

impl GuiTextStyle {
    fn name(self) -> &'static str {
        match self {
            GuiTextStyle::Body => "Body",
            GuiTextStyle::Heading => "Heading",
            GuiTextStyle::Mono => "Mono",
            GuiTextStyle::Link => "Link",
            GuiTextStyle::Instruction => "Instruction",
        }
    }
}

/// Where a Row, Column or Grid places its widgets across.
#[psenum(name = "PwrsGui.Align")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiAlign {
    /// At the left.
    #[default]
    Start,
    /// In the middle.
    Center,
    /// At the right.
    End,
    /// Across the room: a Column's and a Grid's widgets as wide as it, a
    /// Row's sharing it equally.
    Stretch,
}

impl GuiAlign {
    fn name(self) -> &'static str {
        match self {
            GuiAlign::Start => "Start",
            GuiAlign::Center => "Center",
            GuiAlign::End => "End",
            GuiAlign::Stretch => "Stretch",
        }
    }
}

/// `props` with `align` among them, when one is given.
fn with_align(mut props: Vec<(&'static str, PsObject)>, align: Option<GuiAlign>) -> PsResult<Vec<(&'static str, PsObject)>> {
    if let Some(a) = align {
        props.push(("Align", a.name().into_ps()?));
    }
    Ok(props)
}

/// Describes widgets stacked top to bottom: those its block writes, in
/// order, each placed across as -Align says. A list, table or tree, and a
/// widget containing one, shares the height the others leave, and scrolls
/// rather than push them out of the window. In Show-GuiWindow -Content,
/// Column stands for it.
///
/// # Examples
/// Show-GuiWindow { Column { CheckBox 'Logs' -Name Logs; CheckBox 'Cache' -Name Cache }; ButtonBar { Button 'OK' -Default -Result Ok } }
/// New-GuiColumn -Align Stretch { New-GuiText 'Name'; New-GuiTextBox -Name Name }
#[cmdlet(verb = "New", noun = "GuiColumn", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiColumn {
    /// The block that writes the widgets inside, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// Where each widget sits across: Start, at the left, when not given;
    /// Center; End, at the right; or Stretch, as wide as the Column.
    #[param]
    pub align: Option<GuiAlign>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiColumn {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::Column, self.content.as_ref())?;
        let props = with_align(Vec::new(), self.align)?;
        ps.write(describe(Kind::Column, self.name.clone(), props, children, vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes widgets side by side: those its block writes, in order, each
/// centered top to bottom and placed as a group across as -Align says. A
/// Spacer -Fill among them takes the room the others leave. In
/// Show-GuiWindow -Content, Row stands for it.
///
/// # Examples
/// Show-GuiWindow { Row -Align End { Button 'Apply' -Name Apply; Button 'Reset' -Name Reset } }
#[cmdlet(verb = "New", noun = "GuiRow", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiRow {
    /// The block that writes the widgets inside, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// Where the widgets sit across: Start, at the left, when not given;
    /// Center; End, at the right; or Stretch, sharing the Row's width
    /// equally.
    #[param]
    pub align: Option<GuiAlign>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiRow {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::Row, self.content.as_ref())?;
        let props = with_align(Vec::new(), self.align)?;
        ps.write(describe(Kind::Row, self.name.clone(), props, children, vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes widgets in a grid of -Columns columns, filled a row at a time
/// from those its block writes, in order, each placed in its cell as -Align
/// says. In Show-GuiWindow -Content, Grid stands for it.
///
/// # Examples
/// Show-GuiWindow { Grid -Columns 2 -Align Stretch { Text 'Server'; TextBox -Name Server; Text 'Copies'; NumberBox 1 -Name Copies } }
#[cmdlet(verb = "New", noun = "GuiGrid", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiGrid {
    /// The block that writes the widgets inside, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// How many columns, a whole number above 0.
    #[param(mandatory)]
    pub columns: i64,
    /// Where each widget sits in its cell: Start, at the left, when not
    /// given; Center; End, at the right; or Stretch, as wide as its column,
    /// the last column taking the room left across.
    #[param]
    pub align: Option<GuiAlign>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiGrid {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.columns < 1 {
            let text = format!("a Grid's -Columns is {}; it must be a whole number above 0", self.columns);
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
        }
        let children = children_of(ps, Kind::Grid, self.content.as_ref())?;
        let props = with_align(vec![("Columns", self.columns.into_ps()?)], self.align)?;
        ps.write(describe(Kind::Grid, self.name.clone(), props, children, vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes widgets stacked in a frame, under -Title when it is given:
/// those its block writes, in order. In Show-GuiWindow -Content, Group
/// stands for it.
///
/// # Examples
/// Show-GuiWindow { Group -Title 'Options' { CheckBox 'Dry run' -Name DryRun; CheckBox 'Verbose' -Name Verbose } }
#[cmdlet(verb = "New", noun = "GuiGroup", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiGroup {
    /// The block that writes the widgets inside, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// The title above the widgets; none when not given.
    #[param]
    pub title: Option<String>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiGroup {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::Group, self.content.as_ref())?;
        let mut props = Vec::new();
        if let Some(t) = &self.title {
            props.push(("Title", t.clone().into_ps()?));
        }
        ps.write(describe(Kind::Group, self.name.clone(), props, children, vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes tabs, one shown at a time: the Tab widgets its block writes,
/// in order. State gives the title of the tab shown. In Show-GuiWindow
/// -Content, Tabs stands for it.
///
/// # Examples
/// Show-GuiWindow { Tabs -Name Pages { Tab 'General' { TextBox -Name Server }; Tab 'Advanced' { CheckBox 'Verbose' -Name Verbose } } }
#[cmdlet(verb = "New", noun = "GuiTabs", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiTabs {
    /// The block that writes the tabs, Tab widgets, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// The tab shown when the window opens, named by its title without
    /// regard to case; the first when not given.
    #[param]
    pub selected: Option<String>,
    /// Runs when a person chooses a tab, with the event as its argument;
    /// its Value is the tab's title.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiTabs {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::Tabs, self.content.as_ref())?;
        let mut titles = Vec::with_capacity(children.len());
        for c in &children {
            let tab = GuiWidget::from_ps(c)?;
            if !tab.kind.eq_ignore_ascii_case("Tab") {
                let text = format!("a Tabs contains only Tab widgets, and its block wrote a {}", tab.kind);
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotATab", text).terminating());
            }
            titles.push(String::from_ps(&PsReadOnlyTable::over(&tab.properties)?.get("Title")?)?);
        }
        check_selected(self.selected.as_ref(), &titles)?;
        let mut props = Vec::new();
        if let Some(s) = &self.selected {
            props.push(("Selected", s.clone().into_ps()?));
        }
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref())];
        ps.write(describe(Kind::Tabs, self.name.clone(), props, children, handlers, None)?)
    }
}

/// Describes one tab of a Tabs: its title, and the widgets its block
/// writes, stacked, shown while the tab is chosen. In Show-GuiWindow
/// -Content, Tab stands for it.
///
/// # Examples
/// Show-GuiWindow { Tabs { Tab 'General' { Text 'Shown first' }; Tab 'More' { Text 'Shown when chosen' } } }
#[cmdlet(verb = "New", noun = "GuiTab", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiTab {
    /// The tab's title.
    #[param(mandatory, position = 0)]
    pub title: String,
    /// The block that writes the widgets inside, in order.
    #[param(position = 1)]
    pub content: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiTab {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::Tab, self.content.as_ref())?;
        ps.write(describe(Kind::Tab, self.name.clone(), vec![("Title", self.title.clone().into_ps()?)], children, vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// How a Split places its two widgets.
#[psenum(name = "PwrsGui.Orientation")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiOrientation {
    /// Side by side.
    #[default]
    Horizontal,
    /// One above the other.
    Vertical,
}

impl GuiOrientation {
    fn name(self) -> &'static str {
        match self {
            GuiOrientation::Horizontal => "Horizontal",
            GuiOrientation::Vertical => "Vertical",
        }
    }
}

/// Describes two widgets side by side, or with -Orientation Vertical one
/// above the other, with a divider between them that a person drags: the
/// two its block writes. A vertical split is as high as the window leaves
/// it. In Show-GuiWindow -Content, Split stands for it.
///
/// # Examples
/// Show-GuiWindow { Split -Position 0.3 { ListBox 'a', 'b' -Name Pick; Text 'Details' -Name Details } }
#[cmdlet(verb = "New", noun = "GuiSplit", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiSplit {
    /// The block that writes the two widgets, the first on the left or on
    /// top.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// Horizontal, side by side, or Vertical, one above the other;
    /// Horizontal when not given.
    #[param]
    pub orientation: Option<GuiOrientation>,
    /// The share of the space the first widget takes, from 0 to 1; 0.5
    /// when not given.
    #[param]
    pub position: Option<f64>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiSplit {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::Split, self.content.as_ref())?;
        if children.len() != 2 {
            let text = format!("a Split contains two widgets, and its block wrote {}", children.len());
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotTwoWidgets", text).terminating());
        }
        let mut props = Vec::new();
        if let Some(o) = self.orientation {
            props.push(("Orientation", o.name().into_ps()?));
        }
        if let Some(p) = self.position {
            if !(0.0..=1.0).contains(&p) {
                return Err(bad_number(format!("a Split's -Position is {p}; it must be from 0 to 1")));
            }
            props.push(("Position", p.into_ps()?));
        }
        ps.write(describe(Kind::Split, self.name.clone(), props, children, vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes widgets stacked in an area that scrolls: those its block
/// writes, in order. Without -Height, the area is as high as the window
/// leaves it. In Show-GuiWindow -Content, Scroll stands for it.
///
/// # Examples
/// Show-GuiWindow { Scroll -Height 200 { 1..50 | ForEach-Object { Text "Line $_" } } }
#[cmdlet(verb = "New", noun = "GuiScroll", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiScroll {
    /// The block that writes the widgets inside, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// The most the area is high, in points, zero or more.
    #[param]
    pub height: Option<f64>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiScroll {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::Scroll, self.content.as_ref())?;
        let mut props = Vec::new();
        if let Some(h) = self.height {
            settle(Kind::Scroll, &BTreeMap::new(), None, &mut BTreeMap::from([(String::from("height"), Value::Float(h))]))?;
            props.push(("Height", h.into_ps()?));
        }
        ps.write(describe(Kind::Scroll, self.name.clone(), props, children, vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes a line of text, or with -Style Link a link that runs -OnClick
/// when clicked. In Show-GuiWindow -Content, Text stands for it.
///
/// # Examples
/// Show-GuiWindow { Text 'Pick a target' -Style Heading; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { Text 'Open the log' -Style Link -OnClick { Invoke-Item $env:TEMP\deploy.log } }
#[cmdlet(verb = "New", noun = "GuiText", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiText {
    /// The text shown.
    #[param(mandatory, position = 0, allow_empty_string)]
    pub text: String,
    /// Body, Heading, Mono, Link or Instruction, a dialog's main line; Body
    /// when not given.
    #[param]
    pub style: Option<GuiTextStyle>,
    /// With -Style Link, runs when the link is clicked, with the event as
    /// its argument.
    #[param]
    pub on_click: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiText {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.on_click.is_some() && self.style != Some(GuiTextStyle::Link) {
            let text = "-OnClick runs when a link is clicked, and only a Text with -Style Link is one";
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
        }
        let mut props = vec![("Text", self.text.clone().into_ps()?)];
        if let Some(s) = self.style {
            props.push(("Style", s.name().into_ps()?));
        }
        let handlers = vec![("Click", self.on_click.as_ref()), ("Drop", self.on_drop.as_ref())];
        ps.write(describe(Kind::Text, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a Markdown document: the text -Text gives, or the file -Path
/// names, which is read now, as UTF-8 or as UTF-16 when its byte order mark
/// says so. Its images come from files beside it, absolute paths, data in
/// the address, and https addresses the renderer fetches; one the module
/// cannot read is written as a warning and shows its alt text. A link
/// raises Click, which -OnClick acts on, with the link's address as the
/// event's Value; nothing opens on its own. In Show-GuiWindow -Content,
/// Markdown stands for it.
///
/// # Examples
/// Show-GuiWindow { Markdown -Path .\README.md -OnClick { param($e) Start-Process $e.Value } }
/// Show-GuiWindow { Markdown "# Deploy`n`nTargets: **web01**, **web02**`n`n![status](status.png)" }
#[cmdlet(verb = "New", noun = "GuiMarkdown", default_parameter_set = "Text", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiMarkdown {
    /// The document's text, CommonMark with tables, task lists,
    /// strikethrough, footnotes and definition lists. Its relative images
    /// are found from the current location.
    #[param(mandatory, position = 0, set = "Text", allow_empty_string)]
    pub text: Option<String>,
    /// The document's file, resolved from the current location as
    /// PowerShell resolves a path, with wildcards taken literally. Its
    /// relative images are found from the file's folder.
    #[param(mandatory, set = "Path")]
    pub path: Option<String>,
    /// Runs when a link is clicked, with the event as its argument; its
    /// Value is the link's address.
    #[param]
    pub on_click: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiMarkdown {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let props = match (&self.text, &self.path) {
            (Some(text), _) => vec![("Text", text.clone().into_ps()?)],
            (None, Some(path)) => {
                let (_, full) = document_file(ps, path).map_err(|e| PsError::new(ErrorCategory::ObjectNotFound, "GuiNoFile", e).terminating())?;
                vec![("Path", full.to_string_lossy().into_owned().into_ps()?)]
            }
            (None, None) => return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoDocument", "a Markdown shows the text -Text gives or the file -Path names").terminating()),
        };
        let handlers = vec![("Click", self.on_click.as_ref()), ("Drop", self.on_drop.as_ref())];
        ps.write(describe(Kind::Markdown, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// How an Image fills the box its -Width and -Height make.
#[psenum(name = "PwrsGui.ImageFit")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiImageFit {
    /// The whole picture inside the box, its proportions unchanged.
    #[default]
    Contain,
    /// The box filled, the picture's proportions unchanged and what spills over
    /// cropped.
    Cover,
    /// The picture stretched to the box.
    Fill,
    /// The picture at its own size, centered and cropped to the box.
    None,
}

impl GuiImageFit {
    fn name(self) -> &'static str {
        match self {
            GuiImageFit::Contain => "Contain",
            GuiImageFit::Cover => "Cover",
            GuiImageFit::Fill => "Fill",
            GuiImageFit::None => "None",
        }
    }
}

/// Describes a picture: the file -Path names, which is read now, or the
/// bytes -Bytes gives, a PNG, JPEG, GIF or WebP (both animated), BMP, ICO or
/// SVG, told by its bytes. It shows at its size in pixels as points, shrunk
/// to fit the space it gets; -Width or -Height sets that side and scales
/// the other, and both make a box that -Fit fills. In Show-GuiWindow
/// -Content, Image stands for it.
///
/// # Examples
/// Show-GuiWindow { Image .\logo.png; Text 'Deploy to production?' }
/// Show-GuiWindow { Image -Bytes ([IO.File]::ReadAllBytes("$HOME\chart.png")) -Width 320 -Height 200 -Fit Cover -AltText 'This week''s deploys' }
#[cmdlet(verb = "New", noun = "GuiImage", default_parameter_set = "Path", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiImage {
    /// The picture's file, resolved from the current location as PowerShell
    /// resolves a path, with wildcards taken literally.
    #[param(mandatory, position = 0, set = "Path")]
    pub path: Option<String>,
    /// The picture's bytes.
    #[param(mandatory, set = "Bytes")]
    pub bytes: Option<Vec<u8>>,
    /// The picture's width in points; with no -Height, its height follows
    /// from its proportions.
    #[param]
    pub width: Option<f64>,
    /// The picture's height in points; with no -Width, its width follows
    /// from its proportions.
    #[param]
    pub height: Option<f64>,
    /// How the picture fills the box -Width and -Height make: Contain, the
    /// whole picture inside it; Cover, the box filled and what spills over
    /// cropped; Fill, the picture stretched to it; None, the picture at its
    /// own size, centered and cropped. Contain when not given.
    #[param]
    pub fit: Option<GuiImageFit>,
    /// The picture's text, which a screen reader reads and which shows
    /// where the picture cannot be drawn.
    #[param]
    pub alt_text: Option<String>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiImage {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let bytes = match (&self.path, &self.bytes) {
            (Some(path), _) => picture_file(ps, path).map_err(|e| PsError::new(ErrorCategory::ObjectNotFound, "GuiNoFile", e).terminating())?,
            (None, Some(bytes)) => bytes.clone(),
            (None, None) => return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoPicture", "an Image shows the file -Path names or the bytes -Bytes gives").terminating()),
        };
        if let Err(e) = picture_format(&bytes) {
            let text = match &self.path {
                Some(path) => format!("{path}: {e}"),
                None => e,
            };
            return Err(PsError::new(ErrorCategory::InvalidData, "GuiBadImage", text).terminating());
        }
        for (label, given) in [("width", self.width), ("height", self.height)] {
            if let Some(n) = given
                && !(n.is_finite() && n > 0.0)
            {
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadSize", format!("an Image's {label} is {n}; it must be above 0 points")).terminating());
            }
        }
        if self.fit.is_some() && (self.width.is_none() || self.height.is_none()) {
            let text = "-Fit says how the picture fills the box -Width and -Height make, so it takes both";
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
        }
        let mut props = vec![("Bytes", bytes.into_ps()?)];
        if let Some(w) = self.width {
            props.push(("Width", w.into_ps()?));
        }
        if let Some(h) = self.height {
            props.push(("Height", h.into_ps()?));
        }
        if let Some(f) = self.fit {
            props.push(("Fit", f.name().into_ps()?));
        }
        if let Some(a) = &self.alt_text {
            props.push(("AltText", a.clone().into_ps()?));
        }
        ps.write(describe(Kind::Image, self.name.clone(), props, Vec::new(), vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes a progress bar: a thin bar filled to -Value out of -Maximum,
/// 100 when not given, under a line with -Text at the left and the
/// percentage done at the right. Update-GuiWidget moves it. In
/// Show-GuiWindow -Content, ProgressBar stands for it.
///
/// # Examples
/// $w = Show-GuiWindow -NoWait { ProgressBar -Name Copy -Text 'Copying' }; Update-GuiWidget -Window $w.Id -Name Copy -Property @{ Value = 50 }
#[cmdlet(verb = "New", noun = "GuiProgressBar", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiProgressBar {
    /// How far the work has gone, from 0 to -Maximum; 0 when not given.
    #[param(position = 0)]
    pub value: f64,
    /// The value at which the work is done, above 0; 100 when not given.
    #[param]
    pub maximum: Option<f64>,
    /// The text over the bar at the left, cut short where it would reach the
    /// percentage at the right; none when not given.
    #[param]
    pub text: Option<String>,
    /// A SubEtha ring another process writes, each item moving the bar: a
    /// JSON object of any of its Value, Maximum and Text. A SubEtha.Ring
    /// takes items sent with SendFrame, a SubEtha.LocaleRing moved to File
    /// or ShmFs and a SubEtha.BroadcastRing items of one slot; it is taken
    /// when the window opens.
    #[param]
    pub ring: Option<PsObject>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiProgressBar {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut wire = BTreeMap::from([(String::from("value"), Value::Float(self.value))]);
        if let Some(m) = self.maximum {
            wire.insert(String::from("maximum"), Value::Float(m));
        }
        settle(Kind::ProgressBar, &BTreeMap::new(), None, &mut wire)?;
        let mut props = vec![("Value", self.value.into_ps()?)];
        if let Some(m) = self.maximum {
            props.push(("Maximum", m.into_ps()?));
        }
        if let Some(t) = &self.text {
            props.push(("Text", t.clone().into_ps()?));
        }
        if let Some(r) = &self.ring {
            props.push(("Ring", checked_ring(r)?));
        }
        ps.write(describe(Kind::ProgressBar, self.name.clone(), props, Vec::new(), vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// `ring` as a widget's -Ring, checked as [`ring_of`] reads it when the
/// window opens, so a ring no widget can read is refused as it is given.
fn checked_ring(ring: &PsObject) -> PsResult<PsObject> {
    ring_of(ring).map_err(|e| PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", format!("Ring: {e}")).terminating())?;
    Ok(ring.clone())
}

/// Describes a spinner, which turns while it is shown, for work whose end
/// is not known. In Show-GuiWindow -Content, Spinner stands for it.
///
/// # Examples
/// Show-GuiWindow -NoWait { Row { Spinner; Text 'Waiting for the server' } }
#[cmdlet(verb = "New", noun = "GuiSpinner", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiSpinner {
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiSpinner {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        ps.write(describe(Kind::Spinner, self.name.clone(), Vec::new(), Vec::new(), vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes a log of lines, monospaced and scrolled to the newest unless
/// a person scrolls up: those -Line gives and those Add-GuiItem adds.
/// -Width and -Height give its size, which stays the same however many
/// lines it shows; without -Width it is as wide as the window leaves it,
/// and without -Height as high. In Show-GuiWindow -Content, LogView
/// stands for it.
///
/// # Examples
/// $w = Show-GuiWindow -NoWait { LogView -Name Log -Height 200 }; Add-GuiItem -Window $w.Id -Name Log -Item 'started'
#[cmdlet(verb = "New", noun = "GuiLogView", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiLogView {
    /// The lines shown when the window opens; none when not given.
    #[param(position = 0)]
    pub line: Vec<String>,
    /// How wide it is, in points, zero or more.
    #[param]
    pub width: Option<f64>,
    /// How high it is, in points, zero or more.
    #[param]
    pub height: Option<f64>,
    /// A SubEtha ring another process writes, each item one line, its line
    /// break at the end dropped. A SubEtha.Ring takes items sent with
    /// SendFrame, a SubEtha.LocaleRing moved to File or ShmFs and a
    /// SubEtha.BroadcastRing items of one slot; it is taken when the window
    /// opens.
    #[param]
    pub ring: Option<PsObject>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiLogView {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = vec![("Line", self.line.clone().into_ps()?)];
        if let Some(w) = self.width {
            settle(Kind::LogView, &BTreeMap::new(), None, &mut BTreeMap::from([(String::from("width"), Value::Float(w))]))?;
            props.push(("Width", w.into_ps()?));
        }
        if let Some(h) = self.height {
            settle(Kind::LogView, &BTreeMap::new(), None, &mut BTreeMap::from([(String::from("height"), Value::Float(h))]))?;
            props.push(("Height", h.into_ps()?));
        }
        if let Some(r) = &self.ring {
            props.push(("Ring", checked_ring(r)?));
        }
        ps.write(describe(Kind::LogView, self.name.clone(), props, Vec::new(), vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// How a chart draws a series.
#[psenum(name = "PwrsGui.ChartType")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiChartType {
    /// A line through the points.
    #[default]
    Line,
    /// A bar at each point.
    Bar,
    /// A mark at each point.
    Points,
}

impl GuiChartType {
    fn name(self) -> &'static str {
        match self {
            GuiChartType::Line => "Line",
            GuiChartType::Bar => "Bar",
            GuiChartType::Points => "Points",
        }
    }
}

/// Describes a chart of -Series, each a hashtable of its Name, its Y
/// numbers, its X, numbers or dates, as many as Y (0, 1, 2 and on when not
/// given), and its Type, Line, Bar or Points (-Type when not given). A
/// person pans and zooms with the pointer and the wheel and reads the
/// values under the pointer, and a click on a point raises Select, whose
/// Value has its Series, X and Y. Add-GuiItem adds points. Without
/// -Height it is as high as the window leaves it. In Show-GuiWindow
/// -Content, Chart stands for it.
///
/// # Examples
/// Show-GuiWindow { Chart -Series @{ Name = 'cpu'; Y = 3, 5, 4, 6 } -YLabel 'percent' }
/// $w = Show-GuiWindow -NoWait { Chart -Name Load -Height 300 }; Add-GuiItem -Window $w.Id -Name Load -Item @{ Series = 'load'; X = (Get-Date); Y = 0.7 }
#[cmdlet(verb = "New", noun = "GuiChart", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiChart {
    /// The series, each a hashtable of Name and Y, and of X and Type when
    /// given; none when not given.
    #[param(position = 0)]
    pub series: Vec<PsObject>,
    /// How a series without a Type of its own is drawn; Line when not given.
    #[param]
    pub type_: Option<GuiChartType>,
    /// The name of the horizontal axis.
    #[param]
    pub x_label: Option<String>,
    /// The name of the vertical axis.
    #[param]
    pub y_label: Option<String>,
    /// How high it is, in points, above 0.
    #[param]
    pub height: Option<f64>,
    /// A SubEtha ring another process writes, each item one point: a JSON
    /// object of its Series, Y and X, a number or a date and time written
    /// as ISO 8601, as Add-GuiItem takes them. A SubEtha.Ring takes items
    /// sent with SendFrame, a SubEtha.LocaleRing moved to File or ShmFs and
    /// a SubEtha.BroadcastRing items of one slot; it is taken when the
    /// window opens.
    #[param]
    pub ring: Option<PsObject>,
    /// Runs when a person clicks a point, with the event as its argument;
    /// its Value has the point's Series, X and Y.
    #[param]
    pub on_select: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiChart {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = vec![("Series", self.series.clone().into_ps()?)];
        if let Some(t) = self.type_ {
            props.push(("Type", t.name().into_ps()?));
        }
        if let Some(l) = &self.x_label {
            props.push(("XLabel", l.clone().into_ps()?));
        }
        if let Some(l) = &self.y_label {
            props.push(("YLabel", l.clone().into_ps()?));
        }
        if let Some(h) = self.height {
            props.push(("Height", h.into_ps()?));
        }
        if let Some(r) = &self.ring {
            props.push(("Ring", r.clone()));
        }
        let widget = describe(Kind::Chart, self.name.clone(), props, Vec::new(), vec![("Select", self.on_select.as_ref()), ("Drop", self.on_drop.as_ref())], None)?;
        wire_props(ps, Kind::Chart, &widget.properties, &[], true).and_then(|mut wire| settle(Kind::Chart, &BTreeMap::new(), None, &mut wire))?;
        ps.write(widget)
    }
}

/// Describes a surface the session's WGSL draws on with the egui renderer's
/// GPU: -Width by -Height points, or when they are not given the room its
/// place leaves it across and the height the window leaves it. It shows
/// its window's own background until Set-GuiGpuSurface gives it its draws,
/// which it draws each frame, and Save-GuiGpuSurface writes what it drew to
/// a PNG; both find it by -Name. In Show-GuiWindow -Content, GpuSurface
/// stands for it.
///
/// Its pointer handlers run with an event whose Value gives where, as X
/// and Y in physical pixels from its top left, as the frame uniform's
/// pointer, U and V as fractions of its width and height, the same at any
/// display scale, and the display's PixelsPerPoint, with the Button and
/// the Shift, Ctrl and Alt keys down; a wheel turn also gives its Notches.
/// Each kind is sent only when its handler is given, a move once a frame
/// at most, and a gesture the surface's camera makes is reported too.
///
/// # Examples
/// $w = Show-GuiWindow -NoWait { GpuSurface View -Width 320 -Height 240 }; Set-GuiGpuSurface View $wgsl -VertexCount 3 -Window $w.Id
/// GpuSurface View -OnPointerDown { param($e) Write-Host "$($e.Value.Button) pressed at $($e.Value.U), $($e.Value.V)" }
#[cmdlet(verb = "New", noun = "GuiGpuSurface", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiGpuSurface {
    /// The name it goes by in Set-GuiGpuSurface, Save-GuiGpuSurface, the
    /// handlers' events and the update commands.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// Its width in points, above 0; the room its place leaves it when not
    /// given.
    #[param]
    pub width: Option<f64>,
    /// Its height in points, above 0; the height the window leaves it when
    /// not given.
    #[param]
    pub height: Option<f64>,
    /// Runs when a pointer button is pressed over it, with the event as its argument.
    #[param]
    pub on_pointer_down: Option<PsScriptBlock>,
    /// Runs when a button pressed over it is released, wherever the pointer is, with the event as its argument.
    #[param]
    pub on_pointer_up: Option<PsScriptBlock>,
    /// Runs when the pointer moves over it, or anywhere while a button pressed over it stays down, once a frame at most, with the event as its argument.
    #[param]
    pub on_pointer_move: Option<PsScriptBlock>,
    /// Runs when the wheel turns over it, with the event as its argument; its Value's Notches are positive when the wheel turns away from the person.
    #[param]
    pub on_wheel: Option<PsScriptBlock>,
    /// Runs when a key is pressed while it has the focus, which a click gives it, with the event as its argument; its Value is the key written as a shortcut, as `Ctrl+K`.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
}

impl Cmdlet for NewGuiGpuSurface {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut wire = BTreeMap::new();
        let mut props = Vec::new();
        for (key, label, given) in [("width", "Width", self.width), ("height", "Height", self.height)] {
            if let Some(n) = given {
                wire.insert(key.to_string(), Value::Float(n));
                props.push((label, n.into_ps()?));
            }
        }
        settle(Kind::GpuSurface, &BTreeMap::new(), None, &mut wire)?;
        let handlers = vec![
            ("PointerDown", self.on_pointer_down.as_ref()),
            ("PointerUp", self.on_pointer_up.as_ref()),
            ("PointerMove", self.on_pointer_move.as_ref()),
            ("Wheel", self.on_wheel.as_ref()),
            ("Key", self.on_key.as_ref()),
            ("Drop", self.on_drop.as_ref()),
        ];
        ps.write(describe(Kind::GpuSurface, Some(self.name.clone()), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a canvas of -Item, hashtables each naming its Kind, its Name
/// when a handler or an update command is to find it, and its keys: a
/// Rect, Ellipse or Image by its box, X, Y, Width and Height, a
/// RoundedRect adding Radius, a Line by X1, Y1, X2 and Y2, a Polyline or
/// Polygon by Points, x and y pairs, a Path by Data, SVG path data, a Text
/// by X, Y, Text and Size, an Image's picture by Path or Bytes, as the
/// Image widget takes them, and a Group's items under Children, with Clip,
/// X and Y; any item takes Fill, Stroke, StrokeWidth, Dash, Cap, Join,
/// Opacity, Rotate, Scale, Skew, Origin, Visible, Order and Animate. The
/// first item is drawn first, underneath, and a mistake is refused here,
/// naming the item. While its window is open, Update-GuiWidget -Item
/// changes one item by its name, Add-GuiItem adds items, into a Group with
/// -Parent, and Remove-GuiItem takes them away by name; another process
/// changes them through the SubEtha ring -Ring names, each of its items
/// making the changes Update-GuiWidget -Item would, and sets their numbers
/// through the shared memory -SharedMemory names, row by row. Without
/// -Width and -Height it takes the room its place leaves it across and the
/// height the window leaves it. In Show-GuiWindow -Content, Canvas stands
/// for it.
///
/// Its item handlers run for the topmost named item under the pointer, and
/// its pointer and wheel handlers for the canvas itself, each with an
/// event whose Value gives where as a GpuSurface's does, X and Y in
/// physical pixels from its top left, U and V as fractions of its width
/// and height and the display's PixelsPerPoint, with the Button, the
/// Shift, Ctrl and Alt keys down, a wheel turn's Notches, the Item under
/// the pointer, null where no named item is, and SceneX and SceneY, the
/// pointer in the items' coordinates under the canvas's view
/// (Set-GuiCanvasView). With -OnKey, or a view whose controls take keys,
/// the canvas takes the focus when pressed, and -OnKey runs for each key
/// the view's controls do not take.
///
/// # Examples
/// Show-GuiWindow { Canvas -Width 200 -Height 120 @{ Kind = 'Rect'; X = 10; Y = 10; Width = 80; Height = 40; Fill = 'Tomato' }, @{ Kind = 'Ellipse'; Name = 'Dot'; X = 120; Y = 30; Width = 40; Height = 40; Fill = 'Navy' } }
/// Canvas -Name Board @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 40; Height = 40 } -OnClick { param($e) Update-GuiWidget Board -Item $e.Value.Item -Property @{ Fill = 'Gold' } }
#[cmdlet(verb = "New", noun = "GuiCanvas", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiCanvas {
    /// The items, each a hashtable of its Kind, its Name and its keys, a
    /// Group's items under Children; none when not given.
    #[param(position = 0)]
    pub item: Vec<PsObject>,
    /// Its width in points, above 0; the room its place leaves it when not
    /// given.
    #[param]
    pub width: Option<f64>,
    /// Its height in points, above 0; the height the window leaves it when
    /// not given.
    #[param]
    pub height: Option<f64>,
    /// A SubEtha ring another process writes, each item changing one of the
    /// canvas's items or several, as Update-GuiWidget -Item does: a JSON
    /// object of the Item it changes, by its name, and the keys it changes,
    /// or an array of them, an Image's picture given by the full path of
    /// its file as Path or its bytes written as base64 as Bytes. A
    /// SubEtha.Ring takes items sent with SendFrame, a SubEtha.LocaleRing
    /// moved to File or ShmFs and a SubEtha.BroadcastRing items of one
    /// slot; it is taken when the window opens.
    #[param]
    pub ring: Option<PsObject>,
    /// Shared memory another process writes, whose rows set the numbers of
    /// the items, row i the i-th item Item names and column j the key
    /// Property names j-th: a hashtable of Source, a SubEtha.Slab or
    /// SubEtha.Vec whose slots are the rows or a SubEtha.Cell containing them
    /// end to end, or for a writer outside SubEtha of Path, a file's path,
    /// Offset and Sequence, the bytes from its start to the values and to
    /// the 64-bit counter its writer makes odd while it writes, Length, the
    /// count of values, and Shape, their rows and columns, which may be
    /// left out; with Type, the values' element type, Item, names and
    /// wildcards, the names in the order given and each wildcard's items in
    /// the scene's order, Property, keys that take one number, and
    /// Notifier, the SubEtha.NotifierSet the writer signals after each
    /// change, or Interval, the seconds between reads, or both. It is taken
    /// when the window opens.
    #[param]
    pub shared_memory: Option<PsObject>,
    /// Runs when a person clicks a named item, with the event as its argument.
    #[param]
    pub on_click: Option<PsScriptBlock>,
    /// Runs when a person clicks a named item twice in quick succession, with the event as its argument.
    #[param]
    pub on_double_click: Option<PsScriptBlock>,
    /// Runs when the pointer comes over a named item, with the event as its argument.
    #[param]
    pub on_pointer_enter: Option<PsScriptBlock>,
    /// Runs when the pointer leaves the named item it was over, with the event as its argument.
    #[param]
    pub on_pointer_leave: Option<PsScriptBlock>,
    /// Runs when a person starts dragging a named item, with the event as its argument; its Value gives where the drag began.
    #[param]
    pub on_drag_start: Option<PsScriptBlock>,
    /// Runs when the pointer moves while a drag goes on, once a frame at most, with the event as its argument; its Value names the item dragged.
    #[param]
    pub on_drag_move: Option<PsScriptBlock>,
    /// Runs when a drag's button is released, with the event as its argument; its Value names the item dragged.
    #[param]
    pub on_drag_end: Option<PsScriptBlock>,
    /// Runs when a pointer button is pressed over it, with the event as its argument.
    #[param]
    pub on_pointer_down: Option<PsScriptBlock>,
    /// Runs when a button pressed over it is released, wherever the pointer is, with the event as its argument.
    #[param]
    pub on_pointer_up: Option<PsScriptBlock>,
    /// Runs when the pointer moves over it, or anywhere while a button pressed over it stays down, once a frame at most, with the event as its argument.
    #[param]
    pub on_pointer_move: Option<PsScriptBlock>,
    /// Runs when the wheel turns over it, with the event as its argument; its Value's Notches are positive when the wheel turns away from the person.
    #[param]
    pub on_wheel: Option<PsScriptBlock>,
    /// Runs when a key is pressed while it has the focus, for each key its view's controls do not take, with the event as its argument; its Value is the key written as a shortcut is.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiCanvas {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = vec![("Item", self.item.clone().into_ps()?)];
        for (label, given) in [("Width", self.width), ("Height", self.height)] {
            if let Some(n) = given {
                props.push((label, n.into_ps()?));
            }
        }
        if let Some(r) = &self.ring {
            props.push(("Ring", r.clone()));
        }
        if let Some(s) = &self.shared_memory {
            props.push(("SharedMemory", s.clone()));
        }
        let handlers = vec![
            ("Click", self.on_click.as_ref()),
            ("DoubleClick", self.on_double_click.as_ref()),
            ("PointerEnter", self.on_pointer_enter.as_ref()),
            ("PointerLeave", self.on_pointer_leave.as_ref()),
            ("DragStart", self.on_drag_start.as_ref()),
            ("DragMove", self.on_drag_move.as_ref()),
            ("DragEnd", self.on_drag_end.as_ref()),
            ("PointerDown", self.on_pointer_down.as_ref()),
            ("PointerUp", self.on_pointer_up.as_ref()),
            ("PointerMove", self.on_pointer_move.as_ref()),
            ("Wheel", self.on_wheel.as_ref()),
            ("Key", self.on_key.as_ref()),
            ("Drop", self.on_drop.as_ref()),
        ];
        let widget = describe(Kind::Canvas, self.name.clone(), props, Vec::new(), handlers, None)?;
        wire_props(ps, Kind::Canvas, &widget.properties, &[], true).and_then(|mut wire| settle(Kind::Canvas, &BTreeMap::new(), None, &mut wire))?;
        ps.write(widget)
    }
}

/// Describes a media player of the clip in -Path, a video or a sound file,
/// played by the operating system's own player in egui windows, with its
/// sound, and by the webview's video and audio elements in HTML windows. The
/// picture shows at the clip's size in pixels as points, fitted to the room
/// its place gives it, or to -Width and -Height; with -AudioOnly the clip's
/// sound plays with no picture. With -AutoPlay it starts as soon as the clip
/// is open. -Loop plays it again from the start at each end, -Volume sets
/// its volume from 0 to 1, -Muted silences it, and -Rate plays it from 0.25
/// to 4 times its speed, the sound's pitch kept. Its controls, which -NoControls
/// takes away, play and pause it, seek, mute, set the volume and go to full
/// screen. -Subtitles shows the cues of a SubRip or WebVTT file over the
/// picture, beside those of the clip's own text track where the player
/// reports one. An https -Path is fetched by the module's loader, by byte
/// ranges, or with -EngineFetch by the player's own engine. Update-GuiWidget
/// changes Loop, Muted, Volume, Rate, PositionInterval, NoControls,
/// Subtitles and EngineFetch while the window is open, and Path to play
/// another clip. A clip the player cannot play raises
/// Error, whose Value's Message names why and the clip's format, and Remedy
/// what plays it instead. In Show-GuiWindow -Content, MediaPlayer stands for
/// it.
///
/// # Examples
/// Show-GuiWindow { MediaPlayer -Path .\clip.mp4 -AutoPlay -OnEnded { param($e) Close-GuiWindow -Result Done } }
#[cmdlet(verb = "New", noun = "GuiMediaPlayer", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiMediaPlayer {
    /// The clip's file, resolved from the current location as PowerShell
    /// resolves a path, with wildcards taken literally; or the clip's https
    /// address.
    #[param(mandatory, position = 0)]
    pub path: String,
    /// Plays the clip's sound alone, with no picture.
    #[param]
    pub audio_only: bool,
    /// Starts playing as soon as the clip is open.
    #[param]
    pub auto_play: bool,
    /// Plays the clip again from the start each time it ends.
    #[param]
    pub r#loop: bool,
    /// Silences the clip's sound.
    #[param]
    pub muted: bool,
    /// Draws no controls: no play and pause, slider, volume or full screen
    /// switch, which a script then works itself.
    #[param]
    pub no_controls: bool,
    /// A SubRip (.srt) or WebVTT (.vtt) file of subtitles, resolved from
    /// the current location, whose cues show over the picture at their
    /// times, beside those of the clip's own tracks the player reports.
    #[param]
    pub subtitles: Option<String>,
    /// Has the player's own engine fetch an https clip, on the system's
    /// network stack, proxy settings and cache, in place of the module's
    /// loader. The clip's server must allow it under CORS, in either
    /// renderer; one that does not fails the clip, saying so.
    #[param]
    pub engine_fetch: bool,
    /// The volume, from 0 for silence to 1 for the clip's own level; 1 when
    /// not given.
    #[param]
    pub volume: Option<f64>,
    /// How many times the clip's own speed it plays at, from 0.25 to 4, the
    /// sound's pitch kept; 1 when not given.
    #[param]
    pub rate: Option<f64>,
    /// The seconds, or a [TimeSpan], between Position events while the clip
    /// plays, above 0; a quarter of a second when not given.
    #[param]
    pub position_interval: Option<PsObject>,
    /// Its width in points, above 0; the room its place leaves it when not
    /// given.
    #[param]
    pub width: Option<f64>,
    /// Its height in points, above 0; the height the window leaves it when
    /// not given.
    #[param]
    pub height: Option<f64>,
    /// Runs once the clip is open, with the event as its argument; its
    /// Value gives the clip's Duration in seconds, its Width and Height in
    /// pixels, and HasVideo and HasAudio.
    #[param]
    pub on_loaded: Option<PsScriptBlock>,
    /// Runs when the clip starts or goes on playing, with the event as its
    /// argument; its Value is the position in seconds.
    #[param]
    pub on_playing: Option<PsScriptBlock>,
    /// Runs when the clip pauses, with the event as its argument; its Value
    /// is the position in seconds.
    #[param]
    pub on_paused: Option<PsScriptBlock>,
    /// Runs when the clip plays to its end, with the event as its argument;
    /// its Value is the position in seconds.
    #[param]
    pub on_ended: Option<PsScriptBlock>,
    /// Runs every -PositionInterval while the clip plays, after each seek
    /// and at the clip's end, with the event as its argument; its Value is
    /// the position in seconds.
    #[param]
    pub on_position: Option<PsScriptBlock>,
    /// Runs when the clip cannot be played, with the event as its argument;
    /// its Value's Message names the clip, the Cause (Aborted, Network,
    /// Decode or FormatNotSupported) and the Format where known, Remedy
    /// what plays it or what to install, and Detail the player's own text.
    #[param]
    pub on_error: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiMediaPlayer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let full = if self.path.starts_with("https://") { self.path.clone() } else { full_path(ps, &self.path)? };
        if !full.starts_with("https://") && !std::path::Path::new(&full).is_file() {
            return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoFile", format!("{}: no file is at {full}", self.path)).terminating());
        }
        let mut props = vec![("Path", full.into_ps()?)];
        for (label, on) in [("AudioOnly", self.audio_only), ("AutoPlay", self.auto_play), ("Loop", self.r#loop), ("Muted", self.muted), ("NoControls", self.no_controls), ("EngineFetch", self.engine_fetch)] {
            if on {
                props.push((label, true.into_ps()?));
            }
        }
        for (label, given) in [("Volume", self.volume), ("Rate", self.rate), ("Width", self.width), ("Height", self.height)] {
            if let Some(n) = given {
                props.push((label, n.into_ps()?));
            }
        }
        if let Some(every) = &self.position_interval {
            props.push(("PositionInterval", every.clone()));
        }
        if let Some(file) = &self.subtitles {
            props.push(("Subtitles", full_path(ps, file)?.into_ps()?));
        }
        let handlers = vec![
            ("Loaded", self.on_loaded.as_ref()),
            ("Playing", self.on_playing.as_ref()),
            ("Paused", self.on_paused.as_ref()),
            ("Ended", self.on_ended.as_ref()),
            ("Position", self.on_position.as_ref()),
            ("Error", self.on_error.as_ref()),
            ("Drop", self.on_drop.as_ref()),
        ];
        let widget = describe(Kind::MediaPlayer, self.name.clone(), props, Vec::new(), handlers, None)?;
        wire_props(ps, Kind::MediaPlayer, &widget.properties, &[], true).and_then(|mut wire| settle(Kind::MediaPlayer, &BTreeMap::new(), None, &mut wire))?;
        ps.write(widget)
    }
}

/// Describes a button. With -Result, pressing it closes the window, and
/// Show-GuiWindow's result has that value; -OnClick runs first. With
/// -Default, Enter presses it wherever the focused widget does not use
/// Enter itself, and it is drawn in the accent color; with -Cancel, Escape
/// presses it. A window has one of each at most. In Show-GuiWindow
/// -Content, Button stands for it.
///
/// # Examples
/// Show-GuiWindow { Text 'Ready?'; ButtonBar { Button 'OK' -Default -Result Ok; Button 'Cancel' -Cancel -Result Cancel } }
/// Show-GuiWindow { Button 'Count' -Name Count -OnClick { param($e) Update-GuiWidget Count @{ Text = 'Counted' } } }
#[cmdlet(verb = "New", noun = "GuiButton", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiButton {
    /// The button's text.
    #[param(mandatory, position = 0)]
    pub text: String,
    /// The window's result when the button is pressed, which closes the
    /// window.
    #[param]
    pub result: Option<PsObject>,
    /// Whether the button can be pressed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// The window's default button, which Enter presses and the accent
    /// color draws.
    #[param]
    pub default: bool,
    /// The window's cancel button, which Escape presses.
    #[param]
    pub cancel: bool,
    /// Runs when the button is pressed, with the event as its argument.
    #[param]
    pub on_click: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the button does not use itself is pressed while it
    /// has the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiButton {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.default && self.cancel {
            let text = format!("the button {} is the default or the cancel button, not both", self.text);
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiButtonRole", text).terminating());
        }
        let mut props = vec![("Text", self.text.clone().into_ps()?)];
        if let Some(e) = self.enabled {
            props.push(("Enabled", e.into_ps()?));
        }
        if self.default {
            props.push(("Default", true.into_ps()?));
        }
        if self.cancel {
            props.push(("Cancel", true.into_ps()?));
        }
        let handlers = vec![("Click", self.on_click.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::Button, self.name.clone(), props, Vec::new(), handlers, self.result.clone())?)
    }
}

/// Describes a list of items to select from, one or, with -Multiple, any
/// number. State gives the item selected, or with -Multiple the items, as
/// text. In Show-GuiWindow -Content, ListBox stands for it.
///
/// # Examples
/// Show-GuiWindow { ListBox 'staging', 'production' -Name Target; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { ListBox 'a', 'b', 'c' -Multiple -Selected 'b' -Name Picked -OnSelect { param($e) $e.Value } }
#[cmdlet(verb = "New", noun = "GuiListBox", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiListBox {
    /// The items listed, as text.
    #[param(mandatory, position = 0)]
    pub item: Vec<String>,
    /// Lets any number of items be selected rather than one.
    #[param]
    pub multiple: bool,
    /// The item, or with -Multiple the items, selected when the window
    /// opens, named by their text without regard to case.
    #[param]
    pub selected: Option<Vec<String>>,
    /// Runs when the selection changes, with the event as its argument.
    #[param]
    pub on_select: Option<PsScriptBlock>,
    /// Runs when an item is double-clicked or chosen with Enter, with the
    /// event as its argument.
    #[param]
    pub on_activate: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the list does not use itself is pressed while it has
    /// the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiListBox {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = vec![("Item", self.item.clone().into_ps()?), ("Multiple", self.multiple.into_ps()?)];
        if let Some(s) = &self.selected {
            if !self.multiple && s.len() > 1 {
                let text = format!("{} items were given to select in a list that selects one; -Multiple selects several", s.len());
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiTooManySelected", text).terminating());
            }
            props.push(("Selected", s.clone().into_ps()?));
        }
        let handlers = vec![("Select", self.on_select.as_ref()), ("Activate", self.on_activate.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::ListBox, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// How many of a table's rows can be selected.
#[psenum(name = "PwrsGui.Selection")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiSelection {
    /// None.
    None,
    /// One.
    #[default]
    Single,
    /// Any number.
    Multiple,
}

impl GuiSelection {
    fn name(self) -> &'static str {
        match self {
            GuiSelection::None => "None",
            GuiSelection::Single => "Single",
            GuiSelection::Multiple => "Multiple",
        }
    }
}

/// Describes a table of objects, one row each, that sorts by any column
/// and selects rows, and with -Filter shows only the rows containing the text
/// a person types: the grid view's widget. Its columns are the properties -Property names, or those
/// Out-GuiGridView shows for the first object. State gives the object
/// selected, or with -Selection Multiple the objects, as they were given.
/// In Show-GuiWindow -Content, Table stands for it.
///
/// # Examples
/// Show-GuiWindow { Get-Service | Table -Name Services; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { Table (Get-Process) -Property Name, Id -Selection Multiple -Name Picked }
#[cmdlet(verb = "New", noun = "GuiTable", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiTable {
    /// The objects, one row each; piped objects are added in the order
    /// piped.
    #[param(position = 0, value_from_pipeline)]
    pub input_object: Vec<PsObject>,
    /// The properties shown as columns, in order; without it, the columns
    /// Out-GuiGridView shows for the first object.
    #[param]
    pub property: Option<Vec<String>>,
    /// How many rows can be selected: None, Single or Multiple; Single when
    /// not given.
    #[param]
    pub selection: Option<GuiSelection>,
    /// Puts a field above the rows that shows only those containing the text
    /// typed in it, without regard to case, beside how many rows it shows of
    /// how many.
    #[param]
    pub filter: bool,
    /// Runs when the selection changes, with the event as its argument.
    #[param]
    pub on_select: Option<PsScriptBlock>,
    /// Runs when a row is double-clicked, with the event as its argument.
    #[param]
    pub on_activate: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the table does not use itself is pressed while it
    /// has the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
    gathered: Vec<PsObject>,
}

impl Cmdlet for NewGuiTable {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        self.gathered.append(&mut self.input_object);
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let selection = match self.selection {
            Some(s) => s,
            None => GuiSelection::Single,
        };
        let mut props = vec![("Item", std::mem::take(&mut self.gathered).into_ps()?), ("Selection", selection.name().into_ps()?)];
        if let Some(p) = &self.property {
            props.push(("Property", p.clone().into_ps()?));
        }
        if self.filter {
            props.push(("Filter", true.into_ps()?));
        }
        let handlers = vec![("Select", self.on_select.as_ref()), ("Activate", self.on_activate.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::Table, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a tree of nodes that open to show the nodes inside them, one
/// selectable or, with -Multiple, any number with Ctrl and Shift. -Node
/// gives the top nodes, and piped nodes follow them: a string is a leaf; a
/// hashtable gives its Text and its Children up front, in the same forms;
/// any other object takes its text from -Text and, when -HasChildren says
/// it has children, opens through -OnExpand, whose output becomes its
/// children. State gives the object the selected node was made from, or
/// with -Multiple the objects, as they were given. Add-GuiItem -Parent and
/// Remove-GuiItem change the tree while its window is open. In
/// Show-GuiWindow -Content, TreeView stands for it.
///
/// # Examples
/// Show-GuiWindow { TreeView -Name Food -Node @{ Text = 'Fruit'; Children = 'Apple', 'Pear' }, @{ Text = 'Veg'; Children = @('Leek') }; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { TreeView -Name Folders -Node (Get-Item $HOME) -Text Name -HasChildren { $_.PSIsContainer } -OnExpand { param($e) Get-ChildItem $e.Value.FullName -Directory } }
#[cmdlet(verb = "New", noun = "GuiTreeView", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiTreeView {
    /// The top nodes, strings, hashtables or other objects; piped nodes
    /// are added in the order piped.
    #[param(position = 0, value_from_pipeline)]
    pub node: Vec<PsObject>,
    /// How an object node gives its text: the name of one of its
    /// properties, or a script block run with the node as $_; its own text
    /// when not given. A string gives itself, and a hashtable its Text.
    #[param]
    pub text: Option<PsObject>,
    /// A script block run with an object node as $_ that says whether the
    /// node has children, which -OnExpand gives when a person opens it.
    #[param]
    pub has_children: Option<PsScriptBlock>,
    /// Lets any number of nodes be selected, with Ctrl and Shift, rather
    /// than one.
    #[param]
    pub multiple: bool,
    /// Runs when a person opens a node -HasChildren says has children, with
    /// the event as its argument, whose Value is the node's object; what it
    /// writes becomes the node's children, in the forms -Node takes. When
    /// it fails, the node closes again, and opening it runs this again.
    #[param]
    pub on_expand: Option<PsScriptBlock>,
    /// Runs when the selection changes, with the event as its argument.
    #[param]
    pub on_select: Option<PsScriptBlock>,
    /// Runs when a node is double-clicked or chosen with Enter, with the
    /// event as its argument.
    #[param]
    pub on_activate: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the tree does not use itself is pressed while it has
    /// the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
    gathered: Vec<PsObject>,
}

impl Cmdlet for NewGuiTreeView {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        self.gathered.append(&mut self.node);
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let refuse = |text: &str| PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating();
        match (&self.has_children, &self.on_expand) {
            (Some(_), None) => return Err(refuse("-HasChildren says which nodes have children, which -OnExpand gives; give -OnExpand too")),
            (None, Some(_)) => return Err(refuse("-OnExpand gives the children of the nodes -HasChildren says have them; give -HasChildren too")),
            (Some(_), Some(_)) | (None, None) => {}
        }
        let mut props = vec![("Node", std::mem::take(&mut self.gathered).into_ps()?), ("Multiple", self.multiple.into_ps()?)];
        if let Some(t) = &self.text {
            let type_name = t.type_name()?;
            if type_name != "System.String" && type_name != "System.Management.Automation.ScriptBlock" {
                let text = format!("-Text is a {type_name}; it takes the name of a property, or a script block");
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
            }
            props.push(("Text", t.clone()));
        }
        if let Some(h) = &self.has_children {
            props.push(("HasChildren", h.0.clone()));
        }
        let handlers = vec![
            ("Select", self.on_select.as_ref()),
            ("Activate", self.on_activate.as_ref()),
            ("Expand", self.on_expand.as_ref()),
            ("Drop", self.on_drop.as_ref()),
            ("Key", self.on_key.as_ref()),
        ];
        ps.write(describe(Kind::TreeView, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a check box. State gives whether it is checked. In
/// Show-GuiWindow -Content, CheckBox stands for it.
///
/// # Examples
/// Show-GuiWindow { CheckBox 'Dry run' -Name DryRun -Checked; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { CheckBox 'Verbose' -OnChange { param($e) Update-GuiWidget Shown @{ Text = "Verbose: $($e.Value)" } }; Text 'Verbose: False' -Name Shown }
#[cmdlet(verb = "New", noun = "GuiCheckBox", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiCheckBox {
    /// The text beside the box.
    #[param(mandatory, position = 0, allow_empty_string)]
    pub text: String,
    /// Checks the box when the window opens.
    #[param]
    pub checked: bool,
    /// Whether the box can be changed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs when the box is checked or cleared, with the event as its
    /// argument; its Value is whether the box is checked.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the box does not use itself is pressed while it has
    /// the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiCheckBox {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = vec![("Text", self.text.clone().into_ps()?), ("Checked", self.checked.into_ps()?)];
        if let Some(e) = self.enabled {
            props.push(("Enabled", e.into_ps()?));
        }
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::CheckBox, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a text field, one line unless -Multiline. State gives its
/// text; with -Password the characters are hidden, and State and the
/// handlers' events give the text as a SecureString, as Get-Credential
/// does. In Show-GuiWindow -Content, TextBox stands for it.
///
/// # Examples
/// Show-GuiWindow { TextBox -Name Server -Placeholder 'host name'; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { TextBox -Name Secret -Password; ButtonBar { Button 'OK' -Default -Result Ok } }
#[cmdlet(verb = "New", noun = "GuiTextBox", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiTextBox {
    /// The text when the window opens; empty when not given.
    #[param(position = 0)]
    pub text: String,
    /// Takes several lines.
    #[param]
    pub multiline: bool,
    /// Hides the characters typed; State and the handlers' events give the
    /// text as a SecureString. A password field is one line, and stays one
    /// for as long as the window is open. Once it has the focus and is
    /// typed into, it shows an eye at its right end: the password shows
    /// while the eye is held, or Alt+F8 is. Once the focus leaves the field
    /// the eye is not offered again until the field is cleared.
    #[param]
    pub password: bool,
    /// Text shown while the field is empty.
    #[param]
    pub placeholder: Option<String>,
    /// Whether the text can be changed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs after each edit, with the event as its argument; its Value is
    /// the text.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// In a field of one line, runs when Enter is pressed, with the event
    /// as its argument; its Value is the text.
    #[param]
    pub on_submit: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the field does not edit with is pressed while it has
    /// the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiTextBox {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.password && self.multiline {
            let text = "a password field is one line; give -Password or -Multiline, not both";
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
        }
        let mut props = vec![("Text", self.text.clone().into_ps()?), ("Multiline", self.multiline.into_ps()?), ("Password", self.password.into_ps()?)];
        if let Some(p) = &self.placeholder {
            props.push(("Placeholder", p.clone().into_ps()?));
        }
        if let Some(e) = self.enabled {
            props.push(("Enabled", e.into_ps()?));
        }
        if self.multiline && self.on_submit.is_some() {
            let text = "-OnSubmit runs when Enter is pressed in a field of one line, and Enter starts a new line in one with -Multiline";
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
        }
        let handlers = vec![("Change", self.on_change.as_ref()), ("Submit", self.on_submit.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::TextBox, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// The properties of a number field's or slider's numbers, those given.
fn number_props(value: f64, minimum: Option<f64>, maximum: Option<f64>, step: Option<f64>, enabled: Option<bool>) -> PsResult<Vec<(&'static str, PsObject)>> {
    let mut props = vec![("Value", value.into_ps()?)];
    for (name, n) in [("Minimum", minimum), ("Maximum", maximum), ("Step", step)] {
        if let Some(n) = n {
            props.push((name, n.into_ps()?));
        }
    }
    if let Some(e) = enabled {
        props.push(("Enabled", e.into_ps()?));
    }
    Ok(props)
}

/// Describes a number field whose number stays between -Minimum and
/// -Maximum when they are given, which its arrows change by -Step. State gives its number as a
/// double. In Show-GuiWindow -Content, NumberBox stands for it.
///
/// # Examples
/// Show-GuiWindow { NumberBox 3 -Minimum 1 -Maximum 10 -Name Copies; ButtonBar { Button 'OK' -Default -Result Ok } }
#[cmdlet(verb = "New", noun = "GuiNumberBox", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiNumberBox {
    /// The number when the window opens; when not given, 0, or the nearer
    /// of -Minimum and -Maximum when 0 is outside them.
    #[param(position = 0)]
    pub value: Option<f64>,
    /// The smallest number it takes; no limit when not given.
    #[param]
    pub minimum: Option<f64>,
    /// The largest number it takes; no limit when not given.
    #[param]
    pub maximum: Option<f64>,
    /// How much its arrows change it, above 0; 1 when not given.
    #[param]
    pub step: Option<f64>,
    /// Whether the number can be changed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs when the number changes, with the event as its argument; its
    /// Value is the number.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the field does not edit with is pressed while it has
    /// the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiNumberBox {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let value = match self.value {
            Some(v) => v,
            None => nearest(0.0, self.minimum, self.maximum),
        };
        check_numbers(Kind::NumberBox, value, self.minimum, self.maximum, self.step)?;
        let props = number_props(value, self.minimum, self.maximum, self.step, self.enabled)?;
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::NumberBox, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a slider from -Minimum to -Maximum, 0 to 100 when not given,
/// moving in steps of -Step. State gives its number as a double. In
/// Show-GuiWindow -Content, Slider stands for it.
///
/// # Examples
/// Show-GuiWindow { Slider 50 -Name Volume; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { Slider -Maximum 1 -Step 0.05 -OnChange { param($e) Update-GuiWidget Shown @{ Text = "$($e.Value)" } }; Text '0' -Name Shown }
#[cmdlet(verb = "New", noun = "GuiSlider", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiSlider {
    /// The number when the window opens; -Minimum when not given.
    #[param(position = 0)]
    pub value: Option<f64>,
    /// The number at the slider's start; 0 when not given.
    #[param]
    pub minimum: Option<f64>,
    /// The number at the slider's end; 100 when not given.
    #[param]
    pub maximum: Option<f64>,
    /// The steps it moves in, above 0; 1 when not given.
    #[param]
    pub step: Option<f64>,
    /// Whether the slider can be moved; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs as the slider moves, with the event as its argument; its Value
    /// is the number.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the slider does not use itself is pressed while it
    /// has the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiSlider {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let minimum = self.minimum.unwrap_or(0.0);
        let maximum = self.maximum.unwrap_or(100.0);
        let value = match self.value {
            Some(v) => v,
            None => minimum,
        };
        check_numbers(Kind::Slider, value, Some(minimum), Some(maximum), self.step)?;
        let props = number_props(value, Some(minimum), Some(maximum), self.step, self.enabled)?;
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::Slider, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a drop-down of choices, of which one can be chosen; none is
/// until a person picks one, unless -Selected names it. State gives the
/// choice as text, null for none. With -Editable it is a text field that
/// offers the choices as a person types, and State gives the text. In
/// Show-GuiWindow -Content, ComboBox stands for it.
///
/// # Examples
/// Show-GuiWindow { ComboBox 'staging', 'production' -Name Target -Selected staging; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { ComboBox 'web01', 'web02' -Editable -Name Server; ButtonBar { Button 'OK' -Default -Result Ok } }
#[cmdlet(verb = "New", noun = "GuiComboBox", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiComboBox {
    /// The choices, as text.
    #[param(mandatory, position = 0)]
    pub choice: Vec<String>,
    /// The choice chosen when the window opens, named by its text without
    /// regard to case; with -Editable, the text it opens with, any text.
    #[param]
    pub selected: Option<String>,
    /// Lets a person type any text, offering the choices as they type; set
    /// when the window opens.
    #[param]
    pub editable: bool,
    /// Whether the choice can be changed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs when a person picks a choice, with the event as its argument;
    /// its Value is the choice.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the drop-down does not use itself is pressed while
    /// it has the focus, with the event as its argument; its Value is the
    /// key written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiComboBox {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let props = if self.editable {
            let text = match &self.selected {
                Some(s) => s.clone(),
                None => String::new(),
            };
            let mut props = vec![("Choice", self.choice.clone().into_ps()?), ("Editable", true.into_ps()?), ("Text", text.into_ps()?)];
            if let Some(e) = self.enabled {
                props.push(("Enabled", e.into_ps()?));
            }
            props
        } else {
            check_selected(self.selected.as_ref(), &self.choice)?;
            choice_props(&self.choice, self.selected.as_ref(), self.enabled)?
        };
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::ComboBox, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// The properties of a drop-down's or option group's choices, those given.
fn choice_props(choice: &[String], selected: Option<&String>, enabled: Option<bool>) -> PsResult<Vec<(&'static str, PsObject)>> {
    let mut props = vec![("Choice", choice.to_vec().into_ps()?)];
    if let Some(s) = selected {
        props.push(("Selected", s.clone().into_ps()?));
    }
    if let Some(e) = enabled {
        props.push(("Enabled", e.into_ps()?));
    }
    Ok(props)
}

/// Describes option buttons, one per choice, of which one can be chosen;
/// none is until a person picks one, unless -Selected names it. State gives
/// the choice as text, null for none. In Show-GuiWindow -Content,
/// RadioGroup stands for it.
///
/// # Examples
/// Show-GuiWindow { RadioGroup 'Small', 'Medium', 'Large' -Name Size -Selected Medium; ButtonBar { Button 'OK' -Default -Result Ok } }
#[cmdlet(verb = "New", noun = "GuiRadioGroup", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiRadioGroup {
    /// The choices, as text, one button each.
    #[param(mandatory, position = 0)]
    pub choice: Vec<String>,
    /// The choice chosen when the window opens, named by its text without
    /// regard to case.
    #[param]
    pub selected: Option<String>,
    /// Whether the choice can be changed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs when a person picks a choice, with the event as its argument;
    /// its Value is the choice.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the option buttons do not use themselves is pressed
    /// while one has the focus, with the event as its argument; its Value
    /// is the key written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiRadioGroup {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        check_selected(self.selected.as_ref(), &self.choice)?;
        let props = choice_props(&self.choice, self.selected.as_ref(), self.enabled)?;
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::RadioGroup, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a color chooser. State gives the color written #rrggbb in
/// lower case. In Show-GuiWindow -Content, ColorPicker stands for it.
///
/// # Examples
/// Show-GuiWindow { ColorPicker '#336699' -Name Accent; ButtonBar { Button 'OK' -Default -Result Ok } }
#[cmdlet(verb = "New", noun = "GuiColorPicker", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiColorPicker {
    /// The color when the window opens, written #rrggbb in either case;
    /// #000000 when not given.
    #[param(position = 0)]
    pub color: Option<String>,
    /// Whether the color can be changed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs when a person chooses a color, with the event as its argument;
    /// its Value is the color.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the chooser does not use itself is pressed while it
    /// has the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiColorPicker {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let given = match &self.color {
            Some(c) => c.clone(),
            None => String::from("#000000"),
        };
        let Some(written) = color(&given) else {
            let text = format!("a ColorPicker's -Color is {given}; it must be a color written #rrggbb");
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", text).terminating());
        };
        let mut props = vec![("Color", written.into_ps()?)];
        if let Some(e) = self.enabled {
            props.push(("Enabled", e.into_ps()?));
        }
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        ps.write(describe(Kind::ColorPicker, self.name.clone(), props, Vec::new(), handlers, None)?)
    }
}

/// Describes a date and time chooser, to the minute, whose value stays
/// between -Minimum and -Maximum when they are given. State gives the date and time chosen
/// as a [datetime], null until one is. In Show-GuiWindow -Content,
/// DatePicker stands for it.
///
/// # Examples
/// Show-GuiWindow { DatePicker (Get-Date) -Name When; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { DatePicker -Minimum (Get-Date) -Maximum (Get-Date).AddDays(30) -Name Due -OnChange { param($e) $e.Value } }
#[cmdlet(verb = "New", noun = "GuiDatePicker", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiDatePicker {
    /// The date and time when the window opens, to the minute; none when
    /// not given.
    #[param(position = 0)]
    pub value: Option<PsDateTime>,
    /// The earliest date and time it takes; no limit when not given.
    #[param]
    pub minimum: Option<PsDateTime>,
    /// The latest date and time it takes; no limit when not given.
    #[param]
    pub maximum: Option<PsDateTime>,
    /// Whether the date can be changed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs when a person chooses a date or time, with the event as its
    /// argument; its Value is the date and time, a [datetime].
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the chooser does not use itself is pressed while it
    /// has the focus, with the event as its argument; its Value is the key
    /// written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiDatePicker {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = Vec::new();
        for (key, when) in [("Value", self.value), ("Minimum", self.minimum), ("Maximum", self.maximum)] {
            if let Some(w) = when {
                props.push((key, w.into_ps()?));
            }
        }
        if let Some(e) = self.enabled {
            props.push(("Enabled", e.into_ps()?));
        }
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        let widget = describe(Kind::DatePicker, self.name.clone(), props, Vec::new(), handlers, None)?;
        wire_props(ps, Kind::DatePicker, &widget.properties, &[], true).and_then(|mut wire| settle(Kind::DatePicker, &BTreeMap::new(), None, &mut wire))?;
        ps.write(widget)
    }
}

/// Describes a file chooser: the path chosen, beside a button that opens
/// the system's dialog for a file to open, with -Save a file to save, or
/// with -Folder a folder. -Filter limits the files the dialog shows, as
/// strings written 'Text files|*.txt;*.md', or as an ordered dictionary
/// from each name to its extensions, [ordered]@{ 'Text files' = 'txt',
/// 'md' }; a plain hashtable is refused, since it has no order. State
/// gives the path chosen, null until one is, or with -Multiple the paths.
/// In Show-GuiWindow -Content, FilePicker stands for it.
///
/// # Examples
/// Show-GuiWindow { FilePicker -Name Log -Filter 'Logs|*.log', 'All files|*'; ButtonBar { Button 'OK' -Default -Result Ok } }
/// Show-GuiWindow { FilePicker -Folder -Name Target -Title 'Where to copy' -OnChange { param($e) $e.Value } }
#[cmdlet(verb = "New", noun = "GuiFilePicker", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiFilePicker {
    /// The path chosen when the window opens, or with -Multiple the paths,
    /// resolved from the current location; none when not given.
    #[param(position = 0)]
    pub path: Vec<String>,
    /// Chooses a file to save rather than one to open; set when the window
    /// opens.
    #[param]
    pub save: bool,
    /// Chooses a folder rather than a file; set when the window opens.
    #[param]
    pub folder: bool,
    /// Lets any number of files or folders be chosen rather than one; set
    /// when the window opens.
    #[param]
    pub multiple: bool,
    /// The kinds of files the dialog shows, the first shown first: strings
    /// written 'Name|*.txt;*.md', or an ordered dictionary from each name
    /// to its extensions; every file when not given.
    #[param]
    pub filter: Option<PsObject>,
    /// The dialog's title; the system's own when not given.
    #[param]
    pub title: Option<String>,
    /// Whether the path can be changed; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs when a person chooses a path, with the event as its argument;
    /// its Value is the path, or with -Multiple the paths.
    #[param]
    pub on_change: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// Runs when a key the chooser does not use itself is pressed while its
    /// button has the focus, with the event as its argument; its Value is
    /// the key written as a shortcut, as Ctrl+K.
    #[param]
    pub on_key: Option<PsScriptBlock>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiFilePicker {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = Vec::new();
        if !self.path.is_empty() {
            let resolved = self.path.iter().map(|p| full_path(ps, p)).collect::<PsResult<Vec<String>>>()?;
            props.push(("Path", resolved.into_ps()?));
        }
        for (key, on) in [("Save", self.save), ("Folder", self.folder), ("Multiple", self.multiple)] {
            if on {
                props.push((key, true.into_ps()?));
            }
        }
        if let Some(f) = &self.filter {
            props.push(("Filter", f.clone()));
        }
        if let Some(t) = &self.title {
            props.push(("Title", t.clone().into_ps()?));
        }
        if let Some(e) = self.enabled {
            props.push(("Enabled", e.into_ps()?));
        }
        let handlers = vec![("Change", self.on_change.as_ref()), ("Drop", self.on_drop.as_ref()), ("Key", self.on_key.as_ref())];
        let widget = describe(Kind::FilePicker, self.name.clone(), props, Vec::new(), handlers, None)?;
        wire_props(ps, Kind::FilePicker, &widget.properties, &[], true).and_then(|mut wire| settle(Kind::FilePicker, &BTreeMap::new(), None, &mut wire))?;
        ps.write(widget)
    }
}

/// Describes a bar of menus along the top of the window: the Menu widgets
/// its block writes. It stands among the window's own widgets, wherever in
/// the block. In Show-GuiWindow -Content, MenuBar stands for it.
///
/// # Examples
/// Show-GuiWindow { MenuBar { Menu 'File' { MenuItem 'Exit' -Result Exit } }; Text 'Body' }
#[cmdlet(verb = "New", noun = "GuiMenuBar", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiMenuBar {
    /// The block that writes the menus, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// The name it goes by in the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiMenuBar {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::MenuBar, self.content.as_ref())?;
        ps.write(describe(Kind::MenuBar, self.name.clone(), Vec::new(), children, Vec::new(), None)?)
    }
}

/// Describes a menu, in a MenuBar, a ContextMenu or another Menu: its
/// title, and the MenuItem, Menu and Separator widgets its block writes.
/// In Show-GuiWindow -Content, Menu stands for it.
///
/// # Examples
/// Show-GuiWindow { MenuBar { Menu 'File' { MenuItem 'Save' -Shortcut Ctrl+S -OnClick { Save-Report }; Separator; MenuItem 'Exit' -Result Exit } } }
#[cmdlet(verb = "New", noun = "GuiMenu", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiMenu {
    /// The menu's title.
    #[param(mandatory, position = 0)]
    pub title: String,
    /// The block that writes the items, in order.
    #[param(position = 1)]
    pub content: Option<PsScriptBlock>,
    /// The name it goes by in the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiMenu {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::Menu, self.content.as_ref())?;
        ps.write(describe(Kind::Menu, self.name.clone(), vec![("Title", self.title.clone().into_ps()?)], children, Vec::new(), None)?)
    }
}

/// Describes a menu's item. -Shortcut, as Ctrl+S, is shown beside the text
/// and pressed in the window acts as a click; -OnClick runs on a click,
/// and -Result closes the window with that result, as a button's does. In
/// Show-GuiWindow -Content, MenuItem stands for it.
///
/// # Examples
/// Show-GuiWindow { MenuBar { Menu 'File' { MenuItem 'Open' -Shortcut Ctrl+O -OnClick { Open-Report } } } }
#[cmdlet(verb = "New", noun = "GuiMenuItem", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiMenuItem {
    /// The item's text.
    #[param(mandatory, position = 0)]
    pub text: String,
    /// The keys that act as a click, as Ctrl+Shift+S: Ctrl, Shift and Alt
    /// joined by + to a letter, a digit, F1 to F24, Enter, Escape, Tab,
    /// Space, Backspace, Delete, Insert, Home, End, PageUp, PageDown, an
    /// arrow key, or Plus, Minus or Equals, the keys typing +, - and =.
    #[param]
    pub shortcut: Option<String>,
    /// Whether the item can be chosen; true when not given.
    #[param]
    pub enabled: Option<bool>,
    /// Runs when the item is chosen, with the event as its argument.
    #[param]
    pub on_click: Option<PsScriptBlock>,
    /// The window's result when the item is chosen, which closes the
    /// window.
    #[param]
    pub result: Option<PsObject>,
    /// The name it goes by in State, the handlers' events and the update
    /// commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiMenuItem {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = vec![("Text", self.text.clone().into_ps()?)];
        if let Some(s) = &self.shortcut {
            let written = Shortcut::parse(s).map_err(|e| PsError::new(ErrorCategory::InvalidArgument, "GuiBadShortcut", e).terminating())?.written();
            props.push(("Shortcut", written.into_ps()?));
        }
        if let Some(e) = self.enabled {
            props.push(("Enabled", e.into_ps()?));
        }
        let handlers = vec![("Click", self.on_click.as_ref())];
        ps.write(describe(Kind::MenuItem, self.name.clone(), props, Vec::new(), handlers, self.result.clone())?)
    }
}

/// Describes a menu that opens where a person right-clicks the widget -For
/// names: the MenuItem, Menu and Separator widgets its block writes. It may
/// stand anywhere in the block. In Show-GuiWindow -Content, ContextMenu
/// stands for it.
///
/// # Examples
/// Show-GuiWindow { ListBox 'web01', 'web02' -Name Servers; ContextMenu -For Servers { MenuItem 'Copy' -OnClick { param($e) Set-Clipboard $e.State.Servers } } }
#[cmdlet(verb = "New", noun = "GuiContextMenu", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiContextMenu {
    /// The name of the widget the menu belongs to.
    #[param(mandatory)]
    pub for_: String,
    /// The block that writes the items, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// The name it goes by in the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiContextMenu {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::ContextMenu, self.content.as_ref())?;
        ps.write(describe(Kind::ContextMenu, self.name.clone(), vec![("For", self.for_.clone().into_ps()?)], children, Vec::new(), None)?)
    }
}

/// Describes a bar along the bottom of the window: the widgets its block
/// writes, side by side. It stands among the window's own widgets,
/// wherever in the block. In Show-GuiWindow -Content, StatusBar stands for
/// it.
///
/// # Examples
/// Show-GuiWindow { Text 'Body'; StatusBar { Text 'Ready' -Name Status } }
#[cmdlet(verb = "New", noun = "GuiStatusBar", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiStatusBar {
    /// The block that writes the widgets inside, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// Runs when files or text are dropped on it, with the event as its
    /// argument; its Value is the paths dropped, or the text.
    #[param]
    pub on_drop: Option<PsScriptBlock>,
    /// The name it goes by in the handlers' events and the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiStatusBar {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::StatusBar, self.content.as_ref())?;
        ps.write(describe(Kind::StatusBar, self.name.clone(), Vec::new(), children, vec![("Drop", self.on_drop.as_ref())], None)?)
    }
}

/// Describes the window's buttons along its bottom: the Buttons its block
/// writes, at the right, in the order the system's dialogs put them by each
/// button's role: on Windows the -Default button first and the -Cancel
/// button last, as OK, Cancel; on macOS, Linux and the BSDs the -Cancel
/// button then the -Default button last, as Cancel, OK; buttons without a
/// role in the order written. Each is at least 80 by 24 points, and the
/// window's widgets sit inside a dialog's 12-point margin; on Windows the
/// bar is a strip a shade apart from the window, as Windows' own dialogs
/// draw it. It stands among the window's own widgets, wherever in the
/// block, and a window has one at most. In Show-GuiWindow -Content,
/// ButtonBar stands for it.
///
/// # Examples
/// Show-GuiWindow { Text 'Delete the old logs?' -Style Instruction; ButtonBar { Button 'Delete' -Default -Result Delete; Button 'Cancel' -Cancel -Result Cancel } }
#[cmdlet(verb = "New", noun = "GuiButtonBar", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiButtonBar {
    /// The block that writes the buttons, in order.
    #[param(position = 0)]
    pub content: Option<PsScriptBlock>,
    /// The name it goes by in the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiButtonBar {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let children = children_of(ps, Kind::ButtonBar, self.content.as_ref())?;
        ps.write(describe(Kind::ButtonBar, self.name.clone(), Vec::new(), children, Vec::new(), None)?)
    }
}

/// Describes a horizontal rule. In Show-GuiWindow -Content, Separator
/// stands for it.
///
/// # Examples
/// Show-GuiWindow { Text 'Above'; Separator; Text 'Below' }
#[cmdlet(verb = "New", noun = "GuiSeparator", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiSeparator {
    /// The name it goes by in the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiSeparator {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        ps.write(describe(Kind::Separator, self.name.clone(), Vec::new(), Vec::new(), Vec::new(), None)?)
    }
}

/// Describes empty space; with -Fill, the room its Row or Column leaves, so
/// the widgets after it go to the far end. In Show-GuiWindow -Content,
/// Spacer stands for it.
///
/// # Examples
/// Show-GuiWindow { Text 'Above'; Spacer 24; Text 'Below' }
/// Show-GuiWindow { Row { Text 'Left'; Spacer -Fill; Text 'Right' } }
#[cmdlet(verb = "New", noun = "GuiSpacer", output = ["PwrsGui.Widget"])]
#[derive(Default)]
pub struct NewGuiSpacer {
    /// The space, in points, zero or more; 8 when not given. With -Fill,
    /// the least it takes.
    #[param(position = 0)]
    pub size: Option<f64>,
    /// Take the room its Row or Column leaves.
    #[param]
    pub fill: bool,
    /// The name it goes by in the update commands.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for NewGuiSpacer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut props = Vec::new();
        if let Some(s) = self.size {
            settle(Kind::Spacer, &BTreeMap::new(), None, &mut BTreeMap::from([(String::from("size"), Value::Float(s))]))?;
            props.push(("Size", s.into_ps()?));
        }
        if self.fill {
            props.push(("Fill", true.into_ps()?));
        }
        ps.write(describe(Kind::Spacer, self.name.clone(), props, Vec::new(), Vec::new(), None)?)
    }
}

/// How a tree's object nodes give their text and say they have children,
/// stored while its window is open for the nodes added later: -Text, a
/// property name or a script block, and -HasChildren, a script block, each
/// null when not given.
#[derive(Clone)]
pub struct TreeSpec {
    pub text: PsObject,
    pub has_children: PsObject,
}

/// One item of a tree as the module stores it while its window is open: the
/// tree, the item it is under (none for a top item), its text, the object
/// it was made from, and that object's key.
pub struct TreeItem {
    pub tree: NodeId,
    pub parent: Option<NodeId>,
    pub text: String,
    pub object: PsObject,
    pub key: ItemKey,
}

/// What narrows the search for the tree items made from an object to the
/// few that may be, which [`MADE_FROM`] then confirms. Every object
/// [`MADE_FROM`] would match has the key of the one it matches: a string
/// its text in the invariant culture's capitals, which is how a comparison
/// without regard to case compares it; any other object the engine's
/// identity hash of the instance compared, a custom object's own wrapper or
/// another object's base; a number or other value only its kind, since
/// values match across types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKey {
    Text(u64),
    Instance(i64),
    Value,
}

/// A window's tree items, by node and by tree and key, so that finding the
/// items made from an object compares it with those of its key alone.
#[derive(Clone, Default)]
pub struct TreeItems {
    by_node: BTreeMap<NodeId, Arc<TreeItem>>,
    by_key: BTreeSet<(NodeId, ItemKey, NodeId)>,
}

impl TreeItems {
    pub fn get(&self, id: &NodeId) -> Option<&Arc<TreeItem>> {
        self.by_node.get(id)
    }

    pub fn insert(&mut self, id: NodeId, item: Arc<TreeItem>) {
        let key = (item.tree, item.key, id);
        if let Some(old) = self.by_node.insert(id, item) {
            self.by_key.remove(&(old.tree, old.key, id));
        }
        self.by_key.insert(key);
    }

    pub fn remove(&mut self, id: NodeId) {
        if let Some(old) = self.by_node.remove(&id) {
            self.by_key.remove(&(old.tree, old.key, id));
        }
    }

    /// Takes every item of `other` along with those it has.
    pub fn extend(&mut self, other: &TreeItems) {
        for (id, item) in &other.by_node {
            self.insert(*id, Arc::clone(item));
        }
    }

    /// The items of the tree `tree` whose objects have the key `key`.
    pub fn keyed(&self, tree: NodeId, key: ItemKey) -> impl Iterator<Item = NodeId> + '_ {
        self.by_key.range((tree, key, NodeId::MIN)..=(tree, key, NodeId::MAX)).map(|(_, _, id)| *id)
    }

    /// The items under any of `roots`, the roots included, found by their
    /// parents.
    pub fn under(&self, roots: &BTreeSet<NodeId>) -> BTreeSet<NodeId> {
        let mut out = BTreeSet::new();
        for id in self.by_node.keys() {
            let mut at = Some(*id);
            while let Some(a) = at {
                if roots.contains(&a) {
                    out.insert(*id);
                    break;
                }
                at = self.by_node.get(&a).and_then(|i| i.parent);
            }
        }
        out
    }
}

/// A window's tree built from its widgets, with what the module stores of
/// each node: its handlers, a button's result, a table's cells, which the
/// renderer is sent, and its rows with the objects they were made from, a
/// tree's way of reading its nodes and each of its items, a canvas's items
/// stored for later changes, and the number the window's next node takes.
pub struct Built {
    pub tree: Node,
    pub handlers: HashMap<(NodeId, EventKind), PsScriptBlock>,
    pub results: BTreeMap<NodeId, PsObject>,
    pub rows: BTreeMap<NodeId, Vec<Row>>,
    pub tables: BTreeMap<NodeId, TableRows>,
    pub trees: BTreeMap<NodeId, TreeSpec>,
    pub items: TreeItems,
    pub canvases: BTreeMap<NodeId, Board>,
    pub next: NodeId,
}

/// The tree of `widgets`, numbered from 1 in tree order: the widget itself
/// when there is one, else a Column containing them.
pub fn build(ps: &Pipeline<'_>, widgets: &[PsObject]) -> PsResult<Built> {
    let mut built = Built {
        tree: Node::new(0, Kind::Column),
        handlers: HashMap::new(),
        results: BTreeMap::new(),
        rows: BTreeMap::new(),
        tables: BTreeMap::new(),
        trees: BTreeMap::new(),
        items: TreeItems::default(),
        canvases: BTreeMap::new(),
        next: 1,
    };
    let mut next = 1;
    let wraps = widgets.len() != 1;
    built.tree = match widgets {
        [one] => node_of(ps, one, &mut next, &mut built)?,
        many => {
            let mut root = Node::new(next, Kind::Column);
            next += 1;
            for w in many {
                root.children.push(node_of(ps, w, &mut next, &mut built)?);
            }
            root
        }
    };
    built.next = next;
    check_layout(&built.tree, None, true, wraps)?;
    check_menus_for(&built.tree, &built.tree)?;
    check_roles(&built.tree)?;
    Ok(built)
}

/// Describes tree nodes for [`tree_items`]: each node, then a hashtable's
/// children, as its `Depth`, `Text`, whether it has `More` children to come,
/// the `Node` itself, and its `Key`, in tree order. A string is a leaf with
/// that text; a hashtable gives its Text and its Children, in the same
/// forms; any other object takes its text from `$Text`, a property name or
/// a script block run with the node as `$_`, or else its own string, and
/// has children to come when `$HasChildren`, run with the node as `$_`,
/// returns true. The key is [`ItemKey`]'s, spelled `s` for a string, whose
/// key its text gives, `v` for a value, and `r` and the identity hash for
/// any other object. `$null` is passed over. A node that cannot be read
/// ends the description with one record of `Depth` -1 whose `Text` says
/// why.
const TREE_NODES: &str = r#"
param([object[]] $Nodes, $Text, $HasChildren)
$identity = [System.Runtime.CompilerServices.RuntimeHelpers]
# The variables for one InvokeWithContext, $_ as $Node: a list of its own
# for each call, since InvokeWithContext takes $_ out of the list it is given.
function Underscore($Node) {
    $variables = [System.Collections.Generic.List[psvariable]]::new()
    $variables.Add([psvariable]::new('_', $Node))
    , $variables
}
function Describe([object[]] $Nodes, [int] $Depth) {
    foreach ($node in $Nodes) {
        if ($null -eq $node) { continue }
        $base = $node.PSObject.BaseObject
        if ($base -is [string]) {
            [pscustomobject]@{ Depth = $Depth; Text = $base; More = $false; Node = $node; Key = 's' }
        } elseif ($base -is [System.Collections.IDictionary]) {
            if (-not $base.Contains('Text')) {
                $keys = @(foreach ($k in $base.Keys) { [string] $k })
                $has = if ($keys.Count) { "its keys are $($keys -join ', ')" } else { 'it has no keys' }
                throw "a tree's hashtable node gives its Text, and $has"
            }
            [pscustomobject]@{ Depth = $Depth; Text = [string] $base['Text']; More = $false; Node = $node; Key = 'r' + $identity::GetHashCode($base) }
            if ($base.Contains('Children')) { Describe @($base['Children']) ($Depth + 1) }
        } else {
            $shown = if ($null -eq $Text) { [string] $node }
                elseif ($Text -is [scriptblock]) { @($Text.InvokeWithContext($null, (Underscore $node), $node)) -join ' ' }
                else { [string] $node.$Text }
            $more = ($null -ne $HasChildren) -and [System.Management.Automation.LanguagePrimitives]::IsTrue($HasChildren.InvokeWithContext($null, (Underscore $node), $node))
            $key = if ($base -is [System.ValueType]) { 'v' }
                elseif ($base -is [System.Management.Automation.PSCustomObject]) { 'r' + $identity::GetHashCode($node) }
                else { 'r' + $identity::GetHashCode($base) }
            [pscustomobject]@{ Depth = $Depth; Text = $shown; More = $more; Node = $node; Key = $key }
        }
    }
}
try {
    Describe $Nodes 0
} catch {
    $why = $_.Exception
    while ($why -is [System.Management.Automation.MethodInvocationException] -and $why.InnerException) { $why = $why.InnerException }
    [pscustomobject]@{ Depth = -1; Text = $why.Message; More = $false; Node = $null }
}
"#;

/// Runs the script `text` on the calling thread with `args` as its
/// arguments and returns what it writes. It goes through the script
/// block's own Invoke rather than a command's pipeline, so the module's
/// static methods, which have none, run it as commands do. Invoke's
/// arguments are one `params object[]`, passed here as a single array,
/// so an argument that is itself an array stays one argument instead of
/// becoming the whole argument list.
fn run_script(text: &str, args: &[PsObject]) -> PsResult<Vec<PsObject>> {
    let block = PsType::from_name("scriptblock").call_static("Create", &[text.into_ps()?])?;
    Vec::<PsObject>::from_ps(&block.call("Invoke", &[args.to_vec().into_ps()?])?)
}

/// One tree node as [`tree_items`] reads it: its depth below the nodes
/// given, its text, whether it has children to come, the object it was made
/// from, and its key, none for a string, whose key its text gives.
struct Described {
    depth: usize,
    text: String,
    more: bool,
    object: PsObject,
    key: Option<ItemKey>,
}

/// Reads the objects of `run`, none of them a string, through
/// [`TREE_NODES`], after the nodes in `described`, and empties `run`.
fn describe_run(run: &mut Vec<PsObject>, spec: &TreeSpec, described: &mut Vec<Described>) -> PsResult<()> {
    if run.is_empty() {
        return Ok(());
    }
    let records = run_script(TREE_NODES, &[std::mem::take(run).into_ps()?, spec.text.clone(), spec.has_children.clone()])?;
    for d in &records {
        let depth = i64::from_ps(&pwrs::object::property(d, "Depth")?)?;
        if depth < 0 {
            let why = String::from_ps(&pwrs::object::property(d, "Text")?)?;
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadTreeNode", why).terminating());
        }
        let depth = usize::try_from(depth).map_err(|e| bad_tree(format!("a tree node at depth {depth}: {e}")))?;
        let key = match String::from_ps(&pwrs::object::property(d, "Key")?)?.as_str() {
            "s" => None,
            spelled => Some(key_spelled(spelled)?),
        };
        described.push(Described {
            depth,
            text: String::from_ps(&pwrs::object::property(d, "Text")?)?,
            more: bool::from_ps(&pwrs::object::property(d, "More")?)?,
            object: pwrs::object::property(d, "Node")?,
            key,
        });
    }
    Ok(())
}

/// A tree's nodes that could not be read, as an error.
fn bad_tree(text: String) -> PsError {
    PsError::new(ErrorCategory::InvalidResult, "GuiBadTreeNode", text).terminating()
}

/// The tree items `objects` describe, for the tree `tree`, under the item
/// `parent` or at the top when it is `None`: numbered by `number` in tree
/// order, each stored in `items` with the object it was made from and its
/// key. A string is a leaf with its own text, read with no script; the
/// other objects go through [`TREE_NODES`], those between two strings
/// together.
pub fn tree_items(
    spec: &TreeSpec,
    tree: NodeId,
    parent: Option<NodeId>,
    objects: &[PsObject],
    number: &mut dyn FnMut() -> PsResult<NodeId>,
    items: &mut TreeItems,
) -> PsResult<Vec<Node>> {
    let mut described = Vec::with_capacity(objects.len());
    let mut run = Vec::new();
    for o in objects {
        if o.is_null() {
            continue;
        }
        if o.type_tag()? == PS_TYPE_STRING {
            describe_run(&mut run, spec, &mut described)?;
            described.push(Described { depth: 0, text: String::from_ps(o)?, more: false, object: o.clone(), key: None });
        } else {
            run.push(o.clone());
        }
    }
    describe_run(&mut run, spec, &mut described)?;
    let mut text_keys = text_keys(described.iter().filter(|d| d.key.is_none()).map(|d| d.text.as_str()))?.into_iter();
    let mut top = Vec::new();
    let mut open: Vec<Node> = Vec::new();
    for d in described {
        if d.depth > open.len() {
            return Err(bad_tree(format!("a tree node at depth {} follows one at depth {}", d.depth, open.len())));
        }
        while open.len() > d.depth {
            attach(&mut open, &mut top);
        }
        let key = match d.key {
            Some(k) => k,
            None => text_keys.next().ok_or_else(|| bad_tree(format!("the tree node {} has no key", d.text)))?,
        };
        let id = number()?;
        let mut item = Node::new(id, Kind::TreeItem);
        item.props.insert(String::from("text"), Value::Str(d.text.clone()));
        if d.more {
            item.props.insert(String::from("more"), Value::Bool(true));
        }
        let under = match open.last() {
            Some(p) => Some(p.id),
            None => parent,
        };
        items.insert(id, Arc::new(TreeItem { tree, parent: under, text: d.text, object: d.object, key }));
        open.push(item);
    }
    while !open.is_empty() {
        attach(&mut open, &mut top);
    }
    Ok(top)
}

/// The keys of the strings `texts`, each its text in the invariant
/// culture's capitals, hashed. One call of `String.ToUpperInvariant` puts
/// all of them in capitals, joined, and since it changes no string's length
/// in UTF-16 units, the capitals are split again by those lengths.
fn text_keys<'a>(texts: impl Iterator<Item = &'a str>) -> PsResult<Vec<ItemKey>> {
    let mut joined = String::new();
    let mut lengths = Vec::new();
    for t in texts {
        joined.push_str(t);
        lengths.push(t.encode_utf16().count());
    }
    if lengths.is_empty() {
        return Ok(Vec::new());
    }
    let capitals = String::from_ps(&joined.as_str().into_ps()?.call("ToUpperInvariant", &[])?)?;
    let units: Vec<u16> = capitals.encode_utf16().collect();
    let total: usize = lengths.iter().sum();
    if units.len() != total {
        return Err(bad_tree(format!("the capitals of {} tree nodes' text came back {} UTF-16 units long, not {total}", lengths.len(), units.len())));
    }
    let mut keys = Vec::with_capacity(lengths.len());
    let mut at = 0;
    for len in lengths {
        let mut hash = DefaultHasher::new();
        units[at..at + len].hash(&mut hash);
        keys.push(ItemKey::Text(hash.finish()));
        at += len;
    }
    Ok(keys)
}

/// The key of each of `$Objects`, none of them a string or null, spelled as
/// [`TREE_NODES`] spells it: `v` for a value, `r` and the identity hash for
/// any other object.
const KEYS: &str = r#"
param([object[]] $Objects)
$identity = [System.Runtime.CompilerServices.RuntimeHelpers]
foreach ($o in $Objects) {
    $base = $o.PSObject.BaseObject
    if ($base -is [System.ValueType]) { 'v' }
    elseif ($base -is [System.Management.Automation.PSCustomObject]) { 'r' + $identity::GetHashCode($o) }
    else { 'r' + $identity::GetHashCode($base) }
}
"#;

/// The key [`TREE_NODES`] or [`KEYS`] spells `v`, or `r` and a number.
fn key_spelled(spelled: &str) -> PsResult<ItemKey> {
    if spelled == "v" {
        return Ok(ItemKey::Value);
    }
    let hash = spelled.strip_prefix('r').ok_or_else(|| bad_tree(format!("an object's key came back as {spelled:?}")))?;
    hash.parse::<i64>().map(ItemKey::Instance).map_err(|e| bad_tree(format!("an object's key came back as {spelled:?}: {e}")))
}

/// The key of each of `objects`, none for null: a string's read here, a
/// number's or other value's from its type, and any other object's through
/// [`KEYS`], all of them in one run.
pub fn item_keys(objects: &[PsObject]) -> PsResult<Vec<Option<ItemKey>>> {
    let mut keys = vec![None; objects.len()];
    let mut texts = Vec::new();
    let mut others = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        if o.is_null() {
            continue;
        }
        match o.type_tag()? {
            PS_TYPE_STRING => texts.push((i, String::from_ps(o)?)),
            PS_TYPE_OBJECT => others.push((i, o.clone())),
            _ => keys[i] = Some(ItemKey::Value),
        }
    }
    for ((i, _), key) in texts.iter().zip(text_keys(texts.iter().map(|(_, t)| t.as_str()))?) {
        keys[*i] = Some(key);
    }
    if !others.is_empty() {
        let at: Vec<usize> = others.iter().map(|(i, _)| *i).collect();
        let given: Vec<PsObject> = others.into_iter().map(|(_, o)| o).collect();
        let spelled = run_script(KEYS, &[given.into_ps()?])?;
        if spelled.len() != at.len() {
            return Err(bad_tree(format!("finding the keys of {} objects gave {}", at.len(), spelled.len())));
        }
        for (i, s) in at.iter().zip(&spelled) {
            keys[*i] = Some(key_spelled(&String::from_ps(s)?)?);
        }
    }
    Ok(keys)
}

/// For each of `wanted`, the items of the tree `tree` made from it, as
/// [`MADE_FROM`] matches them, compared with the items of its key alone.
pub fn tree_items_of(items: &TreeItems, tree: NodeId, wanted: &[PsObject]) -> PsResult<Vec<Vec<NodeId>>> {
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for key in item_keys(wanted)?.into_iter().flatten() {
        for id in items.keyed(tree, key) {
            let Some(item) = items.get(&id) else {
                continue;
            };
            if seen.insert(id) {
                candidates.push((id, &item.object));
            }
        }
    }
    if candidates.is_empty() {
        return Ok(vec![Vec::new(); wanted.len()]);
    }
    items_made_from(&candidates, wanted)
}

/// Puts the item last in `open`, finished, among the children of the one
/// before it, or among `top` when it is the first.
fn attach(open: &mut Vec<Node>, top: &mut Vec<Node>) {
    let Some(done) = open.pop() else {
        return;
    };
    match open.last_mut() {
        Some(parent) => parent.children.push(done),
        None => top.push(done),
    }
}

/// Finds tree nodes by the objects they were made from: for each of
/// `$Wanted`, the positions in `$Objects` of those it matches, as an array.
/// A string matches by its text without regard to case, a number by its
/// value, and any other object only the very instance a node was made from;
/// a custom object is its own instance, since every one shares one base
/// object.
const MADE_FROM: &str = r#"
param([object[]] $Objects, [object[]] $Wanted)
foreach ($w in $Wanted) {
    $found = [System.Collections.Generic.List[int]]::new()
    if ($null -ne $w) {
        $wb = $w.PSObject.BaseObject
        for ($i = 0; $i -lt $Objects.Count; $i++) {
            $o = $Objects[$i]
            if ($null -eq $o) { continue }
            $ob = $o.PSObject.BaseObject
            $same = if ($wb -is [string]) { ($ob -is [string]) -and [string]::Equals($wb, $ob, [System.StringComparison]::OrdinalIgnoreCase) }
                elseif ($wb -is [System.ValueType]) { ($ob -is [System.ValueType]) -and ($wb -eq $ob) }
                elseif ($wb -is [System.Management.Automation.PSCustomObject]) { [object]::ReferenceEquals($w, $o) }
                else { [object]::ReferenceEquals($wb, $ob) }
            if ($same) { $found.Add($i) }
        }
    }
    , $found.ToArray()
}
"#;

/// For each of `wanted`, the items of `candidates` made from it, as
/// [`MADE_FROM`] matches them.
pub fn items_made_from(candidates: &[(NodeId, &PsObject)], wanted: &[PsObject]) -> PsResult<Vec<Vec<NodeId>>> {
    let objects: Vec<PsObject> = candidates.iter().map(|(_, o)| (*o).clone()).collect();
    let found = run_script(MADE_FROM, &[objects.into_ps()?, wanted.to_vec().into_ps()?])?;
    if found.len() != wanted.len() {
        let text = format!("finding tree nodes gave {} lists for {} objects", found.len(), wanted.len());
        return Err(PsError::new(ErrorCategory::InvalidResult, "GuiBadTreeNode", text).terminating());
    }
    let mut out = Vec::with_capacity(found.len());
    for positions in &found {
        let mut ids = Vec::new();
        for at in Vec::<i64>::from_ps(positions)? {
            let at = usize::try_from(at).map_err(|e| PsError::new(ErrorCategory::InvalidResult, "GuiBadTreeNode", format!("position {at}: {e}")).terminating())?;
            match candidates.get(at) {
                Some((id, _)) => ids.push(*id),
                None => return Err(PsError::new(ErrorCategory::InvalidResult, "GuiBadTreeNode", format!("position {at} is past the {} nodes", candidates.len())).terminating()),
            }
        }
        out.push(ids);
    }
    Ok(out)
}


/// What makes a table's cells from its objects: the properties -Property
/// named, or the columns Out-GuiGridView shows for the first object, each
/// with its label and its format. A table given neither -Property nor an
/// object has none until its first object comes and gives them.
pub struct TableColumns {
    pub labels: Vec<String>,
    formats: Vec<Option<String>>,
    /// Each column's alignment as its table view gives it, when it does.
    aligns: Vec<Option<String>>,
    calc: Vec<PsObject>,
    keys: Vec<String>,
    named: bool,
}

impl TableColumns {
    /// Whether the columns are still to come from the first object added.
    pub fn to_come(&self) -> bool {
        !self.named && self.labels.is_empty()
    }

    /// The columns as the renderer's `columns` property lists them.
    pub fn value(&self) -> Value {
        Value::List(self.labels.iter().enumerate().map(|(i, l)| crate::grid::column_value(l, self.aligns.get(i).and_then(Option::as_deref))).collect())
    }
}

/// A table's columns: the properties `property` names, or those
/// Out-GuiGridView shows for `first`; none when there is neither.
pub fn table_columns(ps: &Pipeline<'_>, first: Option<&PsObject>, property: Option<Vec<String>>) -> PsResult<TableColumns> {
    if let Some(names) = property {
        let formats = vec![None; names.len()];
        let calc = names.iter().map(|n| n.clone().into_ps()).collect::<PsResult<Vec<PsObject>>>()?;
        let keys = names.clone();
        return Ok(TableColumns { labels: names, formats, aligns: Vec::new(), calc, keys, named: true });
    }
    let mut columns = TableColumns { labels: Vec::new(), formats: Vec::new(), aligns: Vec::new(), calc: Vec::new(), keys: Vec::new(), named: false };
    let Some(first) = first else {
        return Ok(columns);
    };
    let script = PsScriptBlock(PsType::from_name("scriptblock").call_static("Create", &[crate::grid::COLUMNS.into_ps()?])?);
    for c in script.call(ps, std::slice::from_ref(first))? {
        columns.labels.push(String::from_ps(&pwrs::object::property(&c, "Label")?)?);
        columns.formats.push(Option::<String>::from_ps(&pwrs::object::property(&c, "Format")?)?.filter(|f| !f.is_empty()));
        columns.aligns.push(Option::<String>::from_ps(&pwrs::object::property(&c, "Align")?)?.filter(|a| !a.is_empty()));
        columns.calc.push(pwrs::object::property(&c, "Calc")?);
    }
    columns.keys = (0..columns.labels.len()).map(|i| format!("c{i}")).collect();
    Ok(columns)
}

/// The rows of `objects`, each its cells in the order of `columns`.
pub fn table_cells(ps: &Pipeline<'_>, objects: &[PsObject], columns: &TableColumns) -> PsResult<Vec<Row>> {
    if objects.is_empty() || columns.labels.is_empty() {
        return Ok(Vec::new());
    }
    let evaluated = ps.invoke_with_input("Select-Object", &[("Property", columns.calc.clone().into_ps()?)], Some(&objects.to_vec().into_ps()?))?;
    if evaluated.len() != objects.len() {
        let text = format!("Select-Object returned {} rows for {} objects", evaluated.len(), objects.len());
        return Err(PsError::new(ErrorCategory::InvalidResult, "GuiRowCount", text).terminating());
    }
    let mut rows = Vec::with_capacity(evaluated.len());
    for e in &evaluated {
        let mut cells = Vec::with_capacity(columns.keys.len());
        for (key, format) in columns.keys.iter().zip(&columns.formats) {
            cells.push(crate::grid::cell(&pwrs::object::property(e, key)?, format.as_deref())?);
        }
        rows.push(Row { cells });
    }
    Ok(rows)
}

/// A table's row as the module stores it while its window is open: its
/// number, which no other row of the table has had, the object it was made
/// from, and that object's key, none for null.
pub struct TableRow {
    number: u64,
    pub object: PsObject,
    key: Option<ItemKey>,
}

/// A table's rows while its window is open, in row order, which is the
/// order of their numbers, each under its object's key as well, so that
/// finding the rows made from an object compares it with those of its key
/// alone, as a tree's items are found; and what makes new rows' cells. The
/// rows are behind `Arc`, so a copy of them takes counts and no handle of
/// the engine's.
#[derive(Clone)]
pub struct TableRows {
    rows: Vec<Arc<TableRow>>,
    by_key: BTreeSet<(ItemKey, u64)>,
    next: u64,
    pub columns: Arc<TableColumns>,
}

impl TableRows {
    /// The rows made from `objects`, with `columns` making the cells of the
    /// rows added later.
    pub fn new(objects: Vec<PsObject>, columns: TableColumns) -> PsResult<TableRows> {
        let mut rows = TableRows { rows: Vec::with_capacity(objects.len()), by_key: BTreeSet::new(), next: 0, columns: Arc::new(columns) };
        let keys = item_keys(&objects)?;
        rows.push(objects, keys);
        Ok(rows)
    }

    /// Adds rows made from `objects`, whose keys are `keys`, after the rows
    /// there.
    pub fn push(&mut self, objects: Vec<PsObject>, keys: Vec<Option<ItemKey>>) {
        for (object, key) in objects.into_iter().zip(keys) {
            let number = self.next;
            self.next += 1;
            if let Some(k) = key {
                self.by_key.insert((k, number));
            }
            self.rows.push(Arc::new(TableRow { number, object, key }));
        }
    }

    /// How many rows there are.
    pub fn count(&self) -> usize {
        self.rows.len()
    }

    /// The object row `row` was made from, counting from 0 in row order.
    pub fn object(&self, row: usize) -> Option<&PsObject> {
        self.rows.get(row).map(|r| &r.object)
    }

    /// The place in row order of the row numbered `number`.
    fn place(&self, number: u64) -> Option<usize> {
        let at = self.rows.partition_point(|r| r.number < number);
        self.rows.get(at).is_some_and(|r| r.number == number).then_some(at)
    }

    /// For each of `wanted`, the numbers of the rows made from it, as
    /// [`MADE_FROM`] matches them, compared with the rows of its key alone.
    pub fn made_from(&self, wanted: &[PsObject]) -> PsResult<Vec<Vec<u64>>> {
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for key in item_keys(wanted)?.into_iter().flatten() {
            for (_, number) in self.by_key.range((key, u64::MIN)..=(key, u64::MAX)) {
                if seen.insert(*number)
                    && let Some(at) = self.place(*number)
                {
                    let tag = NodeId::try_from(candidates.len()).map_err(|e| PsError::new(ErrorCategory::LimitsExceeded, "GuiBadState", format!("{} rows to compare: {e}", candidates.len())).terminating())?;
                    candidates.push((tag, &self.rows[at]));
                }
            }
        }
        if candidates.is_empty() {
            return Ok(vec![Vec::new(); wanted.len()]);
        }
        let compared: Vec<(NodeId, &PsObject)> = candidates.iter().map(|(tag, row)| (*tag, &row.object)).collect();
        let found = items_made_from(&compared, wanted)?;
        let mut out = Vec::with_capacity(found.len());
        for tags in found {
            let mut numbers = Vec::with_capacity(tags.len());
            for tag in tags {
                let at = usize::try_from(tag).map_err(|e| PsError::new(ErrorCategory::InvalidResult, "GuiBadState", format!("row {tag}: {e}")).terminating())?;
                let Some((_, row)) = candidates.get(at) else {
                    return Err(PsError::new(ErrorCategory::InvalidResult, "GuiBadState", format!("row {tag} is past the {} rows compared", candidates.len())).terminating());
                };
                numbers.push(row.number);
            }
            out.push(numbers);
        }
        Ok(out)
    }

    /// The places in row order, counting from 0, of the rows numbered in
    /// `numbers` that are still there.
    pub fn places(&self, numbers: &BTreeSet<u64>) -> Vec<u64> {
        let mut out = Vec::with_capacity(numbers.len());
        for (at, row) in (0u64..).zip(&self.rows) {
            if numbers.contains(&row.number) {
                out.push(at);
            }
        }
        out
    }

    /// The rows without those numbered in `numbers`.
    pub fn without(&self, numbers: &BTreeSet<u64>) -> TableRows {
        let mut kept = TableRows { rows: Vec::with_capacity(self.rows.len()), by_key: self.by_key.clone(), next: self.next, columns: Arc::clone(&self.columns) };
        for row in &self.rows {
            if numbers.contains(&row.number) {
                if let Some(k) = row.key {
                    kept.by_key.remove(&(k, row.number));
                }
            } else {
                kept.rows.push(Arc::clone(row));
            }
        }
        kept
    }
}

/// The node of one widget's description and its children's, numbered from
/// `next` in tree order; a table's rows and objects go into `built`. A Tabs
/// shows its first tab unless Selected names another.
fn node_of(ps: &Pipeline<'_>, obj: &PsObject, next: &mut NodeId, built: &mut Built) -> PsResult<Node> {
    let w = GuiWidget::from_ps(obj)?;
    let Some(kind) = kind_named(&w.kind) else {
        let text = format!("{} is not a widget; the widgets are {}", w.kind, WIDGETS.join(", "));
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiUnknownWidget", text).terminating());
    };
    let id = *next;
    *next += 1;
    let mut node = Node::new(id, kind);
    node.name = w.name.clone();
    for c in &w.children {
        node.children.push(node_of(ps, c, next, built)?);
    }
    let titles = if kind == Kind::Tabs { choices_of(&node)? } else { Vec::new() };
    node.props = wire_props(ps, kind, &w.properties, &titles, true)?;
    if kind == Kind::Tabs && !titles.is_empty() && !node.props.contains_key("selected") {
        node.props.insert(String::from("selected"), Value::Int(0));
    }
    settle(kind, &BTreeMap::new(), None, &mut node.props)?;
    if kind == Kind::Canvas {
        let board = Board::of(&node).map_err(|e| PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", format!("Item: {e}")).terminating())?;
        built.canvases.insert(id, board);
    }
    if kind == Kind::Table {
        let given = PsReadOnlyTable::over(&w.properties)?;
        let objects = Vec::<PsObject>::from_ps(&given.get("Item")?)?;
        let property = Option::<Vec<String>>::from_ps(&given.get("Property")?)?;
        let columns = table_columns(ps, objects.first(), property)?;
        node.props.insert(String::from("columns"), columns.value());
        built.rows.insert(id, table_cells(ps, &objects, &columns)?);
        built.tables.insert(id, TableRows::new(objects, columns)?);
    }
    if kind == Kind::TreeView {
        let given = PsReadOnlyTable::over(&w.properties)?;
        let spec = TreeSpec { text: given.get("Text")?, has_children: given.get("HasChildren")? };
        let nodes = Vec::<PsObject>::from_ps(&given.get("Node")?)?;
        let mut number = || -> PsResult<NodeId> {
            let n = *next;
            *next = n.checked_add(1).ok_or_else(|| PsError::new(ErrorCategory::LimitsExceeded, "GuiNoNodeNumber", "the window has used every node number").terminating())?;
            Ok(n)
        };
        node.children = tree_items(&spec, id, None, &nodes, &mut number, &mut built.items)?;
        built.trees.insert(id, spec);
    }
    let handlers = PsReadOnlyTable::over(&w.handlers)?;
    for key in handlers.keys()? {
        let Some(event) = event_named(kind, &key) else {
            let text = format!("a {} raises no {key} event", widget_name(kind));
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiUnknownEvent", text).terminating());
        };
        // The renderer raises these only for a node that takes them; a
        // canvas takes its items' clicks too, which every other widget
        // raises in any case.
        let asks = match event {
            EventKind::Drop => Some("drop"),
            EventKind::Key => Some("keys"),
            EventKind::PointerDown => Some("pointerDown"),
            EventKind::PointerUp => Some("pointerUp"),
            EventKind::PointerMove => Some("pointerMove"),
            EventKind::Wheel => Some("wheel"),
            EventKind::Click if kind == Kind::Canvas => Some("click"),
            EventKind::DoubleClick => Some("doubleClick"),
            EventKind::PointerEnter => Some("pointerEnter"),
            EventKind::PointerLeave => Some("pointerLeave"),
            EventKind::DragStart => Some("dragStart"),
            EventKind::DragMove => Some("dragMove"),
            EventKind::DragEnd => Some("dragEnd"),
            EventKind::Position => Some("position"),
            EventKind::Click
            | EventKind::Change
            | EventKind::Select
            | EventKind::Activate
            | EventKind::Submit
            | EventKind::Shortcut
            | EventKind::Expand
            | EventKind::Peek
            | EventKind::Loaded
            | EventKind::Playing
            | EventKind::Paused
            | EventKind::Ended
            | EventKind::Error => None,
        };
        if let Some(flag) = asks {
            node.props.insert(flag.to_string(), Value::Bool(true));
        }
        built.handlers.insert((id, event), PsScriptBlock(handlers.get(&key)?));
    }
    if let Some(r) = w.result {
        built.results.insert(id, r);
    }
    Ok(node)
}

/// A list's items, as its `items` property lists them.
fn items_of(node: &Node) -> Result<Vec<String>, String> {
    match node.props.get("items") {
        None => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(|i| i.as_str().map(str::to_string).ok_or_else(|| format!("items must be text, and one is {i:?}"))).collect(),
        Some(other) => Err(format!("items must be a list of text, not {other:?}")),
    }
}

/// The row numbers in a selection: one, or a list of them.
fn rows_in(value: Option<&Value>) -> Result<Vec<usize>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(row).collect(),
        Some(one) => Ok(vec![row(one)?]),
    }
}

/// One row number: a non-negative integer.
fn row(v: &Value) -> Result<usize, String> {
    let i = v.as_int().ok_or_else(|| format!("a row number is an integer, not {v:?}"))?;
    usize::try_from(i).map_err(|e| format!("{i} is not a row number: {e}"))
}

/// The value of a named input in the form PowerShell is given, as a
/// protocol value: a list's selected item, or with -Multiple its items, and
/// a drop-down's or option group's choice, as text, from the renderer's row
/// numbers; a table's rows and a tree's items as the renderer's numbers, a
/// list of them, which the module turns into their objects; a number
/// field's or slider's number as a `Float`; a check box's flag, a text
/// field's text and a date picker's date as they are; a file chooser's
/// path, or with -Multiple its paths. Needs no PowerShell, so any thread
/// can work it out.
pub fn presented_value(node: &Node, value: Option<&Value>) -> Result<Value, String> {
    match node.kind {
        Kind::ListBox => {
            let items = items_of(node)?;
            let multiple = matches!(node.props.get("multiple"), Some(Value::Bool(true)));
            let mut chosen = Vec::new();
            for r in rows_in(value)? {
                match items.get(r) {
                    Some(item) => chosen.push(Value::Str(item.clone())),
                    None => return Err(format!("row {r} is beyond the list's {} items", items.len())),
                }
            }
            if multiple {
                return Ok(Value::List(chosen));
            }
            Ok(match chosen.into_iter().next() {
                Some(c) => c,
                None => Value::Null,
            })
        }
        Kind::Table | Kind::TreeView => {
            let mut rows = Vec::new();
            for r in rows_in(value)? {
                rows.push(Value::Int(i64::try_from(r).map_err(|e| format!("row {r} does not fit the protocol: {e}"))?));
            }
            Ok(Value::List(rows))
        }
        Kind::CheckBox => match value {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(Value::Bool(b)) => Ok(Value::Bool(*b)),
            Some(other) => Err(format!("a check box's value is {other:?}, which is not true or false")),
        },
        Kind::TextBox | Kind::ColorPicker | Kind::DatePicker => match value {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(Value::Str(s)) => Ok(Value::Str(s.clone())),
            Some(other) => Err(format!("a {}'s value is {other:?}, which is not text", widget_name(node.kind))),
        },
        Kind::FilePicker => {
            let paths = match value {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::List(paths)) => paths.clone(),
                Some(other) => return Err(format!("a FilePicker's value is {other:?}, which is not a list of paths")),
            };
            if let Some(bad) = paths.iter().find(|p| p.as_str().is_none()) {
                return Err(format!("a FilePicker's paths include {bad:?}, which is not text"));
            }
            if matches!(node.props.get("multiple"), Some(Value::Bool(true))) {
                return Ok(Value::List(paths));
            }
            Ok(match paths.into_iter().next() {
                Some(p) => p,
                None => Value::Null,
            })
        }
        Kind::ComboBox if is_editable(node) => match value {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(Value::Str(s)) => Ok(Value::Str(s.clone())),
            Some(other) => Err(format!("an editable ComboBox's value is {other:?}, which is not text")),
        },
        Kind::NumberBox | Kind::Slider => match value {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(v) => v.as_number().map(Value::Float).ok_or_else(|| format!("a {}'s value is {v:?}, which is not a number", widget_name(node.kind))),
        },
        Kind::ComboBox | Kind::RadioGroup => match value {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(v) => {
                let r = row(v)?;
                let items = items_of(node)?;
                match items.get(r) {
                    Some(item) => Ok(Value::Str(item.clone())),
                    None => Err(format!("row {r} is beyond the {} choices", items.len())),
                }
            }
        },
        Kind::Tabs => match value {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(v) => {
                let r = row(v)?;
                let titles = tab_titles(node)?;
                match titles.get(r) {
                    Some(title) => Ok(Value::Str(title.clone())),
                    None => Err(format!("tab {r} is beyond the {} tabs", titles.len())),
                }
            }
        },
        Kind::Column
        | Kind::Row
        | Kind::Grid
        | Kind::Group
        | Kind::Tab
        | Kind::Split
        | Kind::Scroll
        | Kind::Text
        | Kind::Image
        | Kind::Markdown
        | Kind::ProgressBar
        | Kind::Spinner
        | Kind::LogView
        | Kind::Chart
        | Kind::GpuSurface
        | Kind::Canvas
        | Kind::MediaPlayer
        | Kind::Button
        | Kind::MenuBar
        | Kind::Menu
        | Kind::MenuItem
        | Kind::ContextMenu
        | Kind::StatusBar
        | Kind::ButtonBar
        | Kind::TreeItem
        | Kind::Separator
        | Kind::Spacer
        | Kind::Html => Ok(match value {
            Some(v) => v.clone(),
            None => Value::Null,
        }),
    }
}

/// The items of the list `node`, which a selection names by their text.
pub fn list_items(node: &Node) -> PsResult<Vec<String>> {
    items_of(node).map_err(|e| PsError::new(ErrorCategory::InvalidData, "GuiBadState", format!("{} {}: {e}", widget_name(node.kind), node.id)).terminating())
}

/// The titles of the tabs in a Tabs, in order.
fn tab_titles(node: &Node) -> Result<Vec<String>, String> {
    node.children
        .iter()
        .map(|tab| match tab.props.get("title") {
            Some(Value::Str(t)) => Ok(t.clone()),
            other => Err(format!("tab {} has a title that is not text: {other:?}", tab.id)),
        })
        .collect()
}

/// The texts a widget's Selected names one of: a list's, drop-down's or
/// option group's items, a set of tabs' titles; none for any other widget.
pub fn choices_of(node: &Node) -> PsResult<Vec<String>> {
    let found = match node.kind {
        Kind::ListBox | Kind::ComboBox | Kind::RadioGroup => items_of(node),
        Kind::Tabs => tab_titles(node),
        _ => Ok(Vec::new()),
    };
    found.map_err(|e| PsError::new(ErrorCategory::InvalidData, "GuiBadState", format!("{} {}: {e}", widget_name(node.kind), node.id)).terminating())
}

/// Fails when a Tab is anywhere but in a Tabs, a Tabs contains anything but
/// Tab widgets, a Split contains other than two widgets, a MenuBar or
/// StatusBar is not among the window's own widgets, a menu contains or sits
/// in anything a menu does not, or a tree's items are anywhere but in a
/// tree. The window's own widgets are those its block wrote: `top` says
/// `node` is one of them, and `wraps` that `node` is the Column the window
/// puts them in, so its children are the window's own.
fn check_layout(node: &Node, parent: Option<Kind>, top: bool, wraps: bool) -> PsResult<()> {
    let refuse = |id: &'static str, text: String| -> PsResult<()> { Err(PsError::new(ErrorCategory::InvalidArgument, id, text).terminating()) };
    if node.kind == Kind::TreeItem && !matches!(parent, Some(Kind::TreeView | Kind::TreeItem)) {
        return refuse("GuiNotInTree", String::from("a tree item sits in a TreeView"));
    }
    if matches!(node.kind, Kind::TreeView | Kind::TreeItem)
        && let Some(c) = node.children.iter().find(|c| c.kind != Kind::TreeItem)
    {
        return refuse("GuiNotInTree", format!("a TreeView contains only its nodes, and this one contains a {}", widget_name(c.kind)));
    }
    if node.kind == Kind::Tab && parent != Some(Kind::Tabs) {
        let which = match node.props.get("title").and_then(Value::as_str) {
            Some(t) => format!("the Tab {t}"),
            None => String::from("a Tab"),
        };
        return refuse("GuiTabOutsideTabs", format!("{which} is not inside a Tabs; a Tab is one tab of a Tabs"));
    }
    if node.kind == Kind::Tabs
        && let Some(c) = node.children.iter().find(|c| c.kind != Kind::Tab)
    {
        return refuse("GuiNotATab", format!("a Tabs contains only Tab widgets, and this one contains a {}", widget_name(c.kind)));
    }
    if node.kind == Kind::Split && node.children.len() != 2 {
        return refuse("GuiNotTwoWidgets", format!("a Split contains two widgets, and this one contains {}", node.children.len()));
    }
    if matches!(node.kind, Kind::MenuBar | Kind::StatusBar | Kind::ButtonBar) && !top {
        return refuse("GuiNotAtTop", format!("a {} stands among the window's own widgets, not inside another", widget_name(node.kind)));
    }
    if node.kind == Kind::ButtonBar
        && let Some(c) = node.children.iter().find(|c| c.kind != Kind::Button)
    {
        return refuse("GuiNotAButton", format!("a ButtonBar contains only Button widgets, and this one contains a {}", widget_name(c.kind)));
    }
    let menu_parent = matches!(parent, Some(Kind::MenuBar | Kind::Menu | Kind::ContextMenu));
    if node.kind == Kind::Menu && !menu_parent {
        return refuse("GuiNotInMenu", String::from("a Menu sits in a MenuBar, a ContextMenu or another Menu"));
    }
    if node.kind == Kind::MenuItem && !matches!(parent, Some(Kind::Menu | Kind::ContextMenu)) {
        return refuse("GuiNotInMenu", String::from("a MenuItem sits in a Menu or a ContextMenu"));
    }
    if node.kind == Kind::MenuBar
        && let Some(c) = node.children.iter().find(|c| c.kind != Kind::Menu)
    {
        return refuse("GuiNotInMenu", format!("a MenuBar contains only Menu widgets, and this one contains a {}", widget_name(c.kind)));
    }
    if matches!(node.kind, Kind::Menu | Kind::ContextMenu)
        && let Some(c) = node.children.iter().find(|c| !matches!(c.kind, Kind::MenuItem | Kind::Menu | Kind::Separator))
    {
        return refuse("GuiNotInMenu", format!("a {} contains only MenuItem, Menu and Separator widgets, and this one contains a {}", widget_name(node.kind), widget_name(c.kind)));
    }
    node.children.iter().try_for_each(|c| check_layout(c, Some(node.kind), wraps, false))
}

/// Fails when the window has more than one ButtonBar, default button or
/// cancel button, naming the two first found.
fn check_roles(tree: &Node) -> PsResult<()> {
    let mut bars = Vec::new();
    let mut defaults = Vec::new();
    let mut cancels = Vec::new();
    gather_roles(tree, &mut bars, &mut defaults, &mut cancels);
    let refuse = |text: String| Err(PsError::new(ErrorCategory::InvalidArgument, "GuiButtonRole", text).terminating());
    let shown = |n: &Node| match (&n.name, n.props.get("text").and_then(Value::as_str)) {
        (Some(name), _) => name.clone(),
        (None, Some(text)) => text.to_string(),
        (None, None) => format!("{}", n.id),
    };
    if bars.len() > 1 {
        return refuse(format!("a window has one ButtonBar, and this one has {}", bars.len()));
    }
    for (found, role) in [(&defaults, "-Default"), (&cancels, "-Cancel")] {
        if let [first, second, ..] = found.as_slice() {
            let which = if role == "-Default" { "default" } else { "cancel" };
            return refuse(format!("{role} is set on two buttons, {} and {}; a window has one {which} button", shown(first), shown(second)));
        }
    }
    Ok(())
}

/// The ButtonBars, default buttons and cancel buttons of a subtree, in
/// tree order.
fn gather_roles<'a>(node: &'a Node, bars: &mut Vec<&'a Node>, defaults: &mut Vec<&'a Node>, cancels: &mut Vec<&'a Node>) {
    let set = |key: &str| matches!(node.props.get(key), Some(Value::Bool(true)));
    match node.kind {
        Kind::ButtonBar => bars.push(node),
        Kind::Button if set("default") => defaults.push(node),
        Kind::Button if set("cancel") => cancels.push(node),
        _ => {}
    }
    for c in &node.children {
        gather_roles(c, bars, defaults, cancels);
    }
}

/// Fails when a ContextMenu's For names no widget of the window.
fn check_menus_for(node: &Node, tree: &Node) -> PsResult<()> {
    if node.kind == Kind::ContextMenu {
        let target = match node.props.get("for") {
            Some(Value::Str(name)) => name.as_str(),
            _ => "",
        };
        if tree.find_named(target).is_none() {
            let text = format!("a ContextMenu is for the widget named {target}, and the window has none; its widgets are {}", tree.names().join(", "));
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoWidget", text).terminating());
        }
    }
    node.children.iter().try_for_each(|c| check_menus_for(c, tree))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(tree: NodeId, parent: Option<NodeId>, key: ItemKey) -> Arc<TreeItem> {
        Arc::new(TreeItem { tree, parent, text: String::new(), object: PsObject::null(), key })
    }

    #[test]
    fn a_pictures_format_is_told_by_its_bytes_and_any_other_named() {
        let bmp: Vec<u8> = [&b"BM"[..], &[0; 12], &[40, 0, 0, 0], &[0; 8]].concat();
        let cases: [(&[u8], &str); 8] = [
            (b"\x89PNG\r\n\x1a\n....", "png"),
            (&[0xFF, 0xD8, 0xFF, 0xE0], "jpeg"),
            (b"GIF89a....", "gif"),
            (b"RIFF\0\0\0\0WEBPVP8 ", "webp"),
            (&bmp, "bmp"),
            (&[0, 0, 1, 0, 1, 0], "ico"),
            (b"\xEF\xBB\xBF  <?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>", "svg"),
            (b"<!-- a logo -->\n<svg width=\"4\" height=\"4\"></svg>", "svg"),
        ];
        for (bytes, format) in cases {
            match picture_format(bytes) {
                Ok(told) => assert_eq!(told, format, "{bytes:?}"),
                Err(why) => panic!("{bytes:?} was refused: {why}"),
            }
        }
        let refused = |bytes: &[u8]| match picture_format(bytes) {
            Ok(told) => panic!("{bytes:?} was taken for {told}"),
            Err(why) => why,
        };
        assert!(refused(b"II*\0....").contains("a TIFF image"));
        assert!(refused(b"\0\0\0\x1cftypavif....").contains("an AVIF image"));
        assert!(refused(b"BM not a bitmap at all").contains("42 4d 20 6e 6f 74 20 61"), "a BM without a bitmap's header is named by its bytes");
        assert!(refused(b"").contains("no bytes at all"));
        assert!(refused(b"<html><body>no picture</body></html>").contains("3c 68 74 6d 6c 3e 3c 62"), "markup without an svg element is no SVG");
    }

    #[test]
    fn a_scroll_height_log_width_or_height_or_spacer_size_below_zero_points_is_refused() {
        for (kind, key) in [(Kind::Scroll, "height"), (Kind::LogView, "width"), (Kind::LogView, "height"), (Kind::Spacer, "size")] {
            for n in [-1.0, f64::NAN] {
                let mut props = BTreeMap::from([(String::from(key), Value::Float(n))]);
                match settle(kind, &BTreeMap::new(), None, &mut props) {
                    Ok(()) => panic!("a {}'s {key} of {n} was taken", widget_name(kind)),
                    Err(e) => assert_eq!(e.error_id, "GuiBadSize", "{}", e.message),
                }
            }
            for n in [0.0, 120.0] {
                let mut props = BTreeMap::from([(String::from(key), Value::Float(n))]);
                match settle(kind, &BTreeMap::new(), None, &mut props) {
                    Ok(()) => {}
                    Err(e) => panic!("a {}'s {key} of {n} was refused: {}", widget_name(kind), e.message),
                }
            }
        }
    }

    #[test]
    fn tree_items_are_found_by_their_tree_and_key_alone() {
        let mut items = TreeItems::default();
        items.insert(2, item(1, None, ItemKey::Text(7)));
        items.insert(3, item(1, None, ItemKey::Text(7)));
        items.insert(4, item(1, None, ItemKey::Instance(7)));
        items.insert(5, item(9, None, ItemKey::Text(7)));
        items.insert(6, item(1, None, ItemKey::Value));
        assert_eq!(items.keyed(1, ItemKey::Text(7)).collect::<Vec<_>>(), [2, 3]);
        assert_eq!(items.keyed(1, ItemKey::Instance(7)).collect::<Vec<_>>(), [4]);
        assert_eq!(items.keyed(9, ItemKey::Text(7)).collect::<Vec<_>>(), [5]);
        assert_eq!(items.keyed(1, ItemKey::Value).collect::<Vec<_>>(), [6]);
        assert_eq!(items.keyed(1, ItemKey::Text(8)).count(), 0);
        items.remove(2);
        assert_eq!(items.keyed(1, ItemKey::Text(7)).collect::<Vec<_>>(), [3]);
        assert!(items.get(&2).is_none());
        assert_eq!(items.by_node.len(), 4);
        assert_eq!(items.by_key.len(), 4);
    }

    #[test]
    fn an_item_put_again_is_found_by_its_new_key_only() {
        let mut items = TreeItems::default();
        items.insert(2, item(1, None, ItemKey::Text(7)));
        items.insert(2, item(1, None, ItemKey::Instance(3)));
        assert_eq!(items.keyed(1, ItemKey::Text(7)).count(), 0);
        assert_eq!(items.keyed(1, ItemKey::Instance(3)).collect::<Vec<_>>(), [2]);
        assert_eq!(items.by_key.len(), 1);
    }

    #[test]
    fn the_items_under_a_root_include_it_and_every_item_below_it() {
        let mut items = TreeItems::default();
        items.insert(2, item(1, None, ItemKey::Value));
        items.insert(3, item(1, Some(2), ItemKey::Value));
        items.insert(4, item(1, Some(3), ItemKey::Value));
        items.insert(5, item(1, None, ItemKey::Value));
        items.insert(6, item(1, Some(5), ItemKey::Value));
        assert_eq!(items.under(&BTreeSet::from([3])), BTreeSet::from([3, 4]));
        assert_eq!(items.under(&BTreeSet::from([2, 6])), BTreeSet::from([2, 3, 4, 6]));
        let mut more = TreeItems::default();
        more.insert(7, item(1, Some(4), ItemKey::Text(1)));
        items.extend(&more);
        assert_eq!(items.under(&BTreeSet::from([2])), BTreeSet::from([2, 3, 4, 7]));
        assert_eq!(items.keyed(1, ItemKey::Text(1)).collect::<Vec<_>>(), [7]);
    }
}
