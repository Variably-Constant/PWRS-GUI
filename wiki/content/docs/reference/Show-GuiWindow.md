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
Show-GuiWindow [-Content] <scriptblock> [-Title <string>] [-Width <double>] [-Height <double>] [-Renderer <Renderer>] [-Style <string>] [-StylePath <string>] [-HandlerOutput <HandlerOutput>] [-OnClose <scriptblock>] [-OnShortcut <Object>] [-OnDrop <scriptblock>] [-WatchPath <string[]>] [-WatchInterval <Object>] [-OnFileChange <scriptblock>] [-NoWait] [<CommonParameters>]

Show-GuiWindow [-Html] <string> [-Title <string>] [-Width <double>] [-Height <double>] [-OnMessage <scriptblock>] [-AllowNavigation] [-DevTools] [-Frameless] [-HideWindowButtons] [-Call <Object[]>] [-CallErrors <CallErrors>] [-TrustNavigation] [-EndWithFirstWindow] [-Channel <Object[]>] [-OnDrop <scriptblock>] [-NoWait] [<CommonParameters>]

Show-GuiWindow [-Html] <string> -Connect <string[]> [-Title <string>] [-Width <double>] [-Height <double>] [-OnMessage <scriptblock>] [-AllowNavigation] [-DevTools] [-Frameless] [-HideWindowButtons] [-Call <Object[]>] [-CallErrors <CallErrors>] [-TrustNavigation] [-EndWithFirstWindow] [-Channel <Object[]>] [-OnDrop <scriptblock>] [-NoWait] [<CommonParameters>]

Show-GuiWindow [-Html] <string> -ContentSecurityPolicy <string> [-Title <string>] [-Width <double>] [-Height <double>] [-OnMessage <scriptblock>] [-AllowNavigation] [-DevTools] [-Frameless] [-HideWindowButtons] [-Call <Object[]>] [-CallErrors <CallErrors>] [-TrustNavigation] [-EndWithFirstWindow] [-Channel <Object[]>] [-OnDrop <scriptblock>] [-NoWait] [<CommonParameters>]

Show-GuiWindow -Path <string> [-Title <string>] [-Width <double>] [-Height <double>] [-OnMessage <scriptblock>] [-AllowNavigation] [-DevTools] [-Frameless] [-HideWindowButtons] [-Call <Object[]>] [-CallErrors <CallErrors>] [-TrustNavigation] [-EndWithFirstWindow] [-Channel <Object[]>] [-OnDrop <scriptblock>] [-NoWait] [<CommonParameters>]

Show-GuiWindow -Path <string> -Connect <string[]> [-Title <string>] [-Width <double>] [-Height <double>] [-OnMessage <scriptblock>] [-AllowNavigation] [-DevTools] [-Frameless] [-HideWindowButtons] [-Call <Object[]>] [-CallErrors <CallErrors>] [-TrustNavigation] [-EndWithFirstWindow] [-Channel <Object[]>] [-OnDrop <scriptblock>] [-NoWait] [<CommonParameters>]

Show-GuiWindow -Path <string> -ContentSecurityPolicy <string> [-Title <string>] [-Width <double>] [-Height <double>] [-OnMessage <scriptblock>] [-AllowNavigation] [-DevTools] [-Frameless] [-HideWindowButtons] [-Call <Object[]>] [-CallErrors <CallErrors>] [-TrustNavigation] [-EndWithFirstWindow] [-Channel <Object[]>] [-OnDrop <scriptblock>] [-NoWait] [<CommonParameters>]
```

## Description

`Show-GuiWindow` opens a window in the helper, or in-process as [Set-GuiOption](Set-GuiOption.md) `-Mode` says. It shows one of two things.

### A window described in PowerShell

The `-Content` block writes its widgets, one under another, as a [Column](New-GuiColumn.md) lays them out: several widgets are the children of a Column at the window's root, and a block that writes one widget makes that widget the root, as [Get-GuiSnapshot](Get-GuiSnapshot.md)'s `Tree` shows. While the block runs, each widget's short name, such as `Column`, `Text`, `Button` and `ListBox`, is an alias to its `New-Gui<Widget>` command, in a scope that ends with the block, so nothing outside it is shadowed. An alias is found before a function or a cmdlet, so `Group` is the widget there, and Group-Object is spelled out. The window is drawn by egui, or by the HTML renderer with `-Renderer Html`; a window meant for HTML is drawn by egui when no webview engine can start, and the Verbose stream says why. In HTML the window looks as egui draws it, with every control and popup drawn by the page and none by the engine, unless `-StylePath` or `-Style` gives CSS of your own, which the page applies after its own styles. Each widget's element carries `data-kind`, its kind, as `Button`, and `data-name`, the name PowerShell gave it, for that CSS to select it by. A window egui draws keeps egui's look. Both use the same palette, spacing and typeface: the fonts the module brings, Inter for text and JetBrains Mono NL for monospace, or with [Set-GuiOption](Set-GuiOption.md) `-Font System` the system's own.

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
| `send(value)` | passes the value to `-OnMessage`, with the window that sent it, and returns a Promise of what the handler returns: `null` when it returns nothing, or when there is no handler |
| `call(name, args)` | runs what `-Call` allows under the name, with the arguments given, and returns a Promise of its output, as `send` does; rejected with an `Error` when the call fails or is refused |
| `on(handler)` | registers a function that takes each value [Send-GuiMessage](Send-GuiMessage.md) sends |
| `channel(name)` | the page's channel to its backend that `-Channel` gives under the name, as a WebSocket is: `send(data)`, a Promise settled once the data is in the channel's ring, `onopen`, `onmessage`, `onclose`, `onerror` and `close()`; [New-GuiChannel](New-GuiChannel.md) describes it |
| `ondrop(handler)` | registers a function that takes each drop of files on the page: `paths`, the files' full paths, `x` and `y`, where they landed, and `taken`, whether the page's own handlers took the drop with `preventDefault` |
| `close(value)` | closes the window; the value is the command's result |
| `save(value)` | stores a value in the renderer, for the page loaded again after its engine ended |
| `restore()` | returns a Promise of the value `save` stored last; `null` when nothing was saved |
| `window` | the page's own window, whose title bar a page opened with `-Frameless` draws itself: `minimize()`, `maximize()`, `restore()`, `toggleMaximize()`, and `close()`, which closes it as the system's close button does; `startDrag()` and `startResize(edge)` hand a move or a resize to the system while the pointer's button is held; `move(x, y)` and `resize(width, height)`, its top left on the screen and the size of its inside in points; `focus()`; `fullscreen(on)`; `setTitle(title)`; `on(kind, handler)` for its `move`, `resize`, `focus` and `blur`; `onclose(handler)`; `buttons`, the box of the system's own buttons over a page without its frame, `null` where there are none; `id`, the window's number as text; `open(options)`, `list()` and `of(id)`, the other windows of the page's app |

Values cross as JSON. What the page sends arrives as `ConvertFrom-Json` makes it, with an array left whole and an object's properties in the order the page wrote them. What a handler returns goes back one object as itself and several as an array. It is made plain data to a depth of 100 first, the same in both hosts: a string or number without the note properties a provider attaches, as a file name from `Get-ChildItem -Name` has them; a date as an ISO 8601 string; a list, dictionary or custom object walked.

`call(name, args)` runs what `-Call` allows under that name: a command `-Call` names, called by its own name, or a script block a hashtable in `-Call` gives a name to, the name matched without regard to case. An object's properties are splatted as named arguments, an array's elements are positional ones, and any other value is one argument; no arguments, or `null`, give none. The output comes back as a handler's does. An error rejects the Promise with an `Error` whose `message` is the error's, with its `errorId`, the record's `FullyQualifiedErrorId`, its `category`, and `output`, what the call wrote before it. With `-CallErrors Any`, the default, the first error record the call writes or throws rejects it. With `-CallErrors Terminating` only an error that ends the call does; the others are written as the command's own errors, and the Promise resolves with the output. A call of a name `-Call` does not give is refused with an `Error` naming it. Calls run one at a time on the pipeline thread, as `-OnMessage` does, with no time limit.

A page keeps every DOM drag and drop event, its own drags included, as in a browser, where a `File` carries no path. Once the page's handlers of its elements and document have seen a drop of files, its `ondrop` handlers get the files' full paths. Then, unless the page took the drop with `preventDefault`, `-OnDrop` runs with an event whose `Kind` is `Drop` and whose `Value` is the paths. A drop the page has no handler for goes ahead when `-OnDrop` is given or the page has an `ondrop` handler. On Windows the paths come from WebView2 with the page's own report of the drop, and elsewhere from the system's drop, which is then handed on to the engine.

A page also moves, resizes and names its own window. `move` and `resize` leave at least 48 points of the window, across and down, on a monitor. `focus()` gives the window the keyboard where the system allows it, and otherwise asks for the person's attention, as a flashing taskbar button. `on(kind, handler)` tells the page of each move, with the window's top left as `x` and `y`, each resize, with its inside's `width` and `height`, and each `focus` and `blur`; a change made while the page loads is told once it has loaded. A window without `-Title` takes its document's title whenever the page sets one, and [Get-GuiWindow](Get-GuiWindow.md) finds it by that title, as by one `setTitle` gives. An `onclose` handler is given each close as the window's person's, from the system's close button or the page's own `close()`, and keeps the window open by calling `preventDefault` on what it is given. The page has 10 seconds to answer such a close, keeping the window open or letting it go. A window whose page gives no answer in that time, as when a handler throws, closes as the person's close, and the command writes a warning saying so. A window the page keeps open stays open as long as the page keeps it. A close from [Close-GuiWindow](Close-GuiWindow.md), Ctrl+C or the session is never held.

A page opens other windows of its app with `window.pwrs.window.open(options)`. `path` opens a file of a `-Path` app's folder, by its path from the folder. `html` opens HTML text, which in a `-Path` app loads the folder's files by relative path. `url` opens an address at another origin, only when the command has both `-AllowNavigation` and `-TrustNavigation`. `title`, `width`, `height` and `frameless` set the window as `-Title`, `-Width`, `-Height` and `-Frameless` set the first, and the rest comes from the command's own window: its policy, `-Call`, `-OnMessage`, `-OnDrop`, `-DevTools` and origins. The Promise `open` returns gives the window once it has opened, and is rejected with an `Error` saying why when it does not open. A window is its `id`, its `title`, and the methods of `window.pwrs.window` that act on a window, from `minimize()` to `setTitle(title)`. Its `id` is the window's number as text, since a JavaScript number cannot hold every window's number exactly. `list()` returns a Promise of the app's open windows, and `of(id)` gives the app's window of that number. A page acts on its own app's windows alone, and its `close()` of another one is as that window's person's, which the other page's `onclose` handlers decide.

Without `-NoWait` the command waits until the app's last window closes, or with `-EndWithFirstWindow` until its own window closes, which closes the rest. It writes one `PwrsGui.WindowResult` whose `Result` is the value its own window's page gave `close`, or `Closed` when the person or the session closed that window. `-OnMessage` runs on the pipeline thread while the command waits, for every window's sends, with the value and the window that sent it, a `PwrsGui.PageWindow`, as its arguments. A later window whose page cannot be shown is an error the command writes, and the app goes on. Ctrl+C closes every window of the app and writes nothing. With `-NoWait` the command writes a `PwrsGui.PageWindow` once the page has loaded and drawn its first frame, and `-OnMessage` and each call run as engine-event actions when the engine is idle; the errors `-CallErrors Terminating` leaves to the command go into the action's event job. [Get-GuiWindow](Get-GuiWindow.md) lists the app's later windows, whose sends and calls raise the same engine event.

A page stays on its own origin unless `-AllowNavigation`: a navigation to another origin is refused, and a message from another origin is refused and reported. With `-AllowNavigation`, a page from another origin may send, close the window and save as the page's own does. Its calls and its moves of the window are refused, though, and its `ondrop` handlers are given no paths, unless `-TrustNavigation` trusts every origin `-AllowNavigation` admits, as a page from a development server needs. `-OnDrop` still runs for the drops such a page leaves. Its files carry the content-security policy `default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'`, which allows `pwrs://`, `data:` and `blob:` URLs, inline script and style, and `eval`. `-Connect` adds the origins the page may fetch from and open WebSockets and event streams to, as a `connect-src` beside its own, and changes nothing else, so remote scripts, styles and pictures stay refused. `-ContentSecurityPolicy` gives the page's files a policy of your own, whole, in place of the default. The two are in parameter sets of their own, so the binder refuses both together, since a policy of your own writes its own `connect-src`. With `-AllowNavigation` alone the page's files carry no policy; with it and either of the two, they carry that policy. `window.pwrs` comes from the engine rather than from the page's files, so no policy takes it away. `-DevTools` lets the person open the webview's developer tools, with the engine's developer keys and its default context menu left on.

With `-Frameless` the window opens without the system's title bar and border, and the page draws its own. An element marked `data-pwrs-drag` moves the window when the primary button presses it, and a double-click on it maximizes or restores the window. The links, buttons, fields and editable text inside it keep their presses, as does anything marked `data-pwrs-no-drag`. On Windows such an element is the window's caption to the system, which also snaps the window to an edge of the screen, and the window keeps the system's shadow and resize edges. On Linux and FreeBSD the page's edges resize the window: the bridge puts a strip along each edge and a square at each corner, above all the page draws. On macOS the window's close, minimize and zoom buttons stay over the page's top left, where `window.pwrs.window.buttons` says, for the page to leave room for them, unless `-HideWindowButtons` hides them too. Under [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) `-Input`, a move or a resize the page starts is not made, and [Get-GuiSnapshot](Get-GuiSnapshot.md)'s `Moving` names it.

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
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The page's HTML, which loads as `index.html`, its only file. Text that does not begin with a doctype loads behind `<!DOCTYPE html>`.

### -Path

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Path`, `PathConnect`, `PathPolicy`: named, mandatory |
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
| Default | `PWRS-GUI` for `-Content` and `-Html`, the first file's name for `-Path`; a page's window takes its document's title whenever the page sets one |

The window's title, which [Get-GuiWindow](Get-GuiWindow.md) `-Title` matches. A page given one keeps it whatever title its document sets, though the page's `window.pwrs.window.setTitle` still names the window.

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

### -Style

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | none: the window looks as egui draws it |

CSS for the window's widgets, which the HTML renderer's page applies after its own styles and after `-StylePath`'s file, so a rule of yours takes the place of one of the page's that selects the same elements as strongly. Each widget's element carries `data-kind`, its kind, as `Button` or `ListBox`, and `data-name`, the name PowerShell gave it, so `[data-name="Go"]` selects the widget named `Go`. A window that fits its widgets is measured with your CSS applied. A window egui draws keeps egui's look.

### -StylePath

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Content`: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiNoFile` for a path that names no file that can be read, or several, or a file whose text is neither UTF-8 nor marked as UTF-16 |

A `.css` file for the window's widgets, a path from the current location, read as UTF-8, or as UTF-16 when a byte order mark says so. The page applies it as it applies `-Style`, before `-Style`'s CSS, so a file can hold an app's look and `-Style` change a rule of it for one window.

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
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the window where no widget under the pointer takes them, with the event as its argument and as `$_`; its `Value` is the paths dropped, or the text. On a page of your own it runs for files the page's handlers leave, not taking the drop with `preventDefault`, after the page's `ondrop` handlers; there what it writes is dropped, and an error it raises becomes the command's own.

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
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | none: every send gets `null` back |

Runs for each value a page of the app sends with `window.pwrs.send`, with the value and the window that sent it, a `PwrsGui.PageWindow`, as its arguments. What it writes goes back to the page as the send's result.

### -AllowNavigation

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | off |

Lets the page navigate away from its own origin, load remote content, and take messages from other origins.

### -DevTools

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | off |

Lets the person open the webview's developer tools: the engine's developer keys and default context menu stay on, and every other setting of its chrome stays off.

### -Connect

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | `HtmlConnect`, `PathConnect`: named, mandatory |
| Pipeline input | no |
| Default | none: the page reaches only its own origin |
| Refused with | `GuiBadPolicy` for a source that is not one; `AmbiguousParameterSet` from the binder with `-ContentSecurityPolicy` |

Origins the page may fetch from and open WebSockets and event streams to, beside its own. Write them as a content-security policy's `connect-src` writes them: a host with its scheme and port, such as `https://api.example.com` or `ws://localhost:8080`, any host under a name with `*.`, as `https://*.example.com`, or a scheme alone, such as `wss:`. They join a `connect-src` of the page's own sources, and the rest of its policy stays as it is. A source with a space, a semicolon, a comma or a quote in it is refused, since it would change more of the policy than `connect-src`.

### -ContentSecurityPolicy

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `HtmlPolicy`, `PathPolicy`: named, mandatory |
| Pipeline input | no |
| Default | none: the page's files carry `default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'` |
| Refused with | `GuiBadPolicy` for a policy that is empty, runs to more than one line or holds a character outside ASCII; `AmbiguousParameterSet` from the binder with `-Connect` |

The content-security policy the page's own files carry, whole, in place of the default; with `-AllowNavigation` they still carry it. The engine reads it as it reads any page's policy.

### -Frameless

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | off: the system draws the window's title bar and border |

Opens the window without the system's title bar and border, so the page draws its own; on Windows the window keeps the system's shadow. An element marked `data-pwrs-drag` is a title bar: a press on it moves the window, and a double-click maximizes or restores it, but for what inside it takes presses itself and what is marked `data-pwrs-no-drag`. The page's `window.pwrs.window` gives it a title bar's other moves. `close()` closes the window as the system's close button does, so the command's result is `Closed`. `startDrag()` and `startResize(edge)` work while the pointer's button is held, as in a `mousedown` handler, the edge being `n`, `ne`, `e`, `se`, `s`, `sw`, `w` or `nw`. Without `-NoWait`, a move the window cannot make is written as a warning naming it. On macOS the window's three buttons stay over the page's top left, and `window.pwrs.window.buttons` gives their box, as `x`, `y`, `width` and `height` in points; elsewhere it is `null`.

### -HideWindowButtons

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | off: on macOS the window's close, minimize and zoom buttons stay over the page |
| Refused with | `GuiBadFrame` without `-Frameless` |

With `-Frameless`, hides the window's close, minimize and zoom buttons on macOS too, so the page draws all three as it does on the other systems, and `window.pwrs.window.buttons` is `null`. The other systems show no buttons over a page without the system's frame, so there it changes nothing.

### -Call

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | none: every call is refused |
| Refused with | `GuiBadCall` for a name that is empty, holds a wildcard or names no command, a hashtable value that is not a script block, a value that is neither a string nor a hashtable, and a name given twice |

What the page's `window.pwrs.call` may run. A string names a command, which the page calls by that name; the command is the one `Get-Command` finds when the window opens. A hashtable gives the names the page calls and the script block each runs. Names match without regard to case.

### -CallErrors

| | |
|---|---|
| Type | `PwrsGui.CallErrors` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Values | `Any`, `Terminating` |
| Default | `Any` |

Which errors of a call reject its Promise. With `Any`, every error record does: the first one the call writes or throws rejects it, with the output written before it. With `Terminating`, only an error that ends the call does; the others are written as the command's own errors, and the Promise resolves with the call's output.

### -TrustNavigation

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | off: only the page's own origin calls and moves its window |
| Refused with | `GuiBadNavigation` without `-AllowNavigation` |

Lets the origins `-AllowNavigation` admits call what `-Call` allows, move the page's window, and be given the full paths of files dropped on the page, as the page's own origin does: for an app whose page a development server serves. With it, a page may also open a window of the app at an address of another origin.

### -EndWithFirstWindow

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | off: the command waits for the app's last window |

Ends the app when the window the command opened closes, closing the windows its pages opened, as an app whose first window is its main one does. Without it the command waits until the app's last window closes. Either way the result is the first window's.

### -Channel

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | `Html`, `HtmlConnect`, `HtmlPolicy`, `Path`, `PathConnect`, `PathPolicy`: named |
| Pipeline input | no |
| Default | none: the page has no channel |
| Refused with | `GuiBadChannel` for a value that is neither a channel nor a hashtable of one, a key the hashtable does not take, a ring a channel cannot take, or a name given twice |

The page's channels to its backend, which it opens with `window.pwrs.channel(name)`: `PwrsGui.Channel` objects [New-GuiChannel](New-GuiChannel.md) wrote, or hashtables from a channel's name to a hashtable of its `ToPage` and `FromPage` rings, `SubEtha.Ring` objects a script or backend made, with `Text`, `Stream`, `MaxFrames`, `MaxBytes` and `Process` as New-GuiChannel takes them. The windows a page opens later take the channels too, and a channel is open in one window at a time. Once the app has ended, the folder of a channel New-GuiChannel made is removed; rings a script made are the script's to remove.

### -NoWait

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Writes the window once it has opened, with its first frame drawn, instead of waiting for it to close. Get-GuiWindow lists the windows its page opens later.

## Inputs

None.

## Outputs

Without `-NoWait`, one `PwrsGui.WindowResult`:

| Property | What it gives |
|---|---|
| `Result` | for a window of widgets, the result of the button or menu item pressed, or of Close-GuiWindow; for a page, the value it gave `close`, as `ConvertFrom-Json` makes it, the command's own window's in an app of several; `Closed` when the person or the session closed the window |
| `State` | for a window of widgets, each named input's last value, under its name |

With `-NoWait`, for a window of widgets one `PwrsGui.Window`:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the window's number, which the update commands, Wait-GuiEvent and Close-GuiWindow take as `-Window` |
| `Title` | `System.String` | the window's title |
| `Names` | `System.String[]` | the names of its widgets, in tree order |
| `SourceIdentifier` | `System.String` | the engine event its handlers run on, which `Unregister-Event` removes; null without handlers |

and for a page one `PwrsGui.PageWindow`, which `-OnMessage` is given too as the window that sent:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the window's number, which [Send-GuiMessage](Send-GuiMessage.md) takes as `-Window` |
| `Title` | `System.String` | the window's title as it opened |
| `SourceIdentifier` | `System.String` | the engine event each send raises for `-OnMessage`'s action, and each call of what `-Call` allows for the call's; null without either |

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

A window drawn in HTML with CSS of its own, which comes after the page's own rule for buttons and so moves the button 100 points to the right:

```powershell
$plain = Show-GuiWindow -NoWait -Renderer Html -Title 'Plain' { Button 'Go' -Name Go }
$styled = Show-GuiWindow -NoWait -Renderer Html -Title 'Styled' -Style 'button { margin-left: 100px; }' { Button 'Go' -Name Go }
(Get-GuiSnapshot -Window $plain.Id).Bounds.Go.X
(Get-GuiSnapshot -Window $styled.Id).Bounds.Go.X
Close-GuiWindow -Window $plain.Id
Close-GuiWindow -Window $styled.Id
```

```text
8
108
```

An app's CSS in a file, which moves every button by its kind, and a window's own rule after it, which moves the button named `Stop` further:

```powershell
Set-Content -Path app.css -Value '[data-kind="Button"] { margin-left: 50px; }'
$w = Show-GuiWindow -NoWait -Renderer Html -Title 'Hooks' -StylePath app.css -Style '[data-name="Stop"] { margin-left: 100px; }' {
    Button 'Go' -Name Go
    Button 'Stop' -Name Stop
}
$shot = Get-GuiSnapshot -Window $w.Id
$shot.Bounds.Go.X
$shot.Bounds.Stop.X
Close-GuiWindow -Window $w.Id
```

```text
58
108
```

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

A page that calls a command the window allows, its object's properties given as the command's named parameters:

```powershell
$page = '<script>window.pwrs.call("Get-Date", { Date: "2026-01-02", Format: "yyyy-MM-dd" }).then(function (d) { window.pwrs.close(d); });</script>'
(Show-GuiWindow -Html $page -Call 'Get-Date').Result
```

```text
2026-01-02
```

A page that calls a script block under a name of its own, its array given as positional arguments; the error the script raises rejects the call:

```powershell
$page = @'
<script>
window.pwrs.call('Divide', [10, 0]).catch(function (e) { window.pwrs.close(e.errorId + ': ' + e.message); });
</script>
'@
(Show-GuiWindow -Html $page -Call @{ Divide = { param($a, $b) $a / $b } }).Result
```

```text
RuntimeException: Attempted to divide by zero.
```

A page that closes with the full paths of the files dropped on it, which its `ondrop` handler is given; here a test drops two files in place of a person:

<!-- person: $w = Wait-Window 'Drop files'; Invoke-GuiTestEvent -Window $w.Id -Drop 'C:\reports\q3.txt', 'C:\reports\q4.txt' -->
```powershell
$page = '<p>Drop files here</p><script>window.pwrs.ondrop(function (d) { window.pwrs.close(d.paths); });</script>'
(Show-GuiWindow -Html $page -Title 'Drop files').Result
```

```text
C:\reports\q3.txt
C:\reports\q4.txt
```

A page whose window has no `-Title` takes its document's title, which [Get-GuiWindow](Get-GuiWindow.md) finds it by:

```powershell
$w = Show-GuiWindow -NoWait -Html '<title>Quarterly report</title><p>Figures</p>'
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (-not ($found = Get-GuiWindow -Title 'Quarterly report') -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
$found.Title
Close-GuiWindow -Window $w.Id
```

```text
Quarterly report
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

An app of two windows: the page opens a second, whose send reaches `-OnMessage` with the window that sent it, and the result is the first window's, though the second closes last:

```powershell
$page = @'
<script>
var second = '<scr' + 'ipt>setTimeout(function () { window.pwrs.send("ready").then(function () { window.pwrs.close("second"); }); }, 500);</scr' + 'ipt>';
window.pwrs.window.open({ html: second, title: 'Details', width: 400, height: 300 }).then(function () { window.pwrs.close('main'); });
</script>
'@
$heard = [System.Collections.Generic.List[string]]::new()
$r = Show-GuiWindow -Html $page -Title 'Main' -OnMessage { param($m, $sender) $heard.Add("$m from $($sender.Title)") }
$heard
$r.Result
```

```text
ready from Details
main
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
4611686018427387913 Waiting page
```

A page that tries to leave its origin stays on it:

```powershell
$page = '<script>location.href = "https://example.com/"; setTimeout(function () { window.pwrs.close(location.origin); }, 1000);</script>'
(Show-GuiWindow -Html $page).Result
```

```text
http://pwrs.localhost
```

A page that fetches from another origin is refused unless `-Connect` names it; here the host does not exist, so the fetch that is allowed fails on the network instead:

```powershell
$page = @'
<script>
document.addEventListener('securitypolicyviolation', function (e) { window.pwrs.close('refused by ' + e.effectiveDirective); });
fetch('https://api.example.invalid/').catch(function () { setTimeout(function () { window.pwrs.close('allowed, and no such host'); }, 500); });
</script>
'@
(Show-GuiWindow -Html $page).Result
(Show-GuiWindow -Html $page -Connect 'https://api.example.invalid').Result
```

```text
refused by connect-src
allowed, and no such host
```

A policy of your own replaces the default whole, so a page whose policy leaves out `'unsafe-eval'` cannot `eval`:

```powershell
$page = '<script>try { eval("1"); window.pwrs.close("eval runs"); } catch (e) { window.pwrs.close(e.name); }</script>'
(Show-GuiWindow -Html $page).Result
(Show-GuiWindow -Html $page -ContentSecurityPolicy "default-src 'self' 'unsafe-inline'").Result
```

```text
eval runs
EvalError
```

A window without the system's frame, whose page draws a title bar of its own: a press on the bar's empty part moves the window, and its buttons minimize, maximize and close it. This page closes itself as it loads, so the example needs no person; the result is the one a person's close gives:

```powershell
$page = @'
<style>
  body { margin: 0; font: 14px sans-serif; }
  #bar { display: flex; align-items: center; height: 32px; padding-left: 10px; background: #2b2b2b; color: #fff; }
  #bar .title { flex: 1; }
  #bar button { width: 46px; height: 32px; border: 0; background: none; color: inherit; }
</style>
<div id="bar">
  <span class="title">My app</span>
  <button onclick="pwrs.window.minimize()">&#x2013;</button>
  <button onclick="pwrs.window.toggleMaximize()">&#x25A1;</button>
  <button onclick="pwrs.window.close()">&#x2715;</button>
</div>
<script>
  document.querySelector('#bar .title').onmousedown = function (e) { if (e.button === 0) pwrs.window.startDrag(); };
  pwrs.window.close();
</script>
'@
(Show-GuiWindow -Html $page -Frameless -Title 'My app').Result
```

```text
Closed
```

A title bar marked `data-pwrs-drag`, pressed by a test's input: the window does not move under a test, and the snapshot names the move the press started. A press on the button at the bar's right end starts none, since a button inside the bar keeps its presses:

```powershell
$page = @'
<style>
  body { margin: 0; }
  #bar { display: flex; justify-content: flex-end; height: 32px; background: #2b2b2b; }
  #bar button { width: 46px; border: 0; }
</style>
<div id="bar" data-pwrs-drag><button>&#x2715;</button></div>
'@
$w = Show-GuiWindow -Html $page -Frameless -Title 'Bar' -NoWait
Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = 100; Y = 16 }, @{ Release = 'Left'; X = 100; Y = 16 }
(Get-GuiSnapshot -Window $w.Id).Moving
Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = 777; Y = 16 }, @{ Release = 'Left'; X = 777; Y = 16 }
$null -eq (Get-GuiSnapshot -Window $w.Id).Moving
Close-GuiWindow -Window $w.Id
```

```text
drag
True
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

A policy given whole beside origins to add to the default one, which the binder refuses, and buttons hidden on a window that keeps the system's frame:

```powershell
try { Show-GuiWindow -Html '<p>Hi</p>' -Connect 'https://api.example.com' -ContentSecurityPolicy "default-src 'self'" } catch { $_.FullyQualifiedErrorId }
try { Show-GuiWindow -Html '<p>Hi</p>' -HideWindowButtons } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
AmbiguousParameterSet,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
GuiBadFrame,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
-HideWindowButtons hides the buttons of a window without the system's frame; give -Frameless too
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
| `GuiBadPolicy` | a `-Connect` source is not one, or a `-ContentSecurityPolicy` is empty, runs to more than one line or holds a character outside ASCII |
| `GuiBadFrame` | `-HideWindowButtons` is given without `-Frameless` |
| `GuiBadCall` | a `-Call` name is empty, holds a wildcard or names no command; a hashtable's value is not a script block; a value is neither a string nor a hashtable; or a name is given twice |
| `GuiBadNavigation` | `-TrustNavigation` is given without `-AllowNavigation` |
| `GuiNoFile` | `-StylePath` names no file that can be read, or several, or a file whose text is neither UTF-8 nor marked as UTF-16 |
| `AmbiguousParameterSet` | from the binder: `-Connect` and `-ContentSecurityPolicy` are given together |
| `GuiNoWebview` | the HTML renderer cannot start a page, naming its reason and what to install when the engine is missing |
| `GuiPageFailed` | a page cannot be shown: its webview could not be made, as when WebView2 refuses another process's options on the shared data folder, or a page loaded again after its engine ended ends again before drawing |

The [Error Reference](Error-Reference.md) describes every error the module raises.
