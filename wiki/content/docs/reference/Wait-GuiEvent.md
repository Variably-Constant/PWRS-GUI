---
title: Wait-GuiEvent
weight: 13
---

Waits for the next event of a window `Show-GuiWindow -NoWait` opened, and writes it. Source: `crates/pwrs-gui/src/window.rs`, `WaitGuiEvent`, and the window's event log, `EventLog`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be.

## Syntax

```powershell
Get-Command Wait-GuiEvent -Syntax
```

```text
Wait-GuiEvent [-Window] <ulong> [[-Name] <string>] [-Timeout <double>] [<CommonParameters>]
```

## Description

When [Show-GuiWindow](Show-GuiWindow.md) opens a window with `-NoWait`, every event its widgets raise waits in order for `Wait-GuiEvent`, whether or not a handler ran for it, as does a `FileChange` for each change to a file its `-WatchPath` names. `Wait-GuiEvent` takes them one at a time, oldest first: it writes the next one at once when one is waiting, and otherwise waits for it. A file has one change waiting at a time, and the next is queued once that one is taken. The window's `Close` comes last; once it is taken, the window is forgotten and `Wait-GuiEvent` on it is refused.

With `-Name`, only that widget's events are taken, or a watched file's changes, named by the file's full path, and the window's `Close`; the others stay for a later `Wait-GuiEvent`. With `-Timeout`, the command writes nothing when no event comes in that many seconds. Ctrl+C ends the wait and writes nothing.

The event is written as a handler is given it: the window, the widget's name, the kind of event, its value, and every named input's value at that moment.

## Parameters

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | yes, by property name, as `Window` or `Id` |
| Default | none |

The window's number, as `Show-GuiWindow -NoWait` writes it.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | none: every widget's events |

Only this widget's events, or with a watched file's full path that file's changes, and the window's `Close`.

### -Timeout

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: no limit |

How many seconds to wait, zero or more; with no event by then the command writes nothing.

## Inputs

An object with a `Window` or `Id` property, such as the window `Show-GuiWindow -NoWait` writes.

## Outputs

`PwrsGui.WidgetEvent`, one, or nothing when `-Timeout` passes or Ctrl+C ends the wait:

| Property | Type | What it gives |
|---|---|---|
| `Window` | `System.UInt64` | the window's number |
| `Name` | `System.String` | the widget's name; for `FileChange`, the watched file's full path; null for the window's other events, such as `Close` |
| `Kind` | `PwrsGui.EventKind` | `Click`, `Change`, `Select`, `Activate`, `Submit`, `Expand`, `Drop`, `Key`, `PointerDown`, `PointerUp`, `PointerMove`, `Wheel`, `DoubleClick`, `PointerEnter`, `PointerLeave`, `DragStart`, `DragMove`, `DragEnd`, `Shortcut`, `FileChange` or `Close` |
| `Value` | `System.Object` | what the event gives, as each widget's page says: a check box's flag, a field's text, a list's items, the paths dropped, where a GpuSurface's or a Canvas's pointer is and the canvas's item there; for `FileChange`, the file's length in bytes, null while it is gone; for `Close`, the window's result, such as `Closed`; null for a button's `Click` |
| `State` | `System.Management.Automation.PSCustomObject` | every named input's value at that moment, as a property under its name; its type name is `PwrsGui.WindowState` |

## Examples

Wait for a click:

<!-- person: $w = Wait-Window 'Ready'; Invoke-GuiTestEvent -Window $w.Id -Name Go -Event Click -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'Ready' { Button 'Go' -Name Go }
$e = Wait-GuiEvent $w.Id -Timeout 20
$e | Select-Object Name, Kind
Close-GuiWindow -Window $w.Id
```

```text
Name  Kind
----  ----
Go   Click
```

```powershell
$e | Get-Member -MemberType Properties
```

```text
   TypeName: PwrsGui.WidgetEvent

Name   MemberType Definition
----   ---------- ----------
Kind   Property   PwrsGui.EventKind Kind {get;set;}
Name   Property   string Name {get;set;}
State  Property   System.Object State {get;set;}
Value  Property   System.Object Value {get;set;}
Window Property   ulong Window {get;set;}
```

Events wait until they are taken, so a script can do its work and read them afterward. The window's `Close` comes last:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Fast -Event Change -Value $true; Invoke-GuiTestEvent -Window $w.Id -Name Size -Event Change -Value 1 -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'Choose' {
    CheckBox 'Fast' -Name Fast
    ComboBox 'Small', 'Large' -Name Size -Selected Small
}
Start-Sleep -Seconds 2
Close-GuiWindow -Window $w.Id
do {
    $e = Wait-GuiEvent $w.Id -Timeout 5
    if ($e) { '{0} {1} {2}' -f $e.Name, $e.Kind, $e.Value }
} until ($null -eq $e -or $e.Kind -eq 'Close')
```

```text
Fast Change True
Size Change Large
 Close Closed
```

With `-Timeout`, nothing is written when no event comes:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Quiet' { Text 'Nothing to click' }
$null -eq (Wait-GuiEvent $w.Id -Timeout 0.5)
Close-GuiWindow -Window $w.Id
```

```text
True
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoWindow` | no window described in PowerShell has that number, or its `Close` has been taken |
| `GuiNotQueued` | the window was opened without `-NoWait`, so the command that opened it reads its events |
| `GuiBadTimeout` | `-Timeout` is below zero or not a number |

Each is described in the [Error Reference](Error-Reference.md).
