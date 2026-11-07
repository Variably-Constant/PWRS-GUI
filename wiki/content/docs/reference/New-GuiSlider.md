---
title: New-GuiSlider
weight: 42
---

Describes a slider from a minimum to a maximum, moving in steps. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiSlider`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The moves and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiSlider -Syntax
```

```text
New-GuiSlider [[-Value] <double>] [-Minimum <double>] [-Maximum <double>] [-Step <double>] [-Enabled] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiSlider` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a slider from `-Minimum` to `-Maximum`, 0 to 100 when they are not given, at `-Value`, moving in steps of `-Step`. In a `-Content` block `Slider` stands for `New-GuiSlider`. A move raises `Change` as the slider moves, which `-OnChange` acts on, and `State` has the number under the widget's name, a `[double]` in either renderer.

[Update-GuiWidget](Update-GuiWidget.md) sets `Value`, `Minimum`, `Maximum`, `Step` and `Enabled`; a new range with no `Value` moves the number into it, to the nearer end. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) refuses a number outside the range. A slider that has the focus uses its arrow keys, Home, End and the page keys itself; `-OnKey` gets the others.

## Parameters

### -Value

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | `-Minimum` |

The number when the window opens.

### -Minimum

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0 |

The number at the slider's start.

### -Maximum

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 100 |

The number at the slider's end.

### -Step

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 1 |

The steps the slider moves in, above 0.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the slider can be moved; `-Enabled:$false` makes it one that cannot.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs as the slider moves, with the event as its argument and as `$_`; its `Value` is the number.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the slider, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the slider does not use itself is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the slider.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Slider` and its `Properties` with `Value`, `Minimum` and `Maximum`, and `Step` and `Enabled` when they are given.

## Examples

A slider a person moves, whose handler shows where it is. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Volume'; Invoke-GuiTestEvent -Window $w.Id -Name Volume -Event Change -Value 55; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Volume' -HandlerOutput Pipeline {
    Slider 20 -Name Volume -OnChange { Update-GuiWidget Shown @{ Text = "$($_.Value) percent" }; "moved to $($_.Value)" }
    Text '20 percent' -Name Shown
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
$out[1].State.Volume
```

```text
moved to 55
55
```

The range a slider takes when none is given:

```powershell
(New-GuiSlider).Properties
```

```text
Name                           Value
----                           -----
Value                          0
Minimum                        0
Maximum                        100
```

A step is above 0:

```powershell
try { New-GuiSlider -Step 0 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadNumber,Pwrs.Modules.PWRS_GUI.NewGuiSliderCommand
a Slider's Step is 0; it must be above 0
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadNumber` | a number is not finite, `-Minimum` is above `-Maximum`, `-Step` is not above 0, or `-Value` is outside the range, as the slider is made or updated |

Described in the [Error Reference](Error-Reference.md).
