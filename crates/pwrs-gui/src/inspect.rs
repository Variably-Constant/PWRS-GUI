//! Commands that look at the session's windows and drive them without a
//! person: for test suites of users' own as well as this project's.

use crate::session::{self, arm, gui_error};
use crate::values::{from_node, from_value, ordered, to_value};
use pwrs::prelude::*;
use pwrs_gui_client::GuiError;
use pwrs_gui_client::proto::test_input::{STEP_TIME, Step};
use pwrs_gui_client::proto::{Bounds, EventKind, Node, NodeId, Shortcut, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// A window the session has open.
#[psclass(name = "PwrsGui.Window")]
#[derive(Clone, Default)]
pub struct GuiWindowInfo {
    /// The window's number in the session, which the other commands take
    /// as -Window.
    pub id: u64,
    /// The window's title.
    pub title: String,
    /// The names of its widgets, in tree order.
    pub names: Vec<String>,
    /// For a window of widgets Show-GuiWindow -NoWait opened with handlers,
    /// the engine event its handlers run on, which Unregister-Event
    /// removes; null otherwise.
    pub source_identifier: Option<String>,
}

/// The engine's wildcard options, for matching titles as PowerShell
/// matches names.
#[psenum(clr = "System.Management.Automation.WildcardOptions")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum WildcardOptions {
    #[default]
    None = 0,
    IgnoreCase = 2,
}

/// Gets the windows the sessions have open, in any runspace of this
/// process and in either renderer, which is how a test finds the window
/// another runspace's command is waiting on. A window is listed once it has
/// opened, with its first frame drawn. Writes nothing when no window has
/// been opened in this process.
///
/// # Examples
/// Get-GuiWindow
/// Get-GuiWindow -Title 'Choose*'
#[cmdlet(verb = "Get", noun = "GuiWindow", output = ["PwrsGui.Window"])]
#[derive(Default)]
pub struct GetGuiWindow {
    /// Only windows whose title matches this wildcard pattern, without
    /// regard to case.
    #[param(position = 0)]
    pub title: Option<String>,
}

impl Cmdlet for GetGuiWindow {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let sessions = session::running();
        if sessions.is_empty() {
            return Ok(());
        }
        let pattern = match &self.title {
            Some(t) => Some(PsType::from_name("System.Management.Automation.WildcardPattern").new(&[t.clone().into_ps()?, WildcardOptions::IgnoreCase.into_ps()?])?),
            None => None,
        };
        for session in sessions {
            for w in session.windows(arm(ps)).map_err(gui_error)? {
                if let Some(p) = &pattern
                    && !bool::from_ps(&p.call("IsMatch", &[w.title.clone().into_ps()?])?)?
                {
                    continue;
                }
                ps.write(GuiWindowInfo { id: w.window, title: w.title, names: w.names, source_identifier: None })?;
            }
        }
        Ok(())
    }
}

/// What happens to a widget.
#[psenum(name = "PwrsGui.EventKind")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiEventKind {
    /// A button is pressed.
    #[default]
    Click,
    /// An input's value changes.
    Change,
    /// Rows of a list, a table or a tree are selected; the value is the row
    /// number or numbers, from 0, a tree's rows as shown.
    Select,
    /// A row is double-clicked or chosen with Enter; the value is its
    /// number, from 0.
    Activate,
    /// Enter is pressed in a text field of one line; the value is its
    /// text.
    Submit,
    /// One of the window's -OnShortcut shortcuts is pressed; the value is
    /// the shortcut, as `Ctrl+S`.
    Shortcut,
    /// A tree's node whose children are still to come is opened; the value
    /// is the node's object, and -OnExpand's output becomes its children.
    Expand,
    /// Files or text are dropped on a widget; the value is the paths
    /// dropped, or the text.
    Drop,
    /// A key the widget does not use itself is pressed while it has the
    /// focus; the value is the key written as a shortcut, as `Ctrl+K`.
    Key,
    /// A pointer button is pressed over a GpuSurface or a Canvas; the value
    /// gives where, as X and Y in physical pixels from its top left, U and V
    /// as fractions of its width and height and the display's
    /// PixelsPerPoint, with the Button and the Shift, Ctrl and Alt keys down,
    /// and on a Canvas the Item there, the name of the topmost named item
    /// under the pointer or null.
    PointerDown,
    /// A button pressed over a GpuSurface is released, wherever the pointer
    /// is; the value is PointerDown's.
    PointerUp,
    /// The pointer moves over a GpuSurface, or anywhere while a button
    /// pressed over it stays down, once a frame at most; the value is
    /// PointerDown's, its Button the one down or null.
    PointerMove,
    /// The wheel turns over a GpuSurface; the value is PointerDown's, its
    /// Button null, with the Notches turned, positive away from the person.
    Wheel,
    /// The window closed; Show-GuiWindow -OnClose runs with it, the value
    /// being the window's result.
    Close,
    /// A canvas item is clicked a second time soon after the first; the
    /// value is PointerDown's, naming the Item.
    DoubleClick,
    /// The pointer comes over a canvas item, the topmost named one there;
    /// the value is PointerDown's, naming the Item.
    PointerEnter,
    /// The pointer leaves the canvas item it was over; the value is
    /// PointerDown's, naming the Item left.
    PointerLeave,
    /// A button pressed over a canvas item stays down while the pointer moves;
    /// the value is PointerDown's at the point pressed, naming the Item.
    DragStart,
    /// The pointer moves while a drag goes on, once a frame at most; the
    /// value is PointerDown's where the pointer is, naming the Item dragged.
    DragMove,
    /// A drag's button is released; the value is PointerDown's where it was
    /// released, naming the Item dragged.
    DragEnd,
    /// A file the window watches changed (Show-GuiWindow -WatchPath); the
    /// name is the file's full path, and the value its length in bytes, null
    /// while the file is gone.
    FileChange,
    /// A test's hold on a password field's peek button, through
    /// Invoke-GuiTestEvent alone: the value $true holds the button, as a
    /// person pressing it does, and $false lets it go. No widget raises it.
    Peek,
}

impl GuiEventKind {
    /// The event as a renderer raises it; `None` for `Close` and
    /// `FileChange`, which are the window's own and no widget's.
    pub fn wire(self) -> Option<EventKind> {
        match self {
            GuiEventKind::Click => Some(EventKind::Click),
            GuiEventKind::Change => Some(EventKind::Change),
            GuiEventKind::Select => Some(EventKind::Select),
            GuiEventKind::Activate => Some(EventKind::Activate),
            GuiEventKind::Submit => Some(EventKind::Submit),
            GuiEventKind::Shortcut => Some(EventKind::Shortcut),
            GuiEventKind::Expand => Some(EventKind::Expand),
            GuiEventKind::Drop => Some(EventKind::Drop),
            GuiEventKind::Key => Some(EventKind::Key),
            GuiEventKind::PointerDown => Some(EventKind::PointerDown),
            GuiEventKind::PointerUp => Some(EventKind::PointerUp),
            GuiEventKind::PointerMove => Some(EventKind::PointerMove),
            GuiEventKind::Wheel => Some(EventKind::Wheel),
            GuiEventKind::Close => None,
            GuiEventKind::DoubleClick => Some(EventKind::DoubleClick),
            GuiEventKind::PointerEnter => Some(EventKind::PointerEnter),
            GuiEventKind::PointerLeave => Some(EventKind::PointerLeave),
            GuiEventKind::DragStart => Some(EventKind::DragStart),
            GuiEventKind::DragMove => Some(EventKind::DragMove),
            GuiEventKind::DragEnd => Some(EventKind::DragEnd),
            GuiEventKind::Peek => Some(EventKind::Peek),
            GuiEventKind::FileChange => None,
        }
    }

    /// The event a renderer raised.
    pub fn of(kind: EventKind) -> GuiEventKind {
        match kind {
            EventKind::Click => GuiEventKind::Click,
            EventKind::Change => GuiEventKind::Change,
            EventKind::Select => GuiEventKind::Select,
            EventKind::Activate => GuiEventKind::Activate,
            EventKind::Submit => GuiEventKind::Submit,
            EventKind::Shortcut => GuiEventKind::Shortcut,
            EventKind::Expand => GuiEventKind::Expand,
            EventKind::Drop => GuiEventKind::Drop,
            EventKind::Key => GuiEventKind::Key,
            EventKind::PointerDown => GuiEventKind::PointerDown,
            EventKind::PointerUp => GuiEventKind::PointerUp,
            EventKind::PointerMove => GuiEventKind::PointerMove,
            EventKind::Wheel => GuiEventKind::Wheel,
            EventKind::DoubleClick => GuiEventKind::DoubleClick,
            EventKind::PointerEnter => GuiEventKind::PointerEnter,
            EventKind::PointerLeave => GuiEventKind::PointerLeave,
            EventKind::DragStart => GuiEventKind::DragStart,
            EventKind::DragMove => GuiEventKind::DragMove,
            EventKind::DragEnd => GuiEventKind::DragEnd,
            EventKind::Peek => GuiEventKind::Peek,
        }
    }
}

/// Acts on a widget as a person would. The command waiting on the window
/// reads the event as it reads a person's, so a test drives a dialog
/// another runspace is showing. A widget the window does not have, or an
/// event it does not raise, is an error naming the widgets it has.
///
/// # Examples
/// Invoke-GuiTestEvent -Window 1 -Name Choice -Event Select -Value 2
/// Get-GuiWindow -Title Choose | Invoke-GuiTestEvent -Name Ok -Event Click
/// Invoke-GuiTestEvent -Window 1 -Shortcut Ctrl+S
/// Invoke-GuiTestEvent -Window 1 -Drop @('C:\Logs\today.log')
/// Invoke-GuiTestEvent -Window 1 -Input @{ Press = 'Right'; X = 40; Y = 12 }, @{ Release = 'Right'; X = 40; Y = 12 }
/// Invoke-GuiTestEvent -Window 1 -Input @{ Key = 'Menu' } -Timeout 30
/// Invoke-GuiTestEvent -Dialog 4 -Value Yes
#[cmdlet(verb = "Invoke", noun = "GuiTestEvent", default_parameter_set = "Widget", output = ["System.Object"])]
#[derive(Default)]
pub struct InvokeGuiTestEvent {
    /// The window's number, as Get-GuiWindow writes it.
    #[param(mandatory, position = 0, value_from_pipeline_by_property_name, alias = ["Id"], set = ["Widget", "Shortcut", "Drop", "Input"])]
    pub window: u64,
    /// Answers the system's dialog of this number, as Get-GuiDialog writes
    /// it, as a person would: -Value is a message's button, Ok, Cancel, Yes
    /// or No, or a file or folder dialog's path or paths; without it the
    /// dialog is canceled.
    #[param(mandatory, set = "Dialog")]
    pub dialog: Option<u64>,
    /// The widget's name, compared without regard to case.
    #[param(mandatory, position = 1, set = "Widget")]
    pub name: String,
    /// What happens to the widget.
    #[param(mandatory, position = 2, set = "Widget")]
    pub event: GuiEventKind,
    /// The event's value: the row or rows to select, the row to activate,
    /// a tree's row to open with Expand, a tree's rows being those shown,
    /// from 0; an input's new value; a chart's point to click, a hashtable
    /// of its Series, X and Y; the paths or the text to drop; the key to
    /// press, as Ctrl+K; with Peek, $true to hold a password field's peek
    /// button and $false to let it go. With -Dialog, the dialog's answer.
    #[param(position = 3, set = ["Widget", "Dialog"])]
    pub value: Option<PsObject>,
    /// Presses a shortcut, as Ctrl+S, as a person would: the window's
    /// enabled menu item with that shortcut is clicked, or else the
    /// window's own shortcut of -OnShortcut raises Shortcut.
    #[param(mandatory, set = "Shortcut")]
    pub shortcut: Option<String>,
    /// Drops on the window where no widget takes drops, as a person would:
    /// a list of files' paths, or text; Show-GuiWindow -OnDrop raises Drop
    /// with them. On a page of your own the files land at the middle of its
    /// window, as files the page makes of the names the paths end in, and
    /// its ondrop handlers and -OnDrop are given the paths.
    #[param(mandatory, set = "Drop")]
    pub drop: Option<PsObject>,
    /// Gives the window a person's input, step by step, a frame apart,
    /// through its renderer's own input. Each step is a hashtable. Press or
    /// Release names a button, Left, Right or Middle, and Move = $true moves
    /// the pointer, each with X and Y in points from the top left of the
    /// window's inside, as Get-GuiSnapshot's Bounds give them. Key names a
    /// key written as a shortcut, as Shift+F10, or Menu; Wait gives a
    /// number of seconds. Any step but a wait takes Shift, Ctrl and Alt,
    /// held through it. The command returns once the frame after the last
    /// step has run. A step not taken and seen in its time (-Timeout) fails
    /// the command with GuiTimeout, naming the step. A page of your own
    /// takes the steps as a window of widgets does; a move or resize of its
    /// window that it starts under them, as a press on its own title bar
    /// does, is not made, and Get-GuiSnapshot's Moving names it.
    #[param(mandatory, set = "Input")]
    pub input: Option<PsObject>,
    /// With -Input, how many seconds each step has to be taken and seen
    /// once it begins, zero or more; 10 without it. A wait step has its own
    /// seconds besides.
    #[param(set = "Input")]
    pub timeout: Option<f64>,
    /// Writes the value the widget took.
    #[param]
    pub pass_thru: bool,
}

impl Cmdlet for InvokeGuiTestEvent {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(dialog) = self.dialog {
            return crate::dialogs::answer(ps, dialog, self.value.as_ref());
        }
        if let Some(pressed) = &self.shortcut {
            let written = Shortcut::parse(pressed).map_err(|e| PsError::new(ErrorCategory::InvalidArgument, "GuiBadShortcut", e).terminating())?.written();
            let session = session::of_window(self.window)?;
            let took = session.test_event(self.window, "", EventKind::Shortcut, Value::Str(written), arm(ps)).map_err(gui_error)?;
            if self.pass_thru {
                ps.write_object(&from_value(&took)?)?;
            }
            return Ok(());
        }
        if let Some(dropped) = &self.drop {
            let session = session::of_window(self.window)?;
            let took = session.test_event(self.window, "", EventKind::Drop, to_value(dropped)?, arm(ps)).map_err(gui_error)?;
            if self.pass_thru {
                ps.write_object(&from_value(&took)?)?;
            }
            return Ok(());
        }
        if let Some(given) = &self.input {
            let steps = input_steps(&to_value(given)?)?;
            let time = step_time(self.timeout)?;
            let session = session::of_window(self.window)?;
            return session.test_input(self.window, steps, time, arm(ps)).map_err(|e| match e {
                GuiError::Late(why) => PsError::new(ErrorCategory::OperationTimeout, "GuiTimeout", format!("{why}; -Timeout gives each step more time")),
                other => gui_error(other),
            });
        }
        let Some(kind) = self.event.wire() else {
            let text = match self.event {
                GuiEventKind::FileChange => "FileChange is the window's own event, which no widget raises; a write to a file Show-GuiWindow -WatchPath names raises it",
                _ => "Close is the window's own event, which no widget raises; Close-GuiWindow closes a window",
            };
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotAWidgetEvent", text).terminating());
        };
        let session = session::of_window(self.window)?;
        let value = match &self.value {
            Some(v) => to_value(v)?,
            None => Value::Null,
        };
        let took = session.test_event(self.window, &self.name, kind, value, arm(ps)).map_err(gui_error)?;
        if self.pass_thru {
            ps.write_object(&from_value(&took)?)?;
        }
        Ok(())
    }
}

/// The time each step of -Input has: -Timeout's seconds, zero or more, or
/// [`STEP_TIME`] without it.
fn step_time(given: Option<f64>) -> PsResult<Duration> {
    match given {
        None => Ok(STEP_TIME),
        Some(t) if t.is_finite() && t >= 0.0 => Duration::try_from_secs_f64(t).map_err(|e| PsError::new(ErrorCategory::InvalidArgument, "GuiBadTimeout", format!("-Timeout is {t} seconds, which no clock counts: {e}")).terminating()),
        Some(t) => Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadTimeout", format!("-Timeout is {t}; it must be zero or more seconds")).terminating()),
    }
}

/// The steps of input `given` describes, one hashtable or a list of them,
/// in order; a step that is not one is refused, naming its place from 1.
fn input_steps(given: &Value) -> PsResult<Vec<Step>> {
    let listed = match given {
        Value::List(items) => items.as_slice(),
        one => std::slice::from_ref(one),
    };
    if listed.is_empty() {
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadStep", String::from("-Input has no steps")).terminating());
    }
    listed
        .iter()
        .enumerate()
        .map(|(i, step)| Step::read(step).map_err(|why| PsError::new(ErrorCategory::InvalidArgument, "GuiBadStep", format!("-Input's step {}: {why}", i + 1)).terminating()))
        .collect()
}

/// A window's tree, the state of its inputs, the widget with the keyboard
/// focus, the text it last copied, and its pixels when they were requested.
#[psclass(name = "PwrsGui.Snapshot")]
pub struct GuiSnapshot {
    /// The window's number.
    pub window: u64,
    /// The window's tree: nested ordered dictionaries with Id, Kind, Name,
    /// Properties and Children, in that order.
    pub tree: PsObject,
    /// Each input widget's current value, under its name, or under its
    /// number for a widget with no name.
    pub state: PsHashtable,
    /// The width of Pixels; null without -Pixels.
    pub width: Option<u32>,
    /// The height of Pixels; null without -Pixels.
    pub height: Option<u32>,
    /// The rendered pixels, RGBA, rows top to bottom; null without -Pixels.
    pub pixels: Option<Vec<u8>>,
    /// Where each named widget was last drawn, under its name, in tree
    /// order: an ordered dictionary of X, Y, Width and Height, in points
    /// from the top left of the window's inside. A widget not drawn, as one
    /// on a tab not shown, is absent.
    pub bounds: PsObject,
    /// The engine of a window the HTML renderer draws: an ordered dictionary
    /// of its Name, Version, the Folder it was loaded from, and Settings,
    /// each setting of its chrome the web helper sets, as the engine reports
    /// it now; null in a window egui draws.
    pub engine: PsObject,
    /// The name of the widget with the keyboard focus, or its number for a
    /// widget with no name; null while no widget of the window has it.
    pub focused: Option<String>,
    /// The text the window last put on the clipboard, as a table's rows
    /// copied with Ctrl+C; null until it has put any there.
    pub copied: Option<String>,
    /// The move of its window a page of your own last started while
    /// Invoke-GuiTestEvent -Input gave it input: drag, or resize and the
    /// edge, as n or se. The system makes no such move under a test, since
    /// it would follow the person's own pointer. Null when the page started
    /// none under the last test's input.
    pub moving: Option<String>,
}

/// Gets a window's tree, the state of its inputs and the widget with the
/// keyboard focus and, with -Pixels, the pixels it last drew, of the whole
/// window or of the widget -Name names.
/// A minimized or covered window is not drawn and has no pixels to take,
/// which is an error, as are pixels no frame has taken 30 seconds after
/// they were requested.
///
/// # Examples
/// (Get-GuiSnapshot -Window 1).State.Choice
/// $shot = Get-GuiSnapshot -Window 1 -Name Ok -Pixels
#[cmdlet(verb = "Get", noun = "GuiSnapshot", output = ["PwrsGui.Snapshot"])]
#[derive(Default)]
pub struct GetGuiSnapshot {
    /// The window's number, as Get-GuiWindow writes it.
    #[param(mandatory, position = 0, value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: u64,
    /// Takes the window's pixels, or the widget's with -Name.
    #[param]
    pub pixels: bool,
    /// The widget whose pixels -Pixels takes, compared without regard to
    /// case; naming one takes its pixels.
    #[param]
    pub name: Option<String>,
}

impl Cmdlet for GetGuiSnapshot {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let session = session::of_window(self.window)?;
        let pixels = self.pixels || self.name.is_some();
        let shot = session.snapshot(self.window, self.name.clone(), pixels, arm(ps)).map_err(gui_error)?;
        let state = PsHashtable::new()?;
        for (id, v) in &shot.state {
            let key = match shot.tree.find(*id).and_then(|n| n.name.clone()) {
                Some(name) => name,
                None => id.to_string(),
            };
            state.set(&key, from_value(v)?)?;
        }
        let bounds = bounds_of(&shot.tree, &shot.bounds)?;
        let engine = match &shot.engine {
            Some(e) => ordered(vec![
                (String::from("Name"), e.name.clone().into_ps()?),
                (String::from("Version"), e.version.clone().into_ps()?),
                (String::from("Folder"), e.folder.clone().into_ps()?),
                (String::from("Settings"), ordered(e.settings.iter().map(|(k, v)| Ok((k.clone(), from_value(v)?))).collect::<PsResult<Vec<(String, PsObject)>>>()?)?),
            ])?,
            None => PsObject::null(),
        };
        let (width, height, rgba) = match shot.pixels {
            Some(image) => (Some(image.width), Some(image.height), Some(image.rgba)),
            None => (None, None, None),
        };
        let focused = shot.focused.map(|id| match shot.tree.find(id).and_then(|n| n.name.clone()) {
            Some(name) => name,
            None => id.to_string(),
        });
        ps.write(GuiSnapshot { window: self.window, tree: from_node(&shot.tree)?, state, width, height, pixels: rgba, bounds, engine, focused, copied: shot.copied.clone(), moving: shot.moving.clone() })
    }
}

/// Each named node's box in `bounds`, under its name in tree order, as an
/// ordered dictionary of X, Y, Width and Height.
fn bounds_of(tree: &Node, bounds: &BTreeMap<NodeId, Bounds>) -> PsResult<PsObject> {
    let mut entries = Vec::new();
    let mut stack = vec![tree];
    let mut order = Vec::new();
    while let Some(n) = stack.pop() {
        order.push(n);
        stack.extend(n.children.iter().rev());
    }
    for n in order {
        if let (Some(name), Some(b)) = (&n.name, bounds.get(&n.id)) {
            let at = ordered(vec![
                (String::from("X"), f64::from(b.x).into_ps()?),
                (String::from("Y"), f64::from(b.y).into_ps()?),
                (String::from("Width"), f64::from(b.width).into_ps()?),
                (String::from("Height"), f64::from(b.height).into_ps()?),
            ])?;
            entries.push((name.clone(), at));
        }
    }
    ordered(entries)
}

/// A GPU adapter the renderer's backends find.
#[psclass(name = "PwrsGui.GpuAdapter")]
#[derive(Clone, Default)]
pub struct GpuAdapter {
    /// The adapter's name, as its driver reports it.
    pub name: String,
    /// The backend that finds it: Dx12, Vulkan, Metal or Gl.
    pub backend: String,
    /// DiscreteGpu, IntegratedGpu, VirtualGpu, Cpu or Other.
    pub device_type: String,
    /// The driver's name.
    pub driver: String,
    /// What the driver says of itself, such as its version.
    pub driver_info: String,
    /// The PCI vendor id.
    pub vendor: u32,
    /// The PCI device id.
    pub device: u32,
    /// Whether the running renderer draws with this adapter through this
    /// backend.
    pub in_use: bool,
}

/// Gets the GPU adapters the renderer's backends find, starting the
/// renderer when none is running. Set-GuiOption -Adapter prefers one by
/// part of its name for the next renderer started.
///
/// # Examples
/// Get-GuiGpuAdapter
/// Get-GuiGpuAdapter | Where-Object InUse
#[cmdlet(verb = "Get", noun = "GuiGpuAdapter", output = ["PwrsGui.GpuAdapter"])]
#[derive(Default)]
pub struct GetGuiGpuAdapter {}

impl Cmdlet for GetGuiGpuAdapter {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let session = session::egui_session(ps)?;
        let used = session.hello().map(|h| (h.capabilities.backend.clone(), h.capabilities.adapter.clone()));
        for a in session.adapters(arm(ps)).map_err(gui_error)? {
            let in_use = used.as_ref().is_some_and(|(backend, name)| *backend == a.backend && *name == a.name);
            ps.write(GpuAdapter {
                name: a.name,
                backend: a.backend,
                device_type: a.device_type,
                driver: a.driver,
                driver_info: a.driver_info,
                vendor: a.vendor,
                device: a.device,
                in_use,
            })?;
        }
        Ok(())
    }
}
