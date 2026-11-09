---
title: New-GuiNumberBox
weight: 41
---

Describes a number field, limited to a range when one is given, that a person drags, steps or types a number into. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiNumberBox`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The changes and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiNumberBox -Syntax
```

```text
New-GuiNumberBox [[-Value] <double>] [-Minimum <double>] [-Maximum <double>] [-Step <double>] [-Enabled] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiNumberBox` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as egui's drag value, in both renderers, in a text field's look, as WinUI's NumberBox shares TextBox's: a field showing `-Value`, limited to `-Minimum` to `-Maximum` when they are given. Dragging it right or up raises the number by `-Step` for each point, and left or down lowers it. Clicked or tabbed to, it becomes a field to type a number in, which Enter or leaving the field takes and Escape drops. While it has the focus, the Up and Down keys step the number by `-Step`. In a `-Content` block `NumberBox` stands for `New-GuiNumberBox`. A change raises `Change`, which `-OnChange` acts on, and `State` has the number under the widget's name, a `[double]` in either renderer.

[Update-GuiWidget](Update-GuiWidget.md) sets `Value`, `Minimum`, `Maximum`, `Step` and `Enabled`. A new range with no `Value` moves the number into it, to the nearer end, and leaves a number already inside it where it is. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) refuses a number outside the range, as the field does. A field that has the focus uses the keys it edits with itself; `-OnKey` gets the others.

## Parameters

### -Value

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | 0, or the nearer of `-Minimum` and `-Maximum` when 0 is outside them |

The number when the window opens.

### -Minimum

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | no limit |

The smallest number the field takes.

### -Maximum

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | no limit |

The largest number the field takes.

### -Step

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 1 |

How far a drag of one point, or a press of Up or Down, changes the number; above 0.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the number can be changed; `-Enabled:$false` makes it a field that cannot.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the number changes, with the event as its argument and as `$_`; its `Value` is the number.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the field, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the field does not edit with is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the field.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `NumberBox` and its `Properties` with `Value`, and `Minimum`, `Maximum`, `Step` and `Enabled` when they are given.

## Examples

A number a person changes before pressing OK:

<!-- person: $w = Wait-Window 'Copies'; Invoke-GuiTestEvent -Window $w.Id -Name Copies -Event Change -Value 7; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Copies' {
    NumberBox 3 -Minimum 1 -Maximum 10 -Name Copies
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.State.Copies
$result.State.Copies.GetType().Name
```

```text
7
Double
```

A smaller range moving the number into it, and a number outside the range refused:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Copies' { NumberBox 8 -Maximum 10 -Name Copies }
Update-GuiWidget -Window $w.Id -Name Copies -Property @{ Maximum = 5 }
(Get-GuiSnapshot -Window $w.Id).State.Copies
Invoke-GuiTestEvent -Window $w.Id -Name Copies -Event Change -Value 9 -ErrorAction SilentlyContinue -ErrorVariable refused
$refused.FullyQualifiedErrorId
Close-GuiWindow -Window $w.Id
```

```text
5
GuiRefused,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand
```

A number outside its range is refused as the field is made:

```powershell
try { New-GuiNumberBox 20 -Maximum 10 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadNumber,Pwrs.Modules.PWRS_GUI.NewGuiNumberBoxCommand
a NumberBox's Value is 20, above its Maximum 10
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadNumber` | a number is not finite, `-Minimum` is above `-Maximum`, `-Step` is not above 0, or `-Value` is outside the range, as the field is made or updated |

Described in the [Error Reference](Error-Reference.md).
