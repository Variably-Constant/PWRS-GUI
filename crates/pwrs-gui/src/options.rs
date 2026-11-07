//! Where windows are drawn, by which renderer, in which theme and fonts, on
//! which GPU, and where webviews store their data, as `Set-GuiOption` leaves
//! it. A change of mode, fonts, GPU or web data applies to the next renderer
//! the module starts; a change of renderer or theme, to the next window.

use crate::session::{arm, gui_error};
use arc_swap::ArcSwapOption;
use pwrs::prelude::*;
use pwrs_gui_client::{Mode, Renderer, WebViewData};
use std::path::PathBuf;
use std::sync::Arc;

/// Where PWRS-GUI draws windows.
#[psenum(name = "PwrsGui.Mode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiMode {
    /// The helper process; on Windows, the module itself when the helper
    /// cannot start.
    #[default]
    Auto,
    /// The helper process only.
    Helper,
    /// The module itself, on a thread of its own. Windows only.
    InProcess,
}

impl From<GuiMode> for Mode {
    fn from(m: GuiMode) -> Mode {
        match m {
            GuiMode::Auto => Mode::Auto,
            GuiMode::Helper => Mode::Helper,
            GuiMode::InProcess => Mode::InProcess,
        }
    }
}

/// Which renderer draws widget windows.
#[psenum(name = "PwrsGui.Renderer")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiRenderer {
    /// egui, on the GPU.
    #[default]
    Egui,
    /// A webview: WebView2 on Windows, WebKit on macOS, WebKitGTK on Linux
    /// and FreeBSD. A widget window falls back to egui when none can start.
    Html,
}

impl From<GuiRenderer> for Renderer {
    fn from(r: GuiRenderer) -> Renderer {
        match r {
            GuiRenderer::Egui => Renderer::Egui,
            GuiRenderer::Html => Renderer::Html,
        }
    }
}

/// The theme windows are drawn in.
#[psenum(name = "PwrsGui.Theme")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiTheme {
    /// The system's, light or dark, following it while a window is open.
    #[default]
    System,
    /// Light, whatever the system's.
    Light,
    /// Dark, whatever the system's.
    Dark,
}

impl From<GuiTheme> for pwrs_gui_client::proto::Theme {
    fn from(t: GuiTheme) -> pwrs_gui_client::proto::Theme {
        match t {
            GuiTheme::System => pwrs_gui_client::proto::Theme::System,
            GuiTheme::Light => pwrs_gui_client::proto::Theme::Light,
            GuiTheme::Dark => pwrs_gui_client::proto::Theme::Dark,
        }
    }
}

/// The theme the next window opens in, as `Set-GuiOption -Theme` set it.
pub fn theme() -> pwrs_gui_client::proto::Theme {
    stored().theme.into()
}

/// The fonts windows draw text in.
#[psenum(name = "PwrsGui.Font")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiFont {
    /// Inter, and JetBrains Mono NL for monospace text, which come with the
    /// module, so text looks the same on every system and in both
    /// renderers.
    #[default]
    Bundled,
    /// The system's own: its UI font, and the monospace font its browser
    /// draws with.
    System,
}

impl From<GuiFont> for pwrs_gui_client::proto::Fonts {
    fn from(f: GuiFont) -> pwrs_gui_client::proto::Fonts {
        match f {
            GuiFont::Bundled => pwrs_gui_client::proto::Fonts::Bundled,
            GuiFont::System => pwrs_gui_client::proto::Fonts::System,
        }
    }
}

/// Whether a process's webviews share their data with other processes'.
#[psenum(name = "PwrsGui.WebView")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiWebView {
    /// One data folder for every process of the user. On Windows one
    /// WebView2 browser process runs them all, so a crash of it ends
    /// every page at once.
    #[default]
    Shared,
    /// A data folder of each process's own, a data store on macOS 14 and
    /// later, and on Windows a WebView2 browser process of its own.
    PerProcess,
}

impl From<GuiWebView> for WebViewData {
    fn from(w: GuiWebView) -> WebViewData {
        match w {
            GuiWebView::Shared => WebViewData::Shared,
            GuiWebView::PerProcess => WebViewData::PerProcess,
        }
    }
}

/// The options a renderer starts with.
#[derive(Clone, Debug)]
pub struct Options {
    pub mode: GuiMode,
    pub renderer: GuiRenderer,
    pub font: GuiFont,
    pub adapter: Option<String>,
    pub low_power: bool,
    pub web_view: GuiWebView,
    pub web_view_data_folder: Option<PathBuf>,
}

/// What `Set-GuiOption` has set; a mode or web data choice it has not set
/// comes from `PWRS_GUI_MODE` or `PWRS_GUI_WEBVIEW`.
#[derive(Clone, Default)]
struct Stored {
    mode: Option<GuiMode>,
    renderer: GuiRenderer,
    theme: GuiTheme,
    font: GuiFont,
    adapter: Option<String>,
    low_power: bool,
    web_view: Option<GuiWebView>,
    web_view_data_folder: Option<PathBuf>,
}

static STORED: ArcSwapOption<Stored> = ArcSwapOption::const_empty();

fn stored() -> Stored {
    match STORED.load_full() {
        Some(s) => (*s).clone(),
        None => Stored::default(),
    }
}

/// Where environment variables are read from.
pub enum Env<'a, 'ps> {
    /// The PowerShell session's environment, which `$env:` changes. On
    /// Linux, FreeBSD and macOS those changes stay out of the process's own
    /// environment.
    Session(&'a Pipeline<'ps>),
    /// The process's own, for a renderer another module starts through the
    /// native entry, which has no session to read.
    Process,
}

impl Env<'_, '_> {
    /// The value of variable `name`; `None` when it is not set; or why it
    /// cannot be read.
    pub fn var(&self, name: &str) -> Result<Option<String>, String> {
        match self {
            Env::Session(ps) => {
                let value = ps.variable(&format!("env:{name}")).map_err(|e| e.message)?;
                if value.is_null() { Ok(None) } else { String::from_ps(&value).map(Some).map_err(|e| e.message) }
            }
            Env::Process => match std::env::var(name) {
                Ok(v) => Ok(Some(v)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(e) => Err(e.to_string()),
            },
        }
    }
}

/// The options as set, the mode falling back to `PWRS_GUI_MODE` and then
/// to `Auto`, and the web data choice to `PWRS_GUI_WEBVIEW` and then to
/// `Shared`, when `Set-GuiOption` has not set them; the variables are read
/// from `env`.
pub fn current(env: &Env<'_, '_>) -> PsResult<Options> {
    let s = stored();
    let mode = match s.mode {
        Some(m) => m,
        None => mode_from_env(env)?,
    };
    let web_view = match s.web_view {
        Some(w) => w,
        None => web_view_from_env(env)?,
    };
    Ok(Options { mode, renderer: s.renderer, font: s.font, adapter: s.adapter, low_power: s.low_power, web_view, web_view_data_folder: s.web_view_data_folder })
}

fn web_view_from_env(env: &Env<'_, '_>) -> PsResult<GuiWebView> {
    match env.var("PWRS_GUI_WEBVIEW") {
        Ok(None) => Ok(GuiWebView::Shared),
        Err(e) => Err(PsError::new(ErrorCategory::InvalidData, "GuiWebViewUnreadable", format!("PWRS_GUI_WEBVIEW cannot be read: {e}"))),
        Ok(Some(v)) => parse_web_view(&v).ok_or_else(|| {
            PsError::new(ErrorCategory::InvalidData, "GuiWebViewUnknown", format!("PWRS_GUI_WEBVIEW is '{v}'; it must be Shared or PerProcess"))
        }),
    }
}

fn parse_web_view(v: &str) -> Option<GuiWebView> {
    match v.trim().to_ascii_lowercase().as_str() {
        "shared" => Some(GuiWebView::Shared),
        "perprocess" => Some(GuiWebView::PerProcess),
        _ => None,
    }
}

fn mode_from_env(env: &Env<'_, '_>) -> PsResult<GuiMode> {
    match env.var("PWRS_GUI_MODE") {
        Ok(None) => Ok(GuiMode::Auto),
        Err(e) => Err(PsError::new(ErrorCategory::InvalidData, "GuiModeUnreadable", format!("PWRS_GUI_MODE cannot be read: {e}"))),
        Ok(Some(v)) => parse_mode(&v).ok_or_else(|| {
            PsError::new(ErrorCategory::InvalidData, "GuiModeUnknown", format!("PWRS_GUI_MODE is '{v}'; it must be Auto, Helper or InProcess"))
        }),
    }
}

fn parse_mode(v: &str) -> Option<GuiMode> {
    match v.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(GuiMode::Auto),
        "helper" => Some(GuiMode::Helper),
        "inprocess" => Some(GuiMode::InProcess),
        _ => None,
    }
}

/// The options PWRS-GUI starts its renderers with, and the renderer drawing
/// widget windows now.
#[psclass(name = "PwrsGui.Option")]
#[derive(Default, Clone)]
pub struct GuiOption {
    /// Where windows are drawn.
    pub mode: GuiMode,
    /// The renderer that draws widget windows.
    pub renderer: GuiRenderer,
    /// The theme windows open in: System, Light or Dark.
    pub theme: GuiTheme,
    /// The fonts the next renderer started draws text in: Bundled or
    /// System.
    pub font: GuiFont,
    /// Part of an adapter's name to prefer; null for the default choice.
    pub adapter: Option<String>,
    /// Whether the adapter drawing the least power is preferred.
    pub low_power: bool,
    /// Whether webviews share their data with other processes', Shared or
    /// PerProcess.
    pub web_view: GuiWebView,
    /// The shared web data folder in place of the default; null for the
    /// default. Used with Shared.
    pub web_view_data_folder: Option<String>,
    /// Where the running session of that renderer draws, Helper or
    /// InProcess; null when none is running.
    pub running: Option<String>,
    /// That session's backend: a GPU backend for egui, the webview's
    /// engine for HTML.
    pub backend: Option<String>,
    /// That session's adapter for egui, or its engine's version for HTML.
    pub adapter_in_use: Option<String>,
    /// What the running HTML renderer's pages draw GpuSurfaces with, WebGPU
    /// or WebGL2, as the last of them to settle it reported; null until a
    /// page has drawn one, and while no HTML renderer runs.
    pub web_gpu: Option<String>,
}

/// Sets where windows are drawn, which renderer draws widget windows, the
/// theme windows open in, the fonts they draw text in, which GPU egui draws
/// on, and where webviews store their data. A running renderer goes on with
/// its settings; the next one the module starts takes these, and the next
/// window takes the renderer and the theme.
///
/// # Examples
/// Set-GuiOption -Mode Helper
/// Set-GuiOption -Renderer Html
/// Set-GuiOption -Theme Light
/// Set-GuiOption -Font System
/// Set-GuiOption -Adapter 'Radeon' -LowPower:$false
/// Set-GuiOption -WebView PerProcess
/// Set-GuiOption -WebViewDataFolder 'D:\WebData'
#[cmdlet(verb = "Set", noun = "GuiOption")]
#[derive(Default)]
pub struct SetGuiOption {
    /// Where windows are drawn.
    #[param]
    pub mode: Option<GuiMode>,
    /// Which renderer draws widget windows: Egui, or Html, which falls back
    /// to egui when no webview engine can start.
    #[param]
    pub renderer: Option<GuiRenderer>,
    /// The theme windows open in: System, the system's light or dark as it
    /// is set; Light; or Dark.
    #[param]
    pub theme: Option<GuiTheme>,
    /// The fonts windows draw text in: Bundled, Inter and JetBrains Mono NL,
    /// which come with the module, the same on every system; or System, the
    /// system's own. Either way the system's fonts draw what the chosen
    /// ones have no letters for.
    #[param]
    pub font: Option<GuiFont>,
    /// Part of an adapter's name to prefer; an empty string restores the
    /// default choice.
    #[param]
    pub adapter: Option<String>,
    /// Prefer the adapter that draws the least power.
    #[param]
    pub low_power: Option<bool>,
    /// Whether webviews share their data with other processes': Shared,
    /// one folder for every process, or PerProcess, a folder for each.
    #[param]
    pub web_view: Option<GuiWebView>,
    /// The shared web data folder in place of the default, used with
    /// Shared; a relative path is taken from the current location, and an
    /// empty string restores the default.
    #[param]
    pub web_view_data_folder: Option<String>,
}

impl Cmdlet for SetGuiOption {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let folder = match self.web_view_data_folder.as_deref() {
            None => None,
            Some("") => Some(None),
            Some(path) => {
                let resolved = ps.resolve_path(path, true)?;
                match resolved.into_iter().next() {
                    Some(full) => Some(Some(PathBuf::from(full))),
                    None => return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoFolder", format!("{path} names no folder")).terminating()),
                }
            }
        };
        let (mode, renderer, theme, font, adapter, low_power, web_view) = (self.mode, self.renderer, self.theme, self.font, self.adapter.clone(), self.low_power, self.web_view);
        STORED.rcu(|current| {
            let mut next = match current {
                Some(s) => (**s).clone(),
                None => Stored::default(),
            };
            if let Some(m) = mode {
                next.mode = Some(m);
            }
            if let Some(r) = renderer {
                next.renderer = r;
            }
            if let Some(t) = theme {
                next.theme = t;
            }
            if let Some(f) = font {
                next.font = f;
            }
            if let Some(a) = &adapter {
                next.adapter = if a.is_empty() { None } else { Some(a.clone()) };
            }
            if let Some(l) = low_power {
                next.low_power = l;
            }
            if let Some(w) = web_view {
                next.web_view = Some(w);
            }
            if let Some(f) = &folder {
                next.web_view_data_folder = f.clone();
            }
            Some(Arc::new(next))
        });
        let now = current(&Env::Session(ps))?;
        if let (GuiWebView::PerProcess, Some(f)) = (now.web_view, &now.web_view_data_folder) {
            pwrs::warning!(ps, "the web data folder {} is used with -WebView Shared; with PerProcess each process has a folder of its own", f.display())?;
        }
        if !crate::session::running().is_empty() {
            pwrs::verbose!(ps, "a renderer is running; these options apply to the next one PWRS-GUI starts")?;
        }
        Ok(())
    }
}

/// Gets the options PWRS-GUI starts its renderer with, what the running
/// renderer uses, and what the running HTML renderer's pages draw
/// GpuSurfaces with.
///
/// # Examples
/// Get-GuiOption
#[cmdlet(verb = "Get", noun = "GuiOption", output = ["PwrsGui.Option"])]
#[derive(Default)]
pub struct GetGuiOption {}

impl Cmdlet for GetGuiOption {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let o = current(&Env::Session(ps))?;
        let mut out = GuiOption {
            mode: o.mode,
            renderer: o.renderer,
            theme: stored().theme,
            font: o.font,
            adapter: o.adapter,
            low_power: o.low_power,
            web_view: o.web_view,
            web_view_data_folder: o.web_view_data_folder.map(|f| f.display().to_string()),
            running: None,
            backend: None,
            adapter_in_use: None,
            web_gpu: None,
        };
        if let Some(s) = crate::session::running_of(o.renderer.into()) {
            out.running = Some(s.path().to_string());
            if let Some(h) = s.hello() {
                out.backend = Some(h.capabilities.backend.clone());
                out.adapter_in_use = Some(h.capabilities.adapter.clone());
            }
        }
        if let Some(web) = crate::session::running_of(Renderer::Html) {
            out.web_gpu = web.page_graphics(arm(ps)).map_err(gui_error)?.map(|g| g.name().to_string());
        }
        ps.write(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_without_regard_to_case() {
        assert_eq!(parse_mode(" InProcess "), Some(GuiMode::InProcess));
        assert_eq!(parse_mode("helper"), Some(GuiMode::Helper));
        assert_eq!(parse_mode("AUTO"), Some(GuiMode::Auto));
        assert_eq!(parse_mode("window"), None);
    }

    #[test]
    fn web_data_choices_parse_without_regard_to_case() {
        assert_eq!(parse_web_view(" PerProcess "), Some(GuiWebView::PerProcess));
        assert_eq!(parse_web_view("shared"), Some(GuiWebView::Shared));
        assert_eq!(parse_web_view("per-process"), None);
    }
}
