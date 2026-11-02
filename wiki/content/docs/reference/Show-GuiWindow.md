---
title: Show-GuiWindow
weight: 3
---

Shows a window: one described in PowerShell, whose widgets a block writes, or one whose whole content is a page of your own, HTML given as text or a file or folder. Source: `crates/pwrs-gui/src/page.rs`, `ShowGuiWindow`; a window of widgets is `show` in `crates/pwrs-gui/src/window.rs`, its widgets `crates/pwrs-gui/src/widgets.rs`; the page's `window.pwrs` is `crates/pwrs-gui-web/src/runtime/bridge.js`, and its files come from `serve_page` in `crates/pwrs-gui-web/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. A window of widgets is driven by `Invoke-GuiTestEvent`, as a person would drive it, and a page closes itself or closes when `Send-GuiMessage` sends it a value.

## Syntax

```powershell
Get-Command Show-GuiWindow -Syntax
```

```text
Show-GuiWindow [-Content] <scriptblock> [-Title <string>] [-Width <double>] [-Height <double>] [-Renderer <Renderer>] [-HandlerOutput <HandlerOutput>] [-OnClose <scriptblock>] [-OnShortcut <Object>] [-OnDrop <scriptblock>] [-WatchPath <string[]>] [-WatchInterval <Object>] [-OnFileChange <scriptblock>] [-NoWait] [<CommonParameters>]

Show-GuiWindow [-Html] <string> [-Title <string>] [-Width <double>] [-Height <double>] [-OnMessage <scriptblock>] [-AllowNavigation] [-DevTools] [-NoWait] [<CommonParameters>]

Show-GuiWindow -Path <string> [-Title <string>] [-Width <double>] [-Height <double>] [-OnMessage <scriptblock>] [-AllowNavigation] [-DevTools] [-NoWait] [<CommonParameters>]
```

## Description

`Show-GuiWindow` opens a window in the helper, or in-process as [Set-GuiOption](Set-GuiOption.md) `-Mode` says. It shows one of two things.

### A window described in PowerShell

The `-Content` block writes its widgets, one under another, as a [Column](New-GuiColumn.md) lays them out: several widgets are the children of a Column at the window's root, and a block that writes one widget makes that widget the root, as [Get-GuiSnapshot](Get-GuiSnapshot.md)'s `Tree` shows. While the block runs, each widget's short name, such as `Column`, `Text`, `Button` and `ListBox`, is an alias to its `New-Gui<Widget>` command, in a scope that ends with the block, so nothing outside it is shadowed. An alias is found before a function or a cmdlet, so `Group` is the widget there, and Group-Object is spelled out. The window is drawn by egui, or by the HTML renderer with `-Renderer Html`; a window meant for HTML is drawn by egui when no webview engine can start, and the Verbose stream says why. In HTML the window looks as egui draws it, with every control and popup drawn by the page and none by the engine. Both use the same palette, spacing and typeface: the fonts the module brings, Inter for text and JetBrains Mono NL for monospace, or with [Set-GuiOption](Set-GuiOption.md) `-Font System` the system's own.

Without `-Width` or `-Height`, the window fits its widgets on that side: at least 320 points across, and at most nine tenths of the screen's work area across and eight tenths down, past which its lists and tables scroll. Where the work area cannot be read, it is taken as 1024 by 768 points. A list, table or tree takes the height its column leaves, so it scrolls in its place rather than push the widgets after it out of sight. The window stays resizable. A [ButtonBar](New-GuiButtonBar.md) puts the window's buttons along its bottom in the system's order, and inside it Enter presses the `-Default` button and Escape the `-Cancel` button.

Each widget's events run its handlers, the `-On<Event>` script blocks its command takes, with the event as their argument and as `$_`: its `Window`, the widget's `Name`, its `Kind`, its `Value`, and `State`, every named input's current value. Without `-NoWait` the handlers run on the command's own thread as the events arrive, and inside a handler the update commands act on its window without `-Window`. What a handler writes is dropped unless `-HandlerOutput` says otherwise: `Pipeline` writes it as the command's output before the result, and `Console` sends it to Out-Default. While the command waits, an error a handler throws or writes becomes the command's own error record, which `-ErrorVariable`, `-ErrorAction` and a redirection act on, and the window goes on.

The window closes when a person closes it, when a button or menu item with `-Result` is pressed, or when [Close-GuiWindow](Close-GuiWindow.md) closes it, from a handler or from anywhere else. The command then runs `-OnClose`, and writes one `PwrsGui.WindowResult` whose `Result` is the button's, the item's or Close-GuiWindow's result, or `Closed`, and whose `State` gives each named input's last value.

With `-NoWait` the command writes a `PwrsGui.Window` once the window has opened, with its first frame drawn, which is when [Get-GuiWindow](Get-GuiWindow.md) lists it. Its events wait in a queue of the module's own for [Wait-GuiEvent](Wait-GuiEvent.md), whether or not a handler ran for them, and its handlers run as engine-event actions when the engine is idle, as at an interactive prompt. [Update-GuiWidget](Update-GuiWidget.md), [Add-GuiItem](Add-GuiItem.md), [Remove-GuiItem](Remove-GuiItem.md) and Close-GuiWindow change it by its `Id`.

The window's own shortcuts, which no widget has, come from `-OnShortcut`, a hashtable from each, written as `Ctrl+S`, to its handler, whose event's `Kind` is `Shortcut` and `Value` the shortcut. An enabled menu item's shortcut acts before the window's own of the same keys. Files or text dropped where no widget takes them go to `-OnDrop`.

`-WatchPath` names files the window watches while it is open, each a path from the current location; a file need not exist yet, but its folder must. A change to one, such as a line added to a log, raises an event whose `Kind` is `FileChange`, whose `Name` is the file's full path and whose `Value` is its length in bytes, `$null` while the file is gone. `-OnFileChange` runs with it, and with `-NoWait` Wait-GuiEvent takes it too. The system's change notifications tell of each change, and on Windows a look at each file once a second as well, since NTFS reports no write that a writer leaves in the system's cache until it closes the file. `-WatchInterval` looks at the files at that pace instead, on any system, for a file system that gives no notifications, such as a network share. A file has one change waiting at a time, and the next comes once that one is taken, so a script that reads the file to its end at each change misses nothing.

### A page of your own

The page is the HTML `-Html` gives, or the file or folder `-Path` names; a folder's `index.html` opens first, and the page loads the folder's other files by relative path. HTML given with `-Html` that does not begin with a doctype loads behind `<!DOCTYPE html>`, so the webview lays it out by the current HTML and CSS standards. A page without a doctype gets older rules, called quirks mode, under which a table does not take the page's font size. A file from `-Path` loads as it is written. It is drawn by a webview: WebView2 on Windows, WebKit on macOS and WebKitGTK on Linux and FreeBSD. Pages load through `pwrs://`, from memory or from the folder, with no network server. Before a page loads, the renderer turns the engine's own chrome off: WebView2's accelerator keys, zoom, pinch zoom, swipe navigation, status bar and default context menu; WebKit's magnification and navigation gestures; WebKitGTK's navigation gestures and context menu. An engine older than the oldest the tests run, WebView2 154, macOS 15.8's WebKit (621) and WebKitGTK 2.46, draws with a warning naming both. Where the WebView2 add-on is installed, its Fixed Version runtime draws pages in place of the evergreen one ([The WebView2 Add-On](The-WebView2-Add-On.md)).

The page reaches the session through `window.pwrs`:

| Member | Does |
|---|---|
| `send(value)` | passes the value to `-OnMessage`, and returns a Promise of what the handler returns: `null` when it returns nothing, or when there is no handler |
| `on(handler)` | registers a function that takes each value [Send-GuiMessage](Send-GuiMessage.md) sends |
| `close(value)` | closes the window; the value is the command's result |
| `save(value)` | stores a value in the renderer, for the page loaded again after its engine ended |
| `restore()` | returns a Promise of the value `save` stored last; `null` when nothing was saved |

Values cross as JSON. What the page sends arrives as `ConvertFrom-Json` makes it, with an array left whole and an object's properties in the order the page wrote them. What a handler returns goes back one object as itself and several as an array. It is made plain data to a depth of 100 first, the same in both hosts: a string or number without the note properties a provider attaches, as a file name from `Get-ChildItem -Name` has them; a date as an ISO 8601 string; a list, dictionary or custom object walked.

Without `-NoWait` the command waits until the window closes, and writes one `PwrsGui.WindowResult` whose `Result` is the value the page gave `close`, or `Closed` when the person or the session closed the window. `-OnMessage` runs on the pipeline thread while the command waits. Ctrl+C closes the window and writes nothing. With `-NoWait` the command writes a `PwrsGui.PageWindow` once the page has loaded and drawn its first frame, and `-OnMessage` runs as an engine-event action when the engine is idle.

A page stays on its own origin unless `-AllowNavigation`: it loads with a content-security policy that allows `pwrs://`, `data:` and `blob:` URLs and inline script and style, a navigation to another origin is refused, and a message from another origin is refused and reported. `-DevTools` lets the person open the webview's developer tools, with the engine's developer keys and its default context menu left on.

When the engine drawing a page ends, as when its process crashes or is killed, the renderer loads the page again, in a new webview when the engine took the webview with it, and the page's `restore()` returns what it saved; the command goes on waiting. A page that ends again before it has drawn closes its window, and the command fails with `GuiPageFailed`, naming what ended. [Web Page Data And Recovery](Web-Page-Data-And-Recovery.md) explains both.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | `Content`: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The block that writes the window's widgets, in which each widget's short name stands for its `New-Gui<Widget>` command.

### -Html

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Html`: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The page's HTML, which loads as `index.html`, its only file. Text that does not begin with a doctype loads behind `<!DOCTYPE html>`.

### -Path

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Path`: named, mandatory |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiNoPage` for a path that resolves to nothing, a folder with no `index.html`, or a file in no folder |

A page's file, or a folder whose `index.html` is the page. A relative path is taken from the current location. Every file under the folder loads by its path; a path that leaves the folder is refused.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `PWRS-GUI` for `-Content` and `-Html`, the first file's name for `-Path` |

The window's title, which [Get-GuiWindow](Get-GuiWindow.md) `-Title` matches.

### -Width, -Height

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Values | 100 to 10000 points |
| Default | a window of widgets fits its widgets; a page's is 800 by 600 |
| Refused with | `ParameterArgumentValidationError` from the binder outside that range |

The window's inside width or height in points, which the display's scale turns into pixels. A side given is that size, at most the screen's work area; a side not given fits a window's widgets, and a page's window is 800 points across and 600 down.

### -Renderer

| | |
|---|---|
| Type | `PwrsGui.Renderer` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | the one `Set-GuiOption -Renderer` names |

The renderer that draws the window of widgets, `Egui` or `Html`.

### -HandlerOutput

| | |
|---|---|
| Type | `PwrsGui.HandlerOutput` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | `Discard` |

What happens to what the handlers write: `Discard` drops it; `Pipeline` writes it as the command's output before the result, and in a `-NoWait` window into the handler's event job; `Console` sends it to Out-Default.

### -OnClose

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | none |

Runs once the window has closed, with an event whose `Kind` is `Close`, whose `Value` is the window's result and whose `State` is the inputs' last values.

### -OnShortcut

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiBadShortcut` for a key that is not written as a shortcut, a shortcut given twice, or a value that is not a script block |

The window's own keyboard shortcuts: a hashtable from each, as `Ctrl+S`, to the script block that runs when it is pressed.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the window where no widget under the pointer takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -WatchPath

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | none: no file is watched |
| Refused with | `GuiBadWatch` for no file or a file named twice; `GuiNoWatch` for a file whose folder does not exist |

Files the window watches while it is open, each a path from the current location; a file need not exist yet, but its folder must. A change to one raises a `FileChange` event, whose `Name` is the file's full path and whose `Value` is its length in bytes.

The watch reads only each file's length and when it was last written, never what is in the file, so it never gets in the way of the program writing it. A handler that reads the file should open it only while it reads, since Windows PowerShell 5.1's `Add-Content` and `Set-Content` fail on a file another program has open; [Following A Log](Following-A-Log.md) builds such a reader.

### -WatchInterval

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | none: the system's change notifications, and on Windows a look once a second as well |
| Refused with | `GuiBadWatch` without `-WatchPath`, or for a value that is not seconds above 0 or a `[TimeSpan]` |

The seconds, or a `[TimeSpan]`, between looks at the watched files, in place of the system's change notifications; for a file system that gives none, such as a network share.

### -OnFileChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiBadWatch` without `-WatchPath` |

Runs when a watched file changes, with the event as its argument and as `$_`; its `Name` is the file's full path and its `Value` its length in bytes, `$null` while the file is gone.

### -OnMessage

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | `Html`, `Path`: named |
| Pipeline input | no |
| Default | none: every send gets `null` back |

Runs for each value the page sends with `window.pwrs.send`, with the value as its argument. What it writes goes back to the page as the send's result.

### -AllowNavigation

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Html`, `Path`: named |
| Pipeline input | no |
| Default | off |

Lets the page navigate away from its own origin, load remote content, and take messages from other origins.

### -DevTools

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Html`, `Path`: named |
| Pipeline input | no |
| Default | off |

Lets the person open the webview's developer tools: the engine's developer keys and default context menu stay on, and every other setting of its chrome stays off.

### -NoWait

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Writes the window once it has opened, with its first frame drawn, instead of waiting for it to close.

## Inputs

None.

## Outputs

Without `-NoWait`, one `PwrsGui.WindowResult`:

| Property | What it gives |
|---|---|
| `Result` | for a window of widgets, the result of the button or menu item pressed, or of Close-GuiWindow; for a page, the value it gave `close`, as `ConvertFrom-Json` makes it; `Closed` when the person or the session closed the window |
| `State` | for a window of widgets, each named input's last value, under its name |

With `-NoWait`, for a window of widgets one `PwrsGui.Window`:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the window's number, which the update commands, Wait-GuiEvent and Close-GuiWindow take as `-Window` |
| `Title` | `System.String` | the window's title |
| `Names` | `System.String[]` | the names of its widgets, in tree order |
| `SourceIdentifier` | `System.String` | the engine event its handlers run on, which `Unregister-Event` removes; null without handlers |

and for a page one `PwrsGui.PageWindow`:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the window's number, which [Send-GuiMessage](Send-GuiMessage.md) takes as `-Window` |
| `Title` | `System.String` | the window's title |
| `SourceIdentifier` | `System.String` | the engine event each send raises for `-OnMessage`'s action; null without `-OnMessage` |

## Examples

The examples from the command's help need a person to press a button or to click through a site of their own:

<!-- skip: waits for a person to press OK -->
```powershell
Show-GuiWindow { Text 'Deploy to' -Style Instruction; ListBox 'staging', 'production' -Name Target; ButtonBar { Button 'OK' -Default -Result Ok; Button 'Cancel' -Cancel -Result Cancel } }
```

<!-- skip: waits for a person to press Done -->
```powershell
Show-GuiWindow -Html '<button onclick="pwrs.close(42)">Done</button>'
```

<!-- skip: needs a site folder of the reader's own and a person -->
```powershell
Show-GuiWindow -Path .\site -OnMessage { param($m) "got $m" }
```

A dialog of widgets, a target picked and OK pressed; the result gives the button's result and the inputs' state:

<!-- person: $w = Wait-Window 'Deploy'; Invoke-GuiTestEvent -Window $w.Id -Name Target -Event Select -Value 1; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Deploy' {
    Text 'Deploy to' -Style Instruction
    ListBox 'staging', 'production' -Name Target
    CheckBox 'Dry run' -Name DryRun -Checked
    ButtonBar {
        Button 'OK' -Name Ok -Default -Result Ok
        Button 'Cancel' -Name Cancel -Cancel -Result Cancel
    }
}
$result.Result
$result.State.Target
$result.State.DryRun
```

```text
Ok
production
True
```

Handlers whose output is written before the result, a shortcut of the window's own, and the window's Close:

<!-- person: $w = Wait-Window 'Handlers'; Invoke-GuiTestEvent -Window $w.Id -Name Go -Event Click; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'F5'; Close-GuiWindow -Window $w.Id -Result 'from outside' -->
```powershell
$out = Show-GuiWindow -Title 'Handlers' -HandlerOutput Pipeline -OnShortcut @{ 'F5' = { "refresh by $($_.Kind)" } } -OnClose { "closed with $($_.Value)" } {
    Button 'Go' -Name Go -OnClick { "pressed $($_.Name)" }
}
$out[0..2]
$out[3].Result
```

```text
pressed Go
refresh by Shortcut
closed with from outside
from outside
```

A window that does not wait, whose events are read from the queue:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Queue' { ListBox 'a', 'b' -Name Pick; ButtonBar { Button 'OK' -Name Ok -Default -Result Ok } }
$w.Names
Invoke-GuiTestEvent -Window $w.Id -Name Pick -Event Select -Value 1
$picked = Wait-GuiEvent -Window $w.Id -Timeout 20
'{0} {1} {2}' -f $picked.Kind, $picked.Name, $picked.Value
Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
(Wait-GuiEvent -Window $w.Id -Timeout 20).Kind
(Wait-GuiEvent -Window $w.Id -Timeout 20).Kind
```

```text
Pick
Ok
Select Pick b
Click
Close
```

A window that watches a log: a line added to it raises `FileChange`, whose `Value` is the log's new length, and the script adds the line to the window. Press Close:

<!-- keep: show-guiwindow-watch.png images/show-guiwindow-watch.png -->
<!-- person: $w = Wait-Window 'Log'; Wait-Property $w Lines lines 'started stopping'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'show-guiwindow-watch.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
Set-Content -Path app.log -Value 'started'
$w = Show-GuiWindow -NoWait -Title 'Log' -WatchPath app.log {
    LogView (Get-Content -Path app.log) -Name Lines -Height 120
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
Add-Content -Path app.log -Value 'stopping'
$changed = Wait-GuiEvent -Window $w.Id -Timeout 20
'{0} {1} {2}' -f $changed.Kind, (Split-Path -Leaf $changed.Name), $changed.Value
Add-GuiItem -Window $w.Id -Name Lines -Item (Get-Content -Path app.log)[-1]
do { $e = Wait-GuiEvent -Window $w.Id } until ($e.Kind -eq 'Close')
```

```text
FileChange app.log 19
```

![The Log window with the lines started and stopping in a log area, and a Close button at the bottom right](/images/show-guiwindow-watch.png)

A page that closes with a value; the result is the value as `ConvertFrom-Json` makes it:

```powershell
$r = Show-GuiWindow -Html '<script>window.pwrs.close({ count: 42, list: [1, 2] });</script>'
$r.Result
```

```text
count list
----- ----
   42 {1, 2}
```

A page that sends the session a value, and closes with what comes back:

```powershell
$page = '<script>window.pwrs.send("ping").then(function (r) { window.pwrs.close(r); });</script>'
(Show-GuiWindow -Html $page -OnMessage { param($m) "$m, pong" }).Result
```

```text
ping, pong
```

A folder's page, which loads its script from the folder:

```powershell
$null = New-Item -ItemType Directory site
Set-Content site/index.html '<script src="app.js"></script>'
Set-Content site/app.js 'window.pwrs.close("from app.js")'
(Show-GuiWindow -Path site -Title 'A site').Result
```

```text
from app.js
```

A page that does not wait, which a value from `Send-GuiMessage` closes:

<!-- varies: the window's number -->
```powershell
$w = Show-GuiWindow -Html '<script>window.pwrs.on(function (v) { window.pwrs.close(v); });</script>' -Title 'Waiting page' -NoWait
$w
Send-GuiMessage -Window $w.Id -Message 'done'
```

```text
                 Id Title        SourceIdentifier
                 -- -----        ----------------
4611686018427387908 Waiting page
```

A page that tries to leave its origin stays on it:

```powershell
$page = '<script>location.href = "https://example.com/"; setTimeout(function () { window.pwrs.close(location.origin); }, 1000);</script>'
(Show-GuiWindow -Html $page).Result
```

```text
http://pwrs.localhost
```

## Errors

A `-Path` that names nothing, and a block that writes no widget:

```powershell
try { Show-GuiWindow -Path 'no-such-site' } catch { $_.FullyQualifiedErrorId }
try { Show-GuiWindow { $null } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNoPage,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
GuiNoWidgets,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
-Content wrote no widget
```

| Error ID | When |
|---|---|
| `GuiNoWidgets` | the `-Content` block writes no widget |
| `GuiNotAWidget` | the `-Content` block writes something other than a widget |
| `GuiNameTwice` | two widgets of the window have one name |
| `GuiBadShortcut` | `-OnShortcut` has a key that is not written as a shortcut, a shortcut twice, or a value that is not a script block |
| `GuiBadWatch` | `-WatchPath` names no file or a file twice, `-WatchInterval` is not seconds above 0 or a `[TimeSpan]`, or `-WatchInterval` or `-OnFileChange` is given without `-WatchPath` |
| `GuiNoWatch` | a file `-WatchPath` names cannot be watched: its folder does not exist, or the system's change notifications cannot start |
| `GuiTabOutsideTabs`, `GuiNotATab`, `GuiNotTwoWidgets`, `GuiNotAtTop`, `GuiNotInMenu`, `GuiNotInTree`, `GuiNoWidget` | a widget is placed where it cannot go, as each widget's page says |
| `GuiNoPage` | `-Path` resolves to nothing, a folder with no `index.html`, or a file in no folder |
| `GuiNoWebview` | the HTML renderer cannot start a page, naming its reason and what to install when the engine is missing |
| `GuiPageFailed` | a page cannot be shown: its webview could not be made, as when WebView2 refuses another process's options on the shared data folder, or a page loaded again after its engine ended ends again before drawing |

The [Error Reference](Error-Reference.md) describes every error the module raises.
