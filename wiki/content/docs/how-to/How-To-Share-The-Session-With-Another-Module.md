---
title: How To Share The Session With Another Module
weight: 11
---

Draw a module's own windows through PWRS-GUI's session, so a process runs one renderer whichever modules show windows. Source: `crates/pwrs-gui-standin/src/lib.rs`, the smallest whole module that does this, which PWRS-GUI's tests build; `NativeEntry` in `crates/pwrs-gui-client/src/entry.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and window numbers. The clicks were sent by `Invoke-GuiTestEvent`, which drives another module's windows as it drives PWRS-GUI's.

## What a module gets

A PWRS module written in Rust takes one of PWRS-GUI's sessions through its native entry, naming the renderer it wants, egui or HTML, and gets an ordinary `pwrs_gui_client::Session`: it opens windows from trees it builds, waits on their events with Ctrl+C ending the wait, and reads and drives them. Its windows are drawn by the renderer PWRS-GUI started, or by one started as PWRS-GUI's options say when none runs, and appear in `Get-GuiWindow` beside PWRS-GUI's.

## Depend on the client, without a renderer

`pwrs-gui-client` is not published; depend on it from a checkout. Leave out its default `in-process` feature, which links the renderer itself:

```toml
[dependencies]
pwrs-gui-client = { path = "../PWRS-GUI/crates/pwrs-gui-client", default-features = false }
```

## Take the session in a command

Get the entry by running `Get-GuiNativeEntry`, pass its result to `NativeEntry::from_addresses`, and take the session of a renderer with an arm that ends the wait on the pipeline's stop. This is the stand-in's `take`, which takes the egui session; `Renderer::Html` takes the HTML one:

```rust
use pwrs::prelude::*;
use pwrs_gui_client::proto::{EventKind, Kind, Node, Value, WindowOptions};
use pwrs_gui_client::{Canceler, GuiError, NativeEntry, Renderer, Session, WindowEvent};

fn gui_error(e: GuiError) -> PsError {
    PsError::new(ErrorCategory::ResourceUnavailable, "StandinGui", e.to_string()).terminating()
}

/// One property of the entry Get-GuiNativeEntry wrote.
fn field(entry: &PsObject, name: &str) -> PsResult<u64> {
    u64::from_ps(&entry.get(name)?)
}

/// PWRS-GUI's session, taken through the native entry of the PWRS-GUI
/// module imported in this runspace, and started when none is running;
/// Ctrl+C ends a wait for it.
fn take(ps: &Pipeline<'_>) -> PsResult<Session> {
    let answers = ps.invoke("Get-GuiNativeEntry", &[])?;
    let Some(entry) = answers.first() else {
        return Err(PsError::new(ErrorCategory::ObjectNotFound, "StandinNoEntry", "Get-GuiNativeEntry wrote nothing").terminating());
    };
    let narrow = |name: &str| -> PsResult<u32> {
        let n = field(entry, name)?;
        u32::try_from(n).map_err(|e| PsError::new(ErrorCategory::InvalidData, "StandinEntry", format!("{name} is {n}, which is not a 32-bit number: {e}")).terminating())
    };
    // SAFETY: the addresses are what PWRS-GUI's Get-GuiNativeEntry returned
    // in this process, and PWRS never unloads the library they are in.
    let native = unsafe {
        NativeEntry::from_addresses(narrow("AbiVersion")?, narrow("Protocol")?, field(entry, "StartV1")?, field(entry, "WatchV1")?, field(entry, "UnwatchV1")?, field(entry, "SendV1")?)
    }
    .map_err(gui_error)?;
    native.session(Renderer::Egui, |c: Canceler| ps.on_stop(move || c.cancel()).map_err(|e| GuiError::Arm(e.message))).map_err(gui_error)
}
```

`ps.invoke` runs `Get-GuiNativeEntry` by name in the caller's runspace, so PWRS-GUI must be importable there; PowerShell imports it on that first use when it is installed.

## Open a window and wait

The stand-in's `Read-StandinChoice` builds a list and an OK button, opens them through the session, and waits on the window's events, registering the wait's canceler with the pipeline's stop so Ctrl+C ends it. Abridged here; the whole command is in the source:

```rust
let session = take(ps)?;
let items = Value::List(self.choice.iter().cloned().map(Value::Str).collect());
let tree = Node::new(1, Kind::Column)
    .child(Node::new(2, Kind::ListBox).named("Choice").prop("items", items))
    .child(Node::new(3, Kind::Button).named("Ok").prop("text", Value::Str(String::from("OK"))));
let options = WindowOptions { close_on_escape: true, ..WindowOptions::titled("PWRS-GUI stand-in") };
let mut window = session.open_window(options, tree).map_err(gui_error)?;
loop {
    let canceler = window.canceler();
    let waker = ps.on_stop(move || canceler.cancel())?;
    let event = window.next_event();
    drop(waker);
    match event {
        // a selection, OK, a close, or the stop's Canceled
        ...
    }
}
```

## One renderer for both

The stand-in's dialog and PWRS-GUI's own, open at once from two runspaces, in one process with one helper:

<!-- varies: window numbers -->
```powershell
Import-Module PwrsGuiStandin
$theirs = [powershell]::Create().AddScript({ Read-StandinChoice -Choice 'red', 'green' })
$ours = [powershell]::Create().AddScript({ Read-GuiChoice -Choice 'small', 'large' -Title 'Size' })
$theirsRunning = $theirs.BeginInvoke()
$oursRunning = $ours.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (@(Get-GuiWindow).Count -lt 2 -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
Get-GuiWindow | Sort-Object Title | Format-Table Id, Title, Names
@(Get-CimInstance Win32_Process -Filter "Name = 'pwrs-gui-egui.exe' AND ParentProcessId = $PID").Count
```

```text
           Id Title             Names
           -- -----             -----
1099511627776 PWRS-GUI stand-in {Choice, Ok}
            1 Size              {Choice, Ok, Cancel}

1
```

The stand-in's window numbers start at the base its take was given, so they never meet PWRS-GUI's. Both are driven the same way, and each command writes its own result. Read-GuiChoice enables its OK once it has read the selection, after `Invoke-GuiTestEvent` returns, so the script waits for that before pressing it:

```powershell
$stand = Get-GuiWindow -Title 'PWRS-GUI stand-in'
Invoke-GuiTestEvent -Window $stand.Id -Name Choice -Event Select -Value 1
Invoke-GuiTestEvent -Window $stand.Id -Name Ok -Event Click
$size = Get-GuiWindow -Title 'Size'
Invoke-GuiTestEvent -Window $size.Id -Name Choice -Event Select -Value 0
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (-not (Get-GuiSnapshot -Window $size.Id).Tree.Children[1].Children[0].Properties.enabled -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
Invoke-GuiTestEvent -Window $size.Id -Name Ok -Event Click
$theirs.EndInvoke($theirsRunning)
$ours.EndInvoke($oursRunning)
$theirs.Dispose()
$ours.Dispose()
```

```text
green
small
```

[The Native Entry's Functions](Native-Entry-Functions.md) gives the contract under `NativeEntry`, for checking what it does or taking the session from another language.
