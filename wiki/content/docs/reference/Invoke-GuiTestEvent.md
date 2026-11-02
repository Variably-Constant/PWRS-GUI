---
title: Invoke-GuiTestEvent
weight: 6
---

Acts on a widget as a person would, presses a window's shortcut, drops on a window, gives a window a person's input step by step, or closes one of the system's dialogs with a person's choice. Source: `crates/pwrs-gui/src/inspect.rs`, `InvokeGuiTestEvent` and `GuiEventKind`, `crates/pwrs-gui-proto/src/test_input.rs`, the wait for an input in `crates/pwrs-gui-client/src/pace.rs`, and `answer` in `crates/pwrs-gui/src/dialogs.rs`; the renderers' sides are `test_event` in `crates/pwrs-gui-egui/src/lib.rs`, `test` in `crates/pwrs-gui-web/src/runtime/runtime.js`, the input of `crates/pwrs-gui-egui/src/input.rs` and `crates/pwrs-gui-web/src/input.rs`, and the crate `pwrs-gui-dialogs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, window numbers and its names for unsigned integer types, `uint32` and `uint64` where PowerShell 7 writes `uint` and `ulong`, except where its output is shown too.

## Syntax

```powershell
Get-Command Invoke-GuiTestEvent -Syntax
```

```text
Invoke-GuiTestEvent [-Window] <ulong> [-Name] <string> [-Event] <EventKind> [[-Value] <Object>] [-PassThru] [<CommonParameters>]

Invoke-GuiTestEvent [-Window] <ulong> -Shortcut <string> [-PassThru] [<CommonParameters>]

Invoke-GuiTestEvent [-Window] <ulong> -Drop <Object> [-PassThru] [<CommonParameters>]

Invoke-GuiTestEvent [-Window] <ulong> -Input <Object> [-Timeout <double>] [-PassThru] [<CommonParameters>]

Invoke-GuiTestEvent [[-Value] <Object>] -Dialog <ulong> [-PassThru] [<CommonParameters>]
```

## Description

`Invoke-GuiTestEvent` raises an event on a widget of an open window, named as `Get-GuiWindow` lists its widgets. The renderer applies it as it would a person's input, so a list selects its rows, and sends the event to the command that owns the window, which reads it as it reads a person's. A test drives a dialog this way while the command showing it runs in another runspace of the same process, and reads the result that command writes.

It does no more than a person can: a disabled widget takes no click, change, Enter or key, a number outside a field's range is refused, and a widget takes no key it uses itself. Such an event, a widget the window does not have, or an event the widget does not raise, is a non-terminating `GuiRefused` error, naming the widgets the window has where the name is wrong, and the command goes on to the next window piped to it. The command returns once the renderer has applied the event; with `-PassThru` it writes the value the widget took. What the window's own command or handlers do in response, such as enabling a button, arrives after that, so a test that acts on it waits for it first, as with `Get-GuiSnapshot` in a loop.

| Event | Raised by | `-Value` |
|---|---|---|
| `Click` | a button, a link, a menu item; a Markdown document's link; a file chooser's Browse button, which opens its dialog for `-Dialog` to close | none; for a Markdown document, the address of one of its links |
| `Change` | an input whose value a person edits | the new value: `[bool]` for a check box, text for a field, a number for a number field or slider, the choice's number from 0 for a drop-down or option buttons (an editable drop-down's text), the tab's number for tabs, `#rrggbb` for a color, a `[datetime]` for a date, a path or paths for a file chooser |
| `Submit` | a text field of one line: Enter | its text |
| `Select` | a list, a table or a tree; a chart's point | the row number, or with several selectable an array of them, from 0, a tree's rows counted as shown; for a chart, a hashtable of the `Series`, `X` and `Y` of one of its points |
| `Activate` | a list, a table or a tree: a double-click, or Enter | the row number, from 0 |
| `Expand` | a tree: opening a node | the row number of the node, as shown; it raises `Expand` only when the node's children are still to come |
| `Drop` | a widget with `-OnDrop` | the paths dropped, as a list, or in an HTML window text; an egui window takes files alone |
| `Key` | a widget with `-OnKey` | the key written as a shortcut, as `Ctrl+K`; a Canvas refuses a key its view's controls take |
| `Peek` | a password field while it offers its eye, which it does once it has the focus and is typed into | `$true` holds the eye, so the password shows, as a person pressing it does, and `$false` lets it go; it raises nothing, and a field that is no password field, or does not offer the eye now, refuses it |
| `PointerDown`, `PointerUp`, `PointerMove`, `Wheel` | a GpuSurface or a Canvas with the handler of that name, once it has been drawn | a hashtable of `U` and `V`, the place as fractions of the widget's width and height, with any of `Button` (`Left`, `Right` or `Middle`; `Left` when a press or release names none), `Shift`, `Ctrl` and `Alt`, and for `Wheel` its `Notches`, 1 when absent; for a Canvas, `Item` in place of `U` and `V` puts the pointer at the middle of where that item was drawn |
| `Click`, `DoubleClick`, `PointerEnter`, `PointerLeave`, `DragStart`, `DragMove`, `DragEnd` | a Canvas with the handler of that name, once it has been drawn | the same hashtable as a Canvas's `PointerDown`, `Button` `Left` when it names none; the event names the topmost named item at that point, and raises nothing where no named item is |

`-Shortcut` presses one of the window's shortcuts: the enabled menu item with those keys is clicked, or else the window's own shortcut of `Show-GuiWindow -OnShortcut` raises `Shortcut`; it presses as though no text field had the focus. `-Drop` drops files on the window where no widget takes drops, given as a list of their paths, or in an HTML window text, which `Show-GuiWindow -OnDrop` takes. On a page of your own the files land at the middle of its window, as files the page makes of the names the paths end in, so the page's own drop handlers see them, and its `ondrop` handlers and `-OnDrop` are given the paths, which need not exist. A page takes no text, which is refused.

`-Input` gives the window a person's own input, a step at a time, through the renderer's input: an egui window takes each step as egui takes the system's, and an HTML window through its engine's own input. So the window does with it what it does with a person's, finding the widget a press reaches and opening a context menu at a right click. In both renderers a press and its release are a click when the release comes within 6 points and 0.8 seconds of the press, as egui counts one.

| Step | Keys | Does |
|---|---|---|
| press | `Press`, the button: `Left`, `Right` or `Middle`; `X`, `Y` | moves the pointer to the point, then presses the button there |
| release | `Release`, the button; `X`, `Y` | moves the pointer to the point, then lets the button go there |
| move | `Move = $true`; `X`, `Y` | moves the pointer to the point |
| key | `Key`, a key written as a shortcut is, as `Shift+F10`, or `Menu`, the key that opens a context menu | presses the key and lets it go |
| wait | `Wait`, seconds, 0 or more | waits that long |

Every step but a wait also takes `Shift`, `Ctrl` and `Alt`, `$true` holding the key through the step; `Ctrl` is the Command key on macOS, as in a shortcut. `X` and `Y` are points from the top left of the window's inside, where [Get-GuiSnapshot](Get-GuiSnapshot.md) places its `Bounds`. The pointer stays where the last step left it, buttons held included, for the next `-Input`. The command returns once the window has drawn what the last step did.

Each step has 10 seconds, or as many as `-Timeout` gives, to be taken and seen from when it begins; a wait has its own seconds besides. The first step that runs out of its time ends the command with a non-terminating `GuiTimeout` naming the step, and the window drops the steps left, so a renderer that stops drawing a window cannot hold the command. Ctrl+C drops them the same way. A key step copies, cuts and pastes as a person's keys do: Ctrl+C, Ctrl+X and Ctrl+V, and on Windows Ctrl+Insert, Shift+Delete and Shift+Insert, the paste taking the text on the system's clipboard.

A step that is not one is a terminating `GuiBadStep`, naming the step by its place. Each of these is a non-terminating `GuiRefused`, and no step is given: a point outside the window's inside, a press of a button the pointer already holds, or a release of one it does not. So is a window still taking another test's input or not yet drawn. A window that closes before every step is given refuses the rest.

A page of your own, as `Show-GuiWindow -Html` shows one, takes the steps as a window of widgets does, through its engine's own input. A move or a resize of its window that the page starts under them, as a press on a title bar it draws itself does, is not made, since the system's move would follow the person's own pointer. [Get-GuiSnapshot](Get-GuiSnapshot.md)'s `Moving` names it instead.

`-Dialog` closes one of the system's dialogs, by the number [Get-GuiDialog](Get-GuiDialog.md) lists it under, the platform's own way. Its command writes the choice `-Value` gives as though a person had made it: a message's button by name, `Ok`, `Cancel`, `Yes` or `No`, or a file or folder dialog's path or paths. Without `-Value` the dialog is canceled. A button the message does not show, more than one path for a dialog of one, and no path for a file dialog are refused with a non-terminating `GuiRefused`, and the dialog stays open. A word that names no button is a terminating `GuiBadAnswer`, and a number no dialog showing has a terminating `GuiNoDialog`.

## Parameters

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Aliases | `Id` |
| Parameter sets | `Widget`, `Shortcut`, `Drop`, `Input`: position 0, mandatory |
| Pipeline input | by property name (`Window`, `Id`) |
| Default | none |

The window's number, as [Get-GuiWindow](Get-GuiWindow.md) writes it in `Id`, so a window piped from `Get-GuiWindow` binds here.

### -Dialog

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | `Dialog`: named, mandatory |
| Pipeline input | no |
| Default | none |

The number of one of the system's dialogs, as [Get-GuiDialog](Get-GuiDialog.md) writes it in `Id`, to close with the choice `-Value` gives.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Widget`: position 1, mandatory |
| Pipeline input | no |
| Default | none |

The widget's name, compared without regard to case.

### -Event

| | |
|---|---|
| Type | `PwrsGui.EventKind` |
| Parameter sets | `Widget`: position 2, mandatory |
| Pipeline input | no |
| Values | `Click`, `Change`, `Select`, `Activate`, `Submit`, `Expand`, `Drop`, `Key`, `PointerDown`, `PointerUp`, `PointerMove`, `Wheel`, `DoubleClick`, `PointerEnter`, `PointerLeave`, `DragStart`, `DragMove`, `DragEnd`, `Peek`; `Close`, `Shortcut` and `FileChange` are refused, as no widget raises them |
| Default | none |

What happens to the widget, as the table above describes.

### -Value

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `Widget`, `Dialog`: position 3 |
| Pipeline input | no |
| Default | none; with `-Dialog`, a cancel |

The event's value. Booleans, numbers, strings, arrays and hashtables arrive as they were given; a date travels as its ISO 8601 text and a time span in .NET's constant format. With `-Dialog`, the choice made in the dialog: a message's button by name, or a file or folder dialog's path or paths.

### -Shortcut

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Shortcut`: named, mandatory |
| Pipeline input | no |
| Default | none |

A shortcut to press, written as `Ctrl+S`.

### -Drop

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `Drop`: named, mandatory |
| Pipeline input | no |
| Default | none |

Files to drop on the window, as a list of their paths, or in an HTML window text. An egui window takes files alone, so text given for one is refused.

### -Input

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `Input`: named, mandatory |
| Pipeline input | no |
| Default | none |

The steps of a person's input to give the window, a hashtable or a list of them, in order, each as the table in the description writes it.

### -Timeout

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | `Input`: named |
| Pipeline input | no |
| Default | 10 |

How many seconds each step of `-Input` has to be taken and seen once it begins, zero or more; a wait step has its own seconds besides. A step that runs out of its time fails the command with `GuiTimeout`, naming the step.

### -PassThru

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Writes the value the widget took.

## Inputs

Any object with a `Window` or `Id` property, such as the `PwrsGui.Window` that `Get-GuiWindow` writes, by property name to `-Window`.

## Outputs

With `-PassThru`, the value the widget took: nothing for a click, a row number for an activation, an array of row numbers (`System.Int64`) for a selection, and a value of the input's own type for a change. Without it, nothing. With `-Dialog`, nothing: the dialog's command writes the choice. With `-Input`, nothing: the window's events carry what the input did.

A dialog started in a second runspace, and found by its title once it has opened:

```powershell
$dialog = [powershell]::Create().AddScript({
    Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production', 'lab' -Title 'Deploy'
})
$running = $dialog.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($window = Get-GuiWindow -Title 'Deploy') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
$window.Names
```

```text
Choice
Ok
Cancel
```

What a selection took:

```powershell
$took = Invoke-GuiTestEvent -Window $window.Id -Name Choice -Event Select -Value 2 -PassThru
$took
$took.GetType().FullName
$took[0].GetType().FullName
```

```text
2
System.Object[]
System.Int64
```

## Examples

The examples from the command's help.

Select a row of the list by its number:

```powershell
Invoke-GuiTestEvent -Window $window.Id -Name Choice -Event Select -Value 1
(Get-GuiSnapshot -Window $window.Id).State.Choice
```

```text
1
```

Press OK on a window piped from `Get-GuiWindow`; the dialog writes the choice selected:

```powershell
Get-GuiWindow -Title Deploy | Invoke-GuiTestEvent -Name Ok -Event Click
$dialog.EndInvoke($running)
$dialog.Dispose()
```

```text
production
```

A window's own shortcut and a drop on the window, each read from the window's queue:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Editor' -OnShortcut @{ 'Ctrl+S' = { } } -OnDrop { } { TextBox -Name Body -Multiline }
Invoke-GuiTestEvent -Window $w.Id -Shortcut 'ctrl+s'
$pressed = Wait-GuiEvent -Window $w.Id -Timeout 20
'{0} {1}' -f $pressed.Kind, $pressed.Value
Invoke-GuiTestEvent -Window $w.Id -Drop @('today.log')
$dropped = Wait-GuiEvent -Window $w.Id -Timeout 20
'{0} {1}' -f $dropped.Kind, ($dropped.Value -join ', ')
Close-GuiWindow -Window $w.Id
```

```text
Shortcut Ctrl+S
Drop today.log
```

Press a button of a message box another runspace waits on, found by its title; its command writes the button:

```powershell
$asking = [powershell]::Create().AddScript({ Show-GuiMessage 'Restart now?' -Title 'Restart' -Buttons YesNo })
$running = $asking.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($open = Get-GuiDialog -Title 'Restart') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
Invoke-GuiTestEvent -Dialog $open.Id -Value Yes
$asking.EndInvoke($running)
$asking.Dispose()
```

```text
Yes
```

A disabled button takes no click, from a test either:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Locked' { Button 'Deploy' -Name Deploy -Enabled:$false }
Invoke-GuiTestEvent -Window $w.Id -Name Deploy -Event Click -ErrorVariable refused -ErrorAction SilentlyContinue
$refused.FullyQualifiedErrorId
Close-GuiWindow -Window $w.Id
```

```text
GuiRefused,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand
```

A person's click 3 points beside a button, which reaches it, as a press within 5 points of a widget does:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Reach' { Button 'Deploy' -Name Deploy }
$b = (Get-GuiSnapshot -Window $w.Id).Bounds.Deploy
$x = $b.X + $b.Width + 3
$y = $b.Y + $b.Height / 2
Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $x; Y = $y }, @{ Release = 'Left'; X = $x; Y = $y }
(Wait-GuiEvent -Window $w.Id -Timeout 20).Kind
Close-GuiWindow -Window $w.Id
```

```text
Click
```

## Errors

Before any window has opened in the process there is no renderer for the command to reach, which is `GuiNoSession`, here in a new PowerShell process of the same host:

```powershell
& (Get-Process -Id $PID).Path -NoProfile -Command {
    Invoke-GuiTestEvent -Window 1 -Name Ok -Event Click -ErrorVariable failed -ErrorAction SilentlyContinue
    $failed.FullyQualifiedErrorId
    $failed.Exception.Message
}
```

```text
GuiNoSession,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand
window 1 would be drawn by the Egui renderer, and none is running in this process, so there is no renderer to send it to
```

A widget the window does not have is `GuiRefused`, naming the widgets it has:

<!-- varies: the window's number -->
```powershell
$shown = [powershell]::Create().AddScript({ Read-GuiChoice -Choice 'one', 'two' })
$waiting = $shown.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($open = Get-GuiWindow -Title 'Choose') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
Invoke-GuiTestEvent -Window $open.Id -Name Missing -Event Click -ErrorVariable failed -ErrorAction SilentlyContinue
$failed.FullyQualifiedErrorId
$failed.Exception.Message
Invoke-GuiTestEvent -Window $open.Id -Name Cancel -Event Click
$shown.EndInvoke($waiting)
$shown.Dispose()
```

```text
GuiRefused,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand
the GUI renderer refused: window 22 has no widget named Missing; its widgets are Choice, Ok, Cancel
```

| Error ID | When |
|---|---|
| `GuiNoSession` | no window of that renderer has opened in the process, so there is no renderer for the command to reach |
| `GuiRefused` | the window has no such widget, the widget raises no such event, the value is one the widget cannot take, or the widget is disabled; for `-Input`, a step the window cannot take, as the description lists |
| `GuiNotAWidgetEvent` | `-Event Close` or `-Event FileChange`, the window's own events; `Close-GuiWindow` closes a window, and a write to a file `Show-GuiWindow -WatchPath` names raises `FileChange` |
| `GuiBadShortcut` | `-Shortcut` is not written as a shortcut |
| `GuiBadStep` | a step of `-Input` is not one, or `-Input` has none |
| `GuiTimeout` | a step of `-Input` was not taken and seen in its time, which the error names with the step |
| `GuiBadTimeout` | `-Timeout` is below zero or not a number |
| `GuiRendererGone` | the renderer went away while the command waited for its reply |

`GuiNoSession`, `GuiRefused` and `GuiTimeout` are non-terminating, the others terminating. The [Error Reference](Error-Reference.md) describes each.
