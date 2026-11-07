//! Tray icons: New-GuiTrayIcon shows one in the system's tray through the
//! egui renderer, with a menu of MenuItem, Menu and Separator widgets;
//! Update-GuiTrayIcon changes its picture, its tooltip and its menu,
//! Remove-GuiTrayIcon takes it away, and Get-GuiTrayIcon lists those shown.
//! A tray icon's number is one of the numbers windows take, so its clicks
//! and menu choices queue for Wait-GuiEvent and run its handlers as a
//! -NoWait window's events do, Update-GuiWidget changes its menu's items,
//! and Invoke-GuiTestEvent -TrayIcon clicks it.

use crate::session::{self, Shown, arm, gui_error};
use crate::widgets::{self, picture, picture_file};
use crate::window::{self, GuiHandlerOutput, Handling, Open};
use pwrs::prelude::*;
use pwrs_gui_client::Renderer;
use pwrs_gui_client::proto::{EventKind, Kind, Node, NodeId, PatchOp, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// The widgets a tray icon's menu holds.
const MENU_KINDS: &[Kind] = &[Kind::MenuItem, Kind::Menu, Kind::Separator];

/// A tray icon the egui renderer shows.
#[psclass(name = "PwrsGui.TrayIcon")]
#[derive(Clone, Default)]
pub struct GuiTrayIcon {
    /// Its number, one of the numbers windows take, which Wait-GuiEvent and
    /// Update-GuiWidget take as -Window, and Update-GuiTrayIcon,
    /// Remove-GuiTrayIcon and Invoke-GuiTestEvent as -TrayIcon.
    pub id: u64,
    /// The text shown while the pointer rests on it; null for none.
    pub tooltip: Option<String>,
    /// The names of its menu's items and menus, in menu order.
    pub names: Vec<String>,
    /// The engine event its handlers run on, which Unregister-Event
    /// removes; null in a listing.
    pub source_identifier: Option<String>,
}

/// A tray icon's picture: the bytes given, or the file a path names,
/// resolved from the current location, in any format Image shows.
fn icon_of(ps: &Pipeline<'_>, given: &PsObject) -> PsResult<Value> {
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadIcon", text).terminating();
    let bytes = match given.type_name()?.as_str() {
        "System.Byte[]" | "System.Object[]" => Vec::<u8>::from_ps(given).map_err(|e| refuse(format!("-Icon's bytes cannot be read: {}", e.message)))?,
        _path => {
            let path = String::from_ps(given)?;
            picture_file(ps, &path).map_err(|e| PsError::new(ErrorCategory::ObjectNotFound, "GuiNoFile", format!("-Icon: {e}")).terminating())?
        }
    };
    picture(bytes).map_err(|e| refuse(format!("-Icon: {e}")))
}

/// The tree of a tray icon: `root`, with the menu a -Menu block writes as
/// its children, numbered from `first`.
fn with_menu(ps: &Pipeline<'_>, root: Node, menu: Option<&PsScriptBlock>, first: NodeId) -> PsResult<widgets::Built> {
    let entries = match menu {
        Some(block) => window::block_widgets(ps, block, "-Menu")?,
        None => Vec::new(),
    };
    let built = widgets::build_under(ps, root, &entries, MENU_KINDS, "a tray icon's -Menu", first)?;
    window::names_once(&built.tree, &mut BTreeSet::new())?;
    Ok(built)
}

/// Shows an icon in the system's tray, with a tooltip and a menu, and
/// returns it once it shows. On Windows it is in the notification area,
/// on macOS in the menu bar, and on Linux and FreeBSD a StatusNotifierItem,
/// which KDE's panel shows, and GNOME with its AppIndicator extension. On
/// a desktop with no such tray, a warning says so, and the icon shows once
/// one starts.
///
/// -Menu writes the menu's MenuItem, Menu and Separator widgets, and an
/// item's -OnClick runs when the item is chosen. A left click on the icon
/// runs -OnClick, or without it opens the menu, as most macOS and GNOME
/// tray icons do. A right click opens the menu, and on Windows a double
/// click runs -OnDoubleClick. Each handler runs as an engine-event action,
/// as a -NoWait window's handlers do, and every click and choice also
/// waits for Wait-GuiEvent, the icon's number as -Window. The icon stays
/// until Remove-GuiTrayIcon takes it away or the module is removed from
/// every runspace.
///
/// # Examples
/// $tray = New-GuiTrayIcon -Icon .\backup.png -Tooltip 'Backups' -Menu { MenuItem 'Run now' -Name Run; Separator; MenuItem 'Quit' -Name Quit }
/// New-GuiTrayIcon .\backup.png 'Backups' -OnClick { Write-Host 'Backups are running' }
#[cmdlet(verb = "New", noun = "GuiTrayIcon", output = ["PwrsGui.TrayIcon"])]
#[derive(Default)]
pub struct NewGuiTrayIcon {
    /// The icon's picture: a path from the current location, or bytes, in
    /// any format Image shows.
    #[param(mandatory, position = 0)]
    pub icon: PsObject,
    /// The text shown while the pointer rests on the icon.
    #[param(position = 1)]
    pub tooltip: Option<String>,
    /// The block that writes the icon's menu: MenuItem, Menu and Separator
    /// widgets, each widget's short name standing for its New-Gui<Widget>
    /// command.
    #[param]
    pub menu: Option<PsScriptBlock>,
    /// Runs when the icon is clicked with the left button, with the event
    /// as its argument; without it, a left click opens the menu.
    #[param]
    pub on_click: Option<PsScriptBlock>,
    /// Runs when the icon is double-clicked with the left button, on
    /// Windows, with the event as its argument.
    #[param]
    pub on_double_click: Option<PsScriptBlock>,
    /// What happens to what the handlers write: Discard drops it, Pipeline
    /// writes it into the handler's event job, Console sends it to
    /// Out-Default. Discard when not given.
    #[param]
    pub handler_output: GuiHandlerOutput,
}

impl Cmdlet for NewGuiTrayIcon {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut root = Node::new(1, Kind::TrayIcon).prop("icon", icon_of(ps, &self.icon)?).prop("menuOnClick", Value::Bool(self.on_click.is_none()));
        if let Some(t) = &self.tooltip {
            root = root.prop("tooltip", Value::Str(t.clone()));
        }
        let mut built = with_menu(ps, root, self.menu.as_ref(), 2)?;
        for (kind, handler) in [(EventKind::Click, &self.on_click), (EventKind::DoubleClick, &self.on_double_click)] {
            if let Some(h) = handler {
                built.handlers.insert((1, kind), PsScriptBlock(h.0.clone()));
            }
        }
        let session = session::egui_session(ps)?;
        let mut tray = session.open_tray(built.tree.clone()).map_err(gui_error)?;
        let Some(before) = session::first_shown(ps, &mut tray, Shown::Tray)? else {
            return Ok(());
        };
        let handling = Handling { on_close: None, output: self.handler_output, files: Vec::new(), on_file_change: None, subscribe: true };
        let tooltip = self.tooltip.clone();
        let info = window::read_without_waiting(ps, session, tray, before, built, String::new(), HashMap::new(), handling)?;
        ps.write(GuiTrayIcon { id: info.id, tooltip, names: info.names, source_identifier: info.source_identifier })
    }
}

/// The tray icon numbered `id`, as the module stores it, while it shows.
fn shown_tray(id: u64) -> Option<Arc<Open>> {
    window::open_of_kind(id, Kind::TrayIcon).filter(|_| window::is_open(id))
}

/// The error for a number no tray icon shown has; non-terminating, for a
/// command given several.
fn no_tray(id: u64) -> PsError {
    PsError::new(ErrorCategory::ObjectNotFound, "GuiNoTrayIcon", format!("no tray icon is shown with the number {id}"))
}

/// Changes a tray icon's picture, its tooltip or its menu. A menu given
/// anew replaces the whole menu, its items' -OnClick handlers with it;
/// Update-GuiWidget changes one item's text or whether it is enabled.
///
/// # Examples
/// Update-GuiTrayIcon -TrayIcon $tray.Id -Tooltip 'Backups: running'
/// $tray | Update-GuiTrayIcon -Icon .\busy.png -Menu { MenuItem 'Stop' -Name Stop }
#[cmdlet(verb = "Update", noun = "GuiTrayIcon")]
#[derive(Default)]
pub struct UpdateGuiTrayIcon {
    /// The tray icon's number, as New-GuiTrayIcon writes it.
    #[param(mandatory, position = 0, value_from_pipeline_by_property_name, alias = ["Id"])]
    pub tray_icon: u64,
    /// The icon's new picture: a path from the current location, or bytes.
    #[param]
    pub icon: Option<PsObject>,
    /// The icon's new tooltip; an empty string shows none.
    #[param]
    pub tooltip: Option<String>,
    /// The block that writes the icon's new menu, as New-GuiTrayIcon -Menu
    /// does.
    #[param]
    pub menu: Option<PsScriptBlock>,
}

impl Cmdlet for UpdateGuiTrayIcon {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = self.tray_icon;
        let Some(open) = shown_tray(id) else {
            return Err(no_tray(id).terminating());
        };
        let mut root = (*open.tree()).clone();
        if let Some(i) = &self.icon {
            root.props.insert(String::from("icon"), icon_of(ps, i)?);
        }
        match self.tooltip.as_deref() {
            Some("") => {
                root.props.remove("tooltip");
            }
            Some(t) => {
                root.props.insert(String::from("tooltip"), Value::Str(t.to_string()));
            }
            None => {}
        }
        let (tree, handlers) = match &self.menu {
            Some(block) => {
                let built = with_menu(ps, root, Some(block), open.next_number())?;
                open.numbers_used_to(built.next);
                (built.tree, built.handlers.into_iter().map(|(k, h)| (k, h.0)).collect())
            }
            None => (root, HashMap::new()),
        };
        open.replace_tree(tree.clone(), handlers);
        session::of_window(id)?.patch(id, vec![PatchOp::Replace { node: tree }]).map_err(gui_error)
    }
}

/// Takes a tray icon away, and returns once it has gone. Its last event,
/// Close, waits for Wait-GuiEvent, as a window's does. A number no tray
/// icon shown has is a non-terminating GuiNoTrayIcon error.
///
/// # Examples
/// Remove-GuiTrayIcon -TrayIcon $tray.Id
/// Get-GuiTrayIcon | Remove-GuiTrayIcon
#[cmdlet(verb = "Remove", noun = "GuiTrayIcon")]
#[derive(Default)]
pub struct RemoveGuiTrayIcon {
    /// The tray icon's number, as New-GuiTrayIcon writes it.
    #[param(mandatory, position = 0, value_from_pipeline_by_property_name, alias = ["Id"])]
    pub tray_icon: u64,
}

impl Cmdlet for RemoveGuiTrayIcon {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = self.tray_icon;
        if shown_tray(id).is_none() {
            return Err(no_tray(id));
        }
        window::close(id)?;
        window::wait_closed(ps, id)
    }
}

/// Gets the tray icons the egui renderer shows, in any runspace of this
/// process. Writes nothing when it shows none or is not running.
///
/// # Examples
/// Get-GuiTrayIcon
/// Get-GuiTrayIcon | Remove-GuiTrayIcon
#[cmdlet(verb = "Get", noun = "GuiTrayIcon", output = ["PwrsGui.TrayIcon"])]
#[derive(Default)]
pub struct GetGuiTrayIcon {}

impl Cmdlet for GetGuiTrayIcon {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(live) = session::running_of(Renderer::Egui) else {
            return Ok(());
        };
        for w in live.windows(arm(ps)).map_err(gui_error)? {
            if w.kind != Kind::TrayIcon {
                continue;
            }
            let tooltip = if w.title.is_empty() { None } else { Some(w.title) };
            ps.write(GuiTrayIcon { id: w.window, tooltip, names: w.names, source_identifier: None })?;
        }
        Ok(())
    }
}
