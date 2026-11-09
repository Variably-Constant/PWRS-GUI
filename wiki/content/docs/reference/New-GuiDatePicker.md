---
title: New-GuiDatePicker
weight: 46
---

Describes a date and time chooser, to the minute, limited to a range when one is given. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiDatePicker`; the minutes as both renderers read them, `crates/pwrs-gui-proto/src/date.rs`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The choices and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiDatePicker -Syntax
```

```text
New-GuiDatePicker [[-Value] <datetime>] [-Minimum <datetime>] [-Maximum <datetime>] [-Enabled] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiDatePicker` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a chooser of a date and a time. Both renderers draw it alike: a field naming the day, or `Choose a date` until one is chosen, in a text field's look with a drop-down's triangle at its right, which opens a calendar of a month, weeks from Monday. Once a day is chosen, its hour and minute appear beside it, in the same look. Under a stretching `-Align`, the day takes the room the hour and minute leave. The chooser has a date and time to the minute: `-Value`'s when it is given, and otherwise none until a person chooses one. With `-Minimum` and `-Maximum` the days outside the range are disabled, and a minute chosen outside it moves to the nearer end. In a `-Content` block `DatePicker` stands for `New-GuiDatePicker`.

A choice raises `Change` with a `[datetime]` of no zone, which `-OnChange` acts on, and `State` has the same under the widget's name, `$null` until one is chosen. Seconds given are dropped. [Update-GuiWidget](Update-GuiWidget.md) sets `Value`, `Minimum`, `Maximum` and `Enabled`; a new range with no `Value` moves the date into it. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) refuses a date outside the range. A chooser that has the focus uses Space, Enter and Escape itself; `-OnKey` gets the other keys.

## Parameters

### -Value

| | |
|---|---|
| Type | `System.DateTime` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none chosen |

The date and time when the window opens, to the minute.

### -Minimum

| | |
|---|---|
| Type | `System.DateTime` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | no limit |

The earliest date and time the chooser takes.

### -Maximum

| | |
|---|---|
| Type | `System.DateTime` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | no limit |

The latest date and time the chooser takes.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the date can be changed; `-Enabled:$false` makes it a chooser that cannot.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person chooses a date or time, with the event as its argument and as `$_`; its `Value` is the date and time, a `[datetime]`.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the chooser, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the chooser does not use itself is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the chooser.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `DatePicker` and its `Properties` with `Value`, `Minimum`, `Maximum` and `Enabled` when they are given.

## Examples

A time two days on that a person chooses, its seconds dropped, read as how far it is from the start. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Schedule'; Invoke-GuiTestEvent -Window $w.Id -Name When -Event Change -Value (Get-Date).Date.AddDays(2).AddHours(14).AddMinutes(30).AddSeconds(45); Invoke-GuiTestEvent -Window $w.Id -Name Done -Event Click -->
```powershell
$start = (Get-Date).Date.AddHours(9)
$out = Show-GuiWindow -Title 'Schedule' -HandlerOutput Pipeline {
    DatePicker $start -Minimum $start -Maximum $start.AddDays(30) -Name When -OnChange { "chose a $($_.Value.GetType().Name)" }
    ButtonBar { Button 'Done' -Name Done -Default -Result Done }
}
$out[0]
($out[1].State.When - $start).ToString()
$out[1].State.When.Kind
```

```text
chose a DateTime
2.05:30:00
Unspecified
```

A chooser given no date has none:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Unset' { DatePicker -Name When }
$null -eq (Get-GuiSnapshot -Window $w.Id).State.When
Close-GuiWindow -Window $w.Id
```

```text
True
```

A date outside the range, and a range that ends before it starts:

```powershell
try { New-GuiDatePicker (Get-Date).AddDays(-1) -Minimum (Get-Date) } catch { $_.FullyQualifiedErrorId }
try { New-GuiDatePicker -Minimum (Get-Date).AddDays(1) -Maximum (Get-Date) } catch { $_.FullyQualifiedErrorId }
```

```text
GuiBadDate,Pwrs.Modules.PWRS_GUI.NewGuiDatePickerCommand
GuiBadDate,Pwrs.Modules.PWRS_GUI.NewGuiDatePickerCommand
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadDate` | `-Minimum` is after `-Maximum`, or `-Value` is outside the range, as the chooser is made or updated |

Described in the [Error Reference](Error-Reference.md).
