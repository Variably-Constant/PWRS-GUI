//! The dialogs this renderer draws where the system has none of their kind,
//! as on a Linux or BSD desktop with neither the XDG portal nor zenity: a
//! message drawn as a dialog, its icon beside its text inside a dialog's
//! margin and its buttons at the right of a button bar in the system's
//! order, and a file dialog of the renderer's own for files to open, a file
//! to save or a folder, both from the shared tokens. Each stays listed and
//! answerable under its dialog's number, as a native dialog is, and closes
//! at a close or at a test's answer.

use super::{button_of, egui, look};
use crossbeam_queue::SegQueue;
use pwrs_gui_dialogs::{Drawn, StandIn};
use pwrs_gui_proto::button_bar::{self, Role};
use pwrs_gui_proto::tokens;
use pwrs_gui_proto::{DialogAnswer, DialogKind, MessageButton, MessageButtons, MessageIcon, RequestId};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{MAIN_SEPARATOR, MAIN_SEPARATOR_STR, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Where the dialogs given to the renderer wait for its next frame.
pub(super) type Waiting = Arc<SegQueue<Drawn>>;

/// The hook pwrs-gui-dialogs gives a dialog the system has none of: the
/// dialog waits in `waiting` for the frame `wake` requests.
pub(super) fn hook(waiting: &Waiting, wake: impl FnOnce() + Send + 'static) -> StandIn {
    let waiting = Arc::clone(waiting);
    Box::new(move |drawn| {
        waiting.push(drawn);
        wake();
    })
}

/// The viewport dialog `request` is drawn in.
pub(super) fn viewport_id(request: RequestId) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("pwrs-gui-stand-in", request))
}

/// The dialogs given to the renderer and waiting for its next frame, and
/// those it has open, by number.
#[derive(Default)]
pub(super) struct StandIns {
    waiting: Waiting,
    open: BTreeMap<RequestId, Dialog>,
}

/// A drawn dialog while it is open.
struct Dialog {
    drawn: Drawn,
    /// Set when a close or a test's answer closes it.
    closing: Arc<AtomicBool>,
    form: Form,
    /// Its window's inside size in points as it opens.
    size: [f32; 2],
    takes_focus: bool,
}

impl StandIns {
    /// Where [`hook`] leaves dialogs; a window's file choosers leave theirs
    /// in the same place.
    pub(super) fn waiting(&self) -> &Waiting {
        &self.waiting
    }

    /// Opens the dialogs given since the last frame, each with what closes
    /// it given to the registry, and returns a line for the log on each
    /// saying why the system had none. A close requests a frame of the root,
    /// whose [`StandIns::viewports`] lets the dialog go.
    pub(super) fn take(&mut self, ctx: &egui::Context, no_focus: bool) -> Vec<String> {
        let mut notes = Vec::new();
        while let Some(drawn) = self.waiting.pop() {
            let request = drawn.request();
            notes.push(format!("dialog {request} is drawn by the renderer, since the system has none of its kind: {}", drawn.why()));
            let closing = Arc::new(AtomicBool::new(false));
            let (flag, wake) = (Arc::clone(&closing), ctx.clone());
            drawn.shown(Box::new(move || {
                flag.store(true, Ordering::Release);
                wake.request_repaint_of(egui::ViewportId::ROOT);
            }));
            let form = Form::of(drawn.kind());
            let size = form.size(ctx);
            self.open.insert(request, Dialog { drawn, closing, form, size, takes_focus: !no_focus });
        }
        notes
    }

    /// Lets go of each dialog a close or a test closed, giving it the answer
    /// Canceled, which the registry replaces with the answer the close or
    /// the test gave, and returns the viewport of each one still open.
    pub(super) fn viewports(&mut self) -> Vec<(RequestId, egui::ViewportBuilder)> {
        let closed: Vec<RequestId> = self.open.iter().filter(|(_, d)| d.closing.load(Ordering::Acquire)).map(|(request, _)| *request).collect();
        for request in closed {
            self.finish(request, DialogAnswer::Canceled);
        }
        self.open.iter().map(|(request, d)| (*request, d.builder())).collect()
    }

    /// Draws a frame of dialog `request` in its viewport, and once a person
    /// answers it, presses Escape or closes its window, gives the answer and
    /// lets the dialog go.
    pub(super) fn frame(&mut self, request: RequestId, ui: &mut egui::Ui) {
        let Some(dialog) = self.open.get_mut(&request) else {
            return;
        };
        let ctx = ui.ctx().clone();
        let answer = if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) { Some(DialogAnswer::Canceled) } else { dialog.form.draw(ui, request) };
        if let Some(answer) = answer {
            self.finish(request, answer);
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    fn finish(&mut self, request: RequestId, answer: DialogAnswer) {
        if let Some(dialog) = self.open.remove(&request) {
            dialog.drawn.give(Ok(answer));
        }
    }
}

impl Dialog {
    fn builder(&self) -> egui::ViewportBuilder {
        let title = match (self.drawn.title(), &self.form) {
            ("", Form::Message { .. }) => "PWRS-GUI",
            ("", Form::Files(b)) => b.choose.title(),
            (given, _) => given,
        };
        egui::ViewportBuilder::default().with_title(title).with_inner_size(self.size).with_resizable(true).with_active(self.takes_focus)
    }
}

/// What a drawn dialog shows.
enum Form {
    Message { text: String, icon: MessageIcon, buttons: MessageButtons },
    Files(Box<Browser>),
}

impl Form {
    fn of(kind: &DialogKind) -> Form {
        match kind {
            DialogKind::Message { text, icon, buttons } => Form::Message { text: text.clone(), icon: *icon, buttons: *buttons },
            DialogKind::Open { multiple, filters, directory } => Form::Files(Box::new(Browser::new(Choose::Open, *multiple, filters.clone(), directory.as_deref(), String::new()))),
            DialogKind::Save { filters, directory, file_name } => {
                let name = match file_name {
                    Some(name) => name.clone(),
                    None => String::new(),
                };
                Form::Files(Box::new(Browser::new(Choose::Save, false, filters.clone(), directory.as_deref(), name)))
            }
            DialogKind::Folder { multiple, directory } => Form::Files(Box::new(Browser::new(Choose::Folder, *multiple, Vec::new(), directory.as_deref(), String::new()))),
        }
    }

    /// The viewport's inside size in points as it opens: a message's fitted
    /// to its text ([`message_size`]), a file dialog's fixed.
    fn size(&self, ctx: &egui::Context) -> [f32; 2] {
        match self {
            Form::Message { text, .. } => message_size(ctx, text),
            Form::Files(_) => [640.0, 440.0],
        }
    }

    /// Draws the form, returning what a person chose once they have.
    fn draw(&mut self, ui: &mut egui::Ui, request: RequestId) -> Option<DialogAnswer> {
        match self {
            Form::Message { text, icon, buttons } => message(ui, request, text, *icon, *buttons),
            Form::Files(browser) => browser.draw(ui, request),
        }
    }
}

/// The side of a message's icon, in points.
const ICON: f32 = 32.0;

/// A message window's inside size, as a dialog fits its content: as wide as
/// its longest line beside its icon, from a dialog's least width to its
/// most, and tall enough for its text wrapped to that width, or its icon,
/// inside a dialog's margin, and its button bar below with the line over
/// it, at most a dialog's most height, past which the text scrolls.
fn message_size(ctx: &egui::Context, text: &str) -> [f32; 2] {
    let d = &tokens::DIALOG;
    let style = ctx.global_style();
    let font = egui::TextStyle::Body.resolve(&style);
    let beside = 2.0 * d.margin + ICON + style.spacing.item_spacing.x;
    let longest = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)).size().x;
    let width = (longest + beside).clamp(d.width[0], d.width[1]);
    let wrapped = ctx.fonts_mut(|f| f.layout(text.to_owned(), font, egui::Color32::WHITE, width - beside)).size().y;
    let bar = d.button[1] + 2.0 * d.margin + f32::from(look::bar_line(&style, button_bar::WINDOWS_ORDER));
    [width, (wrapped.max(ICON) + 2.0 * d.margin + bar).min(d.height_max)]
}

/// A message's role in the dialog's keys: OK and Yes are the default, which
/// Enter presses, Cancel the cancel button, and No the cancel button where
/// the dialog has no Cancel, as the systems' message boxes make them.
fn role(button: MessageButton, shown: &[MessageButton]) -> Role {
    match button {
        MessageButton::Ok | MessageButton::Yes => Role::Default,
        MessageButton::Cancel => Role::Cancel,
        MessageButton::No if !shown.contains(&MessageButton::Cancel) => Role::Cancel,
        MessageButton::No => Role::Other,
    }
}

/// A message's icon beside its text inside a dialog's margin, and its
/// buttons at the right of a button bar, as a dialog's button bar places
/// them: in the system's order, each a dialog's button size at least, the
/// default one in the selection's colors. Enter presses the default
/// button. Text taller than the window scrolls, so all of it can be read.
fn message(ui: &mut egui::Ui, request: RequestId, text: &str, icon: MessageIcon, buttons: MessageButtons) -> Option<DialogAnswer> {
    let mut pressed = None;
    let shown = buttons.buttons();
    let roles: Vec<Role> = shown.iter().map(|b| role(*b, shown)).collect();
    let strip = button_bar::WINDOWS_ORDER;
    egui::Panel::bottom(("pwrs-stand-in-message-buttons", request)).frame(look::button_bar_frame(ui.style(), strip)).show_separator_line(strip).show(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = tokens::DIALOG.button_gap;
            // Right to left, so the last in the system's order is placed
            // first, at the right.
            for &i in button_bar::order(&roles, strip).iter().rev() {
                let [width, height] = tokens::DIALOG.button;
                if ui.add(button_of(ui, label(shown[i]), roles[i], true).min_size(egui::vec2(width, height))).clicked() {
                    pressed = Some(DialogAnswer::Button(shown[i]));
                }
            }
        });
    });
    egui::CentralPanel::default().frame(look::dialog_frame(ui.style())).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                draw_icon(ui, icon);
                ui.add(egui::Label::new(text).wrap());
            });
        });
    });
    if pressed.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        pressed = roles.iter().position(|r| *r == Role::Default).map(|i| DialogAnswer::Button(shown[i]));
    }
    pressed
}

fn label(button: MessageButton) -> &'static str {
    match button {
        MessageButton::Ok => "OK",
        MessageButton::Cancel => "Cancel",
        MessageButton::Yes => "Yes",
        MessageButton::No => "No",
    }
}

/// A message's icon: a disc in the palette's color for the kind, with an i,
/// a question mark, an exclamation mark or a cross on it, named for a
/// screen reader.
fn draw_icon(ui: &mut egui::Ui, icon: MessageIcon) {
    let side = ICON;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let v = ui.visuals();
    let (fill, name) = match icon {
        MessageIcon::Info => (v.hyperlink_color, "Information"),
        MessageIcon::Question => (v.hyperlink_color, "Question"),
        MessageIcon::Warning => (v.warn_fg_color, "Warning"),
        MessageIcon::Error => (v.error_fg_color, "Error"),
    };
    let ink = v.panel_fill;
    let painter = ui.painter();
    let c = rect.center();
    painter.circle_filled(c, side / 2.0, fill);
    let stroke = egui::Stroke::new(3.0, ink);
    match icon {
        MessageIcon::Info => {
            painter.circle_filled(c + egui::vec2(0.0, -8.0), 2.5, ink);
            painter.line_segment([c + egui::vec2(0.0, -2.0), c + egui::vec2(0.0, 9.0)], stroke);
        }
        MessageIcon::Warning => {
            painter.line_segment([c + egui::vec2(0.0, -9.0), c + egui::vec2(0.0, 3.0)], stroke);
            painter.circle_filled(c + egui::vec2(0.0, 8.5), 2.0, ink);
        }
        MessageIcon::Error => {
            painter.line_segment([c + egui::vec2(-7.0, -7.0), c + egui::vec2(7.0, 7.0)], stroke);
            painter.line_segment([c + egui::vec2(7.0, -7.0), c + egui::vec2(-7.0, 7.0)], stroke);
        }
        MessageIcon::Question => {
            painter.text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(22.0), ink);
        }
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, name));
}

/// What a file dialog chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choose {
    Open,
    Save,
    Folder,
}

impl Choose {
    /// The window's title when the dialog has none.
    fn title(self) -> &'static str {
        match self {
            Choose::Open => "Open",
            Choose::Save => "Save",
            Choose::Folder => "Choose a Folder",
        }
    }
}

/// A folder's entry.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    name: String,
    folder: bool,
}

/// What a folder contains, as far as it could be read: its entries, the
/// names that are not Unicode, which a dialog's answer cannot contain, and
/// why it could not be read when it could not.
#[derive(Debug, Default, PartialEq, Eq)]
struct Listing {
    entries: Vec<Entry>,
    not_unicode: Vec<OsString>,
    failed: Option<String>,
}

/// The renderer's own file dialog: the folder shown, what it contains, the
/// chosen filter's files among them, and what a person has selected or
/// typed.
struct Browser {
    choose: Choose,
    multiple: bool,
    filters: Vec<(String, Vec<String>)>,
    filter: usize,
    folder: PathBuf,
    /// The folder field's text, which Enter goes to.
    typed: String,
    listing: Listing,
    /// Whether names starting with a dot are shown.
    hidden: bool,
    /// The names selected.
    selected: BTreeSet<String>,
    /// The row a Shift click selects from.
    anchor: Option<usize>,
    /// A file to save's name.
    name: String,
    /// What the last thing a person asked for could not do.
    problem: Option<String>,
}

impl Browser {
    fn new(choose: Choose, multiple: bool, filters: Vec<(String, Vec<String>)>, directory: Option<&str>, name: String) -> Browser {
        let (folder, problem) = start(directory);
        let mut b = Browser {
            choose,
            multiple,
            filters,
            filter: 0,
            typed: folder.display().to_string(),
            folder,
            listing: Listing::default(),
            hidden: false,
            selected: BTreeSet::new(),
            anchor: None,
            name,
            problem,
        };
        b.relist();
        b
    }

    fn relist(&mut self) {
        self.listing = list(&self.folder, self.hidden);
    }

    /// Goes to folder `to`, or says why it cannot.
    fn enter(&mut self, to: PathBuf) {
        if to.is_dir() {
            self.folder = to;
            self.selected.clear();
            self.anchor = None;
            self.problem = None;
            self.relist();
        } else {
            self.problem = Some(format!("{} is not a folder", to.display()));
        }
        self.typed = self.folder.display().to_string();
    }

    /// The entries shown: every folder, and the files the chosen filter
    /// takes, but none in a dialog for folders.
    fn shown(&self) -> Vec<Entry> {
        self.listing.entries.iter().filter(|e| e.folder || (self.choose != Choose::Folder && takes(self.filters.get(self.filter), &e.name))).cloned().collect()
    }

    /// Selects row `at` of `shown` as a click with `modifiers` pressed does:
    /// alone, or in a dialog of several, Ctrl adds or removes it and Shift
    /// selects the rows from the anchor's to it.
    fn select(&mut self, shown: &[Entry], at: usize, modifiers: egui::Modifiers) {
        let name = shown[at].name.clone();
        if self.multiple && modifiers.command {
            if !self.selected.remove(&name) {
                self.selected.insert(name);
            }
            self.anchor = Some(at);
        } else if self.multiple && modifiers.shift {
            let from = self.anchor.filter(|a| *a < shown.len()).unwrap_or(at);
            let (lo, hi) = if from <= at { (from, at) } else { (at, from) };
            self.selected = shown[lo..=hi].iter().map(|e| e.name.clone()).collect();
        } else {
            self.selected = BTreeSet::from([name]);
            self.anchor = Some(at);
        }
    }

    /// The paths of `names` in the folder shown.
    fn within(&self, names: &[String]) -> Vec<PathBuf> {
        names.iter().map(|n| self.folder.join(n)).collect()
    }

    /// The answer of `paths`, or none when one is not Unicode, which
    /// [`Browser::problem`] then says.
    fn answer(&mut self, paths: Vec<PathBuf>) -> Option<DialogAnswer> {
        let mut answered = Vec::new();
        for path in paths {
            match path.into_os_string().into_string() {
                Ok(path) => answered.push(path),
                Err(path) => {
                    self.problem = Some(format!("the path {} is not Unicode, which a dialog's answer must be", PathBuf::from(path).display()));
                    return None;
                }
            }
        }
        Some(DialogAnswer::Paths(answered))
    }

    /// What the dialog's main button does: open the files selected, or the
    /// folder when it is all that is selected; save to the name typed; or
    /// choose the folders selected, or the folder shown when none is.
    fn act(&mut self, shown: &[Entry]) -> Option<DialogAnswer> {
        let picked: Vec<&Entry> = shown.iter().filter(|e| self.selected.contains(&e.name)).collect();
        match self.choose {
            Choose::Open => match picked.as_slice() {
                [only] if only.folder => {
                    let to = self.folder.join(&only.name);
                    self.enter(to);
                    None
                }
                _ => {
                    let files: Vec<String> = picked.iter().filter(|e| !e.folder).map(|e| e.name.clone()).collect();
                    if files.is_empty() { None } else { self.answer(self.within(&files)) }
                }
            },
            Choose::Save => {
                let name = self.name.trim().to_string();
                if name.is_empty() { None } else { self.answer(self.within(&[name])) }
            }
            Choose::Folder => {
                let folders: Vec<String> = picked.iter().filter(|e| e.folder).map(|e| e.name.clone()).collect();
                if folders.is_empty() { self.answer(vec![self.folder.clone()]) } else { self.answer(self.within(&folders)) }
            }
        }
    }

    /// Draws the dialog: the folder's field above what it contains, and below
    /// them, in a button bar, the name to save under, the filters, and the
    /// buttons at its right in the system's order, the main one the default.
    fn draw(&mut self, ui: &mut egui::Ui, request: RequestId) -> Option<DialogAnswer> {
        let shown = self.shown();
        let mut go: Option<PathBuf> = None;
        let mut answer = None;
        egui::Panel::top(("pwrs-stand-in-folder", request)).frame(look::bar_frame(ui.style())).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(self.folder.parent().is_some(), egui::Button::new("Up")).clicked() {
                    go = self.folder.parent().map(Path::to_path_buf);
                }
                if let Some(home) = std::env::home_dir()
                    && ui.button("Home").clicked()
                {
                    go = Some(home);
                }
                let field = look::field(ui, |ui| ui.add(egui::TextEdit::singleline(&mut self.typed).desired_width(f32::INFINITY).margin(look::text_edit_margin())));
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = Some(PathBuf::from(&self.typed));
                }
            });
        });
        let strip = button_bar::WINDOWS_ORDER;
        egui::Panel::bottom(("pwrs-stand-in-buttons", request)).frame(look::button_bar_frame(ui.style(), strip)).show_separator_line(strip).show(ui, |ui| {
            if self.choose == Choose::Save {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    let field = look::field(ui, |ui| ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY).margin(look::text_edit_margin())));
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        answer = self.act(&shown);
                    }
                });
            }
            ui.horizontal(|ui| {
                if let Some((current, _)) = self.filters.get(self.filter) {
                    let current = current.clone();
                    egui::ComboBox::from_id_salt(("pwrs-stand-in-filter", request)).selected_text(current).show_ui(ui, |ui| {
                        for (i, (name, _)) in self.filters.iter().enumerate() {
                            if ui.selectable_label(i == self.filter, name.as_str()).clicked() {
                                self.filter = i;
                            }
                        }
                    });
                }
                if ui.checkbox(&mut self.hidden, "Hidden files").changed() {
                    self.relist();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = tokens::DIALOG.button_gap;
                    let (main, ready) = match self.choose {
                        Choose::Open => ("Open", shown.iter().any(|e| self.selected.contains(&e.name))),
                        Choose::Save => ("Save", !self.name.trim().is_empty()),
                        Choose::Folder => ("Choose", true),
                    };
                    let [width, height] = tokens::DIALOG.button;
                    let size = egui::vec2(width, height);
                    // The main button is the dialog's default and Cancel its
                    // cancel button, placed right to left, so the last in the
                    // system's order stands at the right.
                    let roles = [Role::Default, Role::Cancel];
                    for &i in button_bar::order(&roles, strip).iter().rev() {
                        if roles[i] == Role::Default {
                            if ui.add_enabled(ready, button_of(ui, main, Role::Default, ready).min_size(size)).clicked() {
                                answer = self.act(&shown);
                            }
                        } else if ui.add(button_of(ui, "Cancel", Role::Cancel, true).min_size(size)).clicked() {
                            answer = Some(DialogAnswer::Canceled);
                        }
                    }
                });
            });
        });
        egui::CentralPanel::default().frame(look::central_frame(ui.style())).show(ui, |ui| {
            let error = ui.visuals().error_fg_color;
            for said in [&self.problem, &self.listing.failed].into_iter().flatten() {
                ui.colored_label(error, said.as_str());
            }
            let modifiers = ui.input(|i| i.modifiers);
            egui::ScrollArea::vertical().id_salt(("pwrs-stand-in-entries", request)).auto_shrink([false, false]).show(ui, |ui| {
                for (at, entry) in shown.iter().enumerate() {
                    let text = if entry.folder { format!("{}{MAIN_SEPARATOR}", entry.name) } else { entry.name.clone() };
                    let r = ui.selectable_label(self.selected.contains(&entry.name), text);
                    if r.clicked() {
                        self.select(&shown, at, modifiers);
                        if self.choose == Choose::Save && !entry.folder {
                            self.name.clone_from(&entry.name);
                        }
                    }
                    if r.double_clicked() {
                        if entry.folder {
                            go = Some(self.folder.join(&entry.name));
                        } else if self.choose == Choose::Open {
                            answer = self.answer(vec![self.folder.join(&entry.name)]);
                        }
                    }
                }
                for name in &self.listing.not_unicode {
                    ui.weak(format!("{} (its name is not Unicode, which a dialog's answer must be)", Path::new(name).display()));
                }
            });
        });
        if let Some(to) = go {
            self.enter(to);
        }
        answer
    }
}

/// Where a file dialog starts: `directory` when it is a folder, else the
/// home folder, else the current one, else the top of the file system; and
/// why it does not start in `directory`.
fn start(directory: Option<&str>) -> (PathBuf, Option<String>) {
    let asked = directory.map(PathBuf::from);
    let refused = match &asked {
        Some(d) if d.is_dir() => return (d.clone(), None),
        Some(d) => Some(format!("{} is not a folder, so the dialog starts elsewhere", d.display())),
        None => None,
    };
    if let Some(home) = std::env::home_dir().filter(|h| h.is_dir()) {
        return (home, refused);
    }
    match std::env::current_dir() {
        Ok(here) => (here, refused),
        Err(e) => (PathBuf::from(MAIN_SEPARATOR_STR), Some(format!("neither the home folder nor the current one can be read ({e}), so the dialog starts at the top"))),
    }
}

/// What `folder` contains: folders first and each kind by name without regard
/// to case, names starting with a dot only when `hidden`, and the names
/// that are not Unicode apart; or why it cannot be read.
fn list(folder: &Path, hidden: bool) -> Listing {
    let mut listing = Listing::default();
    let read = match std::fs::read_dir(folder) {
        Ok(read) => read,
        Err(e) => {
            listing.failed = Some(format!("{} cannot be read: {e}", folder.display()));
            return listing;
        }
    };
    for item in read {
        let item = match item {
            Ok(item) => item,
            Err(e) => {
                listing.failed = Some(format!("{} could not be read to its end: {e}", folder.display()));
                break;
            }
        };
        match item.file_name().into_string() {
            Ok(name) if !hidden && name.starts_with('.') => {}
            Ok(name) => listing.entries.push(Entry { folder: item.path().is_dir(), name }),
            Err(raw) => listing.not_unicode.push(raw),
        }
    }
    listing.entries.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    listing
}

/// Whether `filter` takes the file `name`: every file without a filter or
/// for `*`, else a file whose name ends in one of its extensions, without
/// regard to case.
fn takes(filter: Option<&(String, Vec<String>)>, name: &str) -> bool {
    let Some((_, extensions)) = filter else {
        return true;
    };
    let lower = name.to_lowercase();
    extensions.iter().any(|x| x == "*" || lower.ends_with(&format!(".{}", x.to_lowercase())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, folder: bool) -> Entry {
        Entry { name: name.to_string(), folder }
    }

    #[test]
    fn a_filter_takes_its_extensions_without_regard_to_case_and_star_takes_every_file() {
        let logs = (String::from("Logs"), vec![String::from("log"), String::from("tar.gz")]);
        assert!(takes(Some(&logs), "app.LOG"));
        assert!(takes(Some(&logs), "old.tar.gz"));
        assert!(!takes(Some(&logs), "notes.txt"));
        assert!(!takes(Some(&logs), "log"), "a name that is only the extension has no dot before it");
        assert!(takes(Some(&(String::from("All"), vec![String::from("*")])), "notes.txt"));
        assert!(takes(None, "notes.txt"));
    }

    #[test]
    fn a_folder_lists_its_folders_first_and_hides_dotted_names_until_asked() {
        let dir = std::env::temp_dir().join(format!("pwrs-gui-stand-in-list-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Zeta")).expect("a folder is made");
        std::fs::create_dir_all(dir.join("alpha")).expect("a folder is made");
        for file in ["b.log", "A.txt", ".hidden"] {
            std::fs::write(dir.join(file), b"x").expect("a file is written");
        }
        let listing = list(&dir, false);
        assert_eq!(listing.failed, None);
        assert_eq!(listing.entries, vec![entry("alpha", true), entry("Zeta", true), entry("A.txt", false), entry("b.log", false)]);
        assert!(listing.not_unicode.is_empty());
        assert!(list(&dir, true).entries.contains(&entry(".hidden", false)));
        std::fs::remove_dir_all(&dir).expect("the test's folder is removed");
    }

    #[test]
    fn a_folder_that_cannot_be_read_says_so() {
        let missing = std::env::temp_dir().join(format!("pwrs-gui-stand-in-missing-{}", std::process::id()));
        let listing = list(&missing, false);
        let why = listing.failed.expect("a missing folder does not read");
        assert!(why.contains("cannot be read"), "{why}");
        assert!(listing.entries.is_empty());
        let (_, refused) = start(missing.to_str());
        let refused = refused.expect("a missing folder is refused");
        assert!(refused.contains("is not a folder"), "{refused}");
    }

    fn browser(choose: Choose, multiple: bool) -> Browser {
        Browser {
            choose,
            multiple,
            filters: Vec::new(),
            filter: 0,
            folder: PathBuf::from(MAIN_SEPARATOR_STR).join("data"),
            typed: String::new(),
            listing: Listing { entries: vec![entry("logs", true), entry("a.log", false), entry("b.log", false), entry("c.log", false)], ..Listing::default() },
            hidden: false,
            selected: BTreeSet::new(),
            anchor: None,
            name: String::new(),
            problem: None,
        }
    }

    fn in_data(names: &[&str]) -> DialogAnswer {
        let data = PathBuf::from(MAIN_SEPARATOR_STR).join("data");
        DialogAnswer::Paths(names.iter().map(|n| data.join(n).display().to_string()).collect())
    }

    #[test]
    fn ctrl_adds_and_shift_takes_a_range_only_in_a_dialog_of_several() {
        let mut many = browser(Choose::Open, true);
        let shown = many.shown();
        many.select(&shown, 1, egui::Modifiers::NONE);
        many.select(&shown, 3, egui::Modifiers::COMMAND);
        assert_eq!(many.selected, BTreeSet::from([String::from("a.log"), String::from("c.log")]));
        many.select(&shown, 1, egui::Modifiers::SHIFT);
        assert_eq!(many.selected, BTreeSet::from([String::from("a.log"), String::from("b.log"), String::from("c.log")]));
        let mut one = browser(Choose::Open, false);
        one.select(&shown, 1, egui::Modifiers::NONE);
        one.select(&shown, 2, egui::Modifiers::COMMAND);
        assert_eq!(one.selected, BTreeSet::from([String::from("b.log")]), "Ctrl does not add in a dialog of one");
    }

    #[test]
    fn open_answers_the_files_selected_in_the_folder_shown() {
        let mut b = browser(Choose::Open, true);
        let shown = b.shown();
        b.select(&shown, 2, egui::Modifiers::NONE);
        b.select(&shown, 1, egui::Modifiers::COMMAND);
        assert_eq!(b.act(&shown), Some(in_data(&["a.log", "b.log"])));
    }

    #[test]
    fn a_folder_dialog_shows_folders_alone_and_chooses_the_folder_shown_when_none_is_selected() {
        let mut b = browser(Choose::Folder, false);
        let shown = b.shown();
        assert_eq!(shown, vec![entry("logs", true)]);
        let data = PathBuf::from(MAIN_SEPARATOR_STR).join("data");
        assert_eq!(b.act(&shown), Some(DialogAnswer::Paths(vec![data.display().to_string()])), "the folder itself, with no separator after it");
        b.select(&shown, 0, egui::Modifiers::NONE);
        assert_eq!(b.act(&shown), Some(in_data(&["logs"])));
    }

    #[test]
    fn save_answers_the_name_typed_and_waits_while_there_is_none() {
        let mut b = browser(Choose::Save, false);
        assert_eq!(b.act(&[]), None);
        b.name = String::from(" report.csv ");
        assert_eq!(b.act(&[]), Some(in_data(&["report.csv"])));
    }

    #[test]
    fn a_message_fits_its_text_within_a_dialogs_least_and_most_sizes() {
        let ctx = egui::Context::default();
        look::apply(&ctx);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let d = &tokens::DIALOG;
        let short = message_size(&ctx, "Short.");
        assert_eq!(short[0], d.width[0], "a short message takes a dialog's least width");
        let line = f32::from(look::bar_line(&ctx.global_style(), button_bar::WINDOWS_ORDER));
        assert_eq!(short[1], ICON + 2.0 * d.margin + d.button[1] + 2.0 * d.margin + line, "and the height of its icon, its margins, its button bar and the line over it");
        let two = message_size(&ctx, "Short.\nShort.\nShort.");
        assert!(two[1] > short[1], "lines past the icon's height add theirs: {two:?}");
        let long = message_size(&ctx, &"word ".repeat(2000));
        assert_eq!(long, [d.width[1], d.height_max], "a long one takes a dialog's most size, past which it scrolls");
    }
}
