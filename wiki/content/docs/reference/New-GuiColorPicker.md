---
title: New-GuiColorPicker
weight: 45
---

Describes a color chooser. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiColorPicker`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The choices and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiColorPicker -Syntax
```

```text
New-GuiColorPicker [[-Color] <string>] [-Enabled] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiColorPicker` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a swatch of `-Color`, which opens a chooser when a person clicks it. In a `-Content` block `ColorPicker` stands for `New-GuiColorPicker`. A color is written `#rrggbb`, in either case, and comes back in lower case. A choice raises `Change` with the color, which `-OnChange` acts on, and `State` has the color under the widget's name.

[Update-GuiWidget](Update-GuiWidget.md) sets `Color` and `Enabled`. A chooser that has the focus uses Space and Enter itself; `-OnKey` gets the other keys.

## Parameters

### -Color

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | `#000000` |

The color when the window opens, written `#rrggbb` in either case.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the color can be changed; `-Enabled:$false` makes it a chooser that cannot.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person chooses a color, with the event as its argument and as `$_`; its `Value` is the color.

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

`PwrsGui.Widget`, its `Kind` `ColorPicker` and its `Properties` with `Color` in lower case, and `Enabled` when it is given.

## Examples

A color a person chooses before pressing OK. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Accent'; Invoke-GuiTestEvent -Window $w.Id -Name Accent -Event Change -Value '#ff8800'; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Accent' -HandlerOutput Pipeline {
    ColorPicker '#336699' -Name Accent -OnChange { "chose $($_.Value)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
$out[1].State.Accent
```

```text
chose #ff8800
#ff8800
```

A color given in capitals comes back in lower case, and a color is written `#rrggbb`:

```powershell
(New-GuiColorPicker '#AABBCC').Properties.Color
try { New-GuiColorPicker 'red' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
#aabbcc
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiColorPickerCommand
a ColorPicker's -Color is red; it must be a color written #rrggbb
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | `-Color` is not written `#rrggbb`, as the chooser is made or updated |

Described in the [Error Reference](Error-Reference.md).
