//! `Show-GuiProgress` and `Set-GuiProgress`: a window showing an activity's
//! progress, fed by the script or from a SubEtha ring.

use crate::window;
use pwrs::prelude::*;
use pwrs_gui_client::proto::{Fit, WindowOptions};

/// A progress window's width in points: Windows' long progress bar, 355
/// points, and the margins beside it.
const PROGRESS_WIDTH: f32 = 380.0;

/// The names of a progress window's widgets.
const ACTIVITY: &str = "Activity";
const PROGRESS: &str = "Progress";

/// Gives a progress window its State, read each time it is requested.
const WITH_STATE: &str = r#"
param($Window)
Add-Member -InputObject $Window -MemberType ScriptProperty -Name State -Value { [PwrsGui.ProgressWindow]::StateOf($this.Id) } -PassThru
"#;

/// A progress window Show-GuiProgress opened, which Set-GuiProgress takes.
/// Its State reads Open while the window is open and Closed once the person
/// or Set-GuiProgress -Completed has closed it.
#[psclass(name = "PwrsGui.ProgressWindow")]
#[derive(Clone, Default)]
pub struct GuiProgressWindow {
    /// The window's number in the session.
    pub id: u64,
    /// The window's title.
    pub title: String,
}

/// What a progress window's State calls.
#[psmethods]
impl GuiProgressWindow {
    /// Open while window `window` is open, Closed once it has closed.
    pub fn state_of(window: u64) -> PsResult<String> {
        Ok(String::from(if window::is_open(window) { "Open" } else { "Closed" }))
    }
}

/// Opens a window showing an activity's progress and writes it once it has
/// opened, for Set-GuiProgress to update: the activity as its main line,
/// and under it a progress bar, its status with how much is done at the
/// right over a thin bar filled to -PercentComplete, or fed by -Ring, a
/// SubEtha ring whose items are JSON objects of Value and Text as a
/// ProgressBar's -Ring reads them, the Text being the status. The window stays open until
/// Set-GuiProgress -Completed closes it or the person does; its State reads
/// Open until then and Closed after, and a closed window takes no update,
/// so the script runs on.
///
/// # Examples
/// $p = Show-GuiProgress 'Copying files' -Status 'Starting' -PercentComplete 0
/// Set-GuiProgress $p -Status 'Half way' -PercentComplete 50
/// Set-GuiProgress $p -Completed
#[cmdlet(verb = "Show", noun = "GuiProgress", output = ["PwrsGui.ProgressWindow"])]
#[derive(Default)]
pub struct ShowGuiProgress {
    /// What is in progress, the window's main line.
    #[param(mandatory, position = 0)]
    pub activity: String,
    /// Where the activity stands, shown over the bar with how much is done
    /// at its right.
    #[param(position = 1)]
    pub status: Option<String>,
    /// How much of the activity is done, from 0 to 100.
    #[param]
    pub percent_complete: Option<i32>,
    /// A SubEtha ring another process writes, which fills the bar, as a
    /// ProgressBar's -Ring takes it.
    #[param]
    pub ring: Option<PsObject>,
    /// The window's title; the activity when none is given.
    #[param]
    pub title: Option<String>,
}

impl Cmdlet for ShowGuiProgress {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut bar = vec![("Name", PROGRESS.into_ps()?), ("Maximum", 100_i32.into_ps()?)];
        if let Some(p) = self.percent_complete {
            bar.push(("Value", percent(p)?.into_ps()?));
        }
        if let Some(s) = &self.status {
            bar.push(("Text", s.clone().into_ps()?));
        }
        if let Some(r) = &self.ring {
            bar.push(("Ring", r.clone()));
        }
        let mut widgets = ps.invoke("PWRS-GUI\\New-GuiText", &[("Text", self.activity.clone().into_ps()?), ("Name", ACTIVITY.into_ps()?), ("Style", "Instruction".into_ps()?)])?;
        widgets.extend(ps.invoke("PWRS-GUI\\New-GuiProgressBar", &bar)?);
        let title = match &self.title {
            Some(t) => t.clone(),
            None => self.activity.clone(),
        };
        let options = WindowOptions {
            title,
            width: Some(PROGRESS_WIDTH),
            height: None,
            resizable: true,
            always_on_top: false,
            close_on_escape: false,
            dialog: true,
            fit: Fit::WINDOW,
            theme: crate::options::theme(),
        };
        let Some(opened) = window::open_widgets(ps, &widgets, options)? else {
            return Ok(());
        };
        let shown = GuiProgressWindow { id: opened.id, title: opened.title }.into_ps()?;
        for o in window::script(WITH_STATE)?.call(ps, &[shown])? {
            ps.write_object(&o)?;
        }
        Ok(())
    }
}

/// Updates a progress window Show-GuiProgress opened: its activity, its
/// status and how much is done; -Completed closes it and returns once it
/// has closed, so its State reads Closed from then on. A window that has
/// closed takes no update, and this then does nothing.
///
/// # Examples
/// Set-GuiProgress $p -Status "Copied $n of $total" -PercentComplete (100 * $n / $total)
/// $p | Set-GuiProgress -Completed
#[cmdlet(verb = "Set", noun = "GuiProgress")]
#[derive(Default)]
pub struct SetGuiProgress {
    /// The progress window Show-GuiProgress wrote, or its number.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub window: PsObject,
    /// Where the activity stands now.
    #[param(position = 1)]
    pub status: Option<String>,
    /// What is in progress now.
    #[param]
    pub activity: Option<String>,
    /// How much of the activity is done now, from 0 to 100.
    #[param]
    pub percent_complete: Option<i32>,
    /// Closes the window, returning once it has closed.
    #[param]
    pub completed: bool,
}

impl Cmdlet for SetGuiProgress {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window_number(&self.window)?;
        let value = match self.percent_complete {
            Some(p) => Some(percent(p)?),
            None => None,
        };
        if !window::is_open(id) {
            return Ok(());
        }
        if self.completed {
            window::close(id)?;
            return window::wait_closed(ps, id);
        }
        if let Some(a) = &self.activity {
            window::set_widget(ps, id, ACTIVITY, &table("Text", a.clone().into_ps()?)?)?;
        }
        if let Some(s) = &self.status {
            window::set_widget(ps, id, PROGRESS, &table("Text", s.clone().into_ps()?)?)?;
        }
        if let Some(v) = value {
            window::set_widget(ps, id, PROGRESS, &table("Value", v.into_ps()?)?)?;
        }
        Ok(())
    }
}

/// The window -Window names: a number as it is, else the object's Id, as
/// a progress window has it.
fn window_number(given: &PsObject) -> PsResult<u64> {
    let kind = given.type_name()?;
    if matches!(kind.as_str(), "System.Byte" | "System.SByte" | "System.Int16" | "System.UInt16" | "System.Int32" | "System.UInt32" | "System.Int64" | "System.UInt64") {
        return u64::from_ps(given);
    }
    let id = pwrs::object::property(given, "Id").map_err(|e| {
        let text = format!("-Window takes a progress window Show-GuiProgress wrote, or its number, and was given a {kind}: {}", e.message);
        PsError::new(ErrorCategory::InvalidArgument, "GuiNoWindow", text).terminating()
    })?;
    u64::from_ps(&id)
}

/// -PercentComplete as the bar's value, refused outside 0 to 100.
fn percent(given: i32) -> PsResult<f64> {
    if (0..=100).contains(&given) {
        Ok(f64::from(given))
    } else {
        let text = format!("-PercentComplete is {given}; it runs from 0 to 100");
        Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadPercent", text).terminating())
    }
}

/// A hashtable of one property, as Update-GuiWidget takes it.
fn table(name: &str, value: PsObject) -> PsResult<PsObject> {
    let t = PsHashtable::new()?;
    t.set(name, value)?;
    Ok(t.0)
}
