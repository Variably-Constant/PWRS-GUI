---
title: New-GuiRadioGroup
weight: 44
---

Describes option buttons, one per choice, of which one can be chosen. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiRadioGroup`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The choices and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiRadioGroup -Syntax
```

```text
New-GuiRadioGroup [-Choice] <string[]> [-Selected <string>] [-Enabled] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiRadioGroup` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as an option button for each of `-Choice`, of which one can be chosen; none is until a person picks one, unless `-Selected` names it. In a `-Content` block `RadioGroup` stands for `New-GuiRadioGroup`. A pick raises `Change` with the choice, which `-OnChange` acts on, and `State` has the choice as text under the widget's name, `$null` for none. [Get-GuiSnapshot](Get-GuiSnapshot.md) gives the choice's number instead, from 0, and [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) picks a choice by that number.

[Update-GuiWidget](Update-GuiWidget.md) sets `Choice`, `Selected` and `Enabled`; when the new choices still include the one chosen, it stays chosen. [Add-GuiItem](Add-GuiItem.md) and [Remove-GuiItem](Remove-GuiItem.md) add and remove choices. An option button that has the focus uses Space, Enter and the arrow keys itself, the arrow keys moving among the choices; `-OnKey` gets the other keys.

## Parameters

### -Choice

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The choices, as text, one button each.

### -Selected

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none chosen |

The choice chosen when the window opens, named by its text without regard to case.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the choice can be changed; `-Enabled:$false` makes it one that cannot.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person picks a choice, with the event as its argument and as `$_`; its `Value` is the choice.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the buttons, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the option buttons do not use themselves is pressed while one has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the buttons.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `RadioGroup` and its `Properties` with `Choice`, and `Selected` and `Enabled` when they are given.

## Examples

A size a person picks before pressing OK. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Size'; Invoke-GuiTestEvent -Window $w.Id -Name Size -Event Change -Value 2; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Size' -HandlerOutput Pipeline {
    RadioGroup 'Small', 'Medium', 'Large' -Name Size -Selected Medium -OnChange { "picked $($_.Value)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
$out[1].State.Size
```

```text
picked Large
Large
```

The choice chosen, by number, before and after a script chooses another:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Size' { RadioGroup 'Small', 'Medium', 'Large' -Name Size -Selected Medium }
(Get-GuiSnapshot -Window $w.Id).State.Size
Update-GuiWidget -Window $w.Id -Name Size -Property @{ Selected = 'small' }
(Get-GuiSnapshot -Window $w.Id).State.Size
Close-GuiWindow -Window $w.Id
```

```text
1
0
```

`-Selected` names one of the choices:

```powershell
try { New-GuiRadioGroup 'Small', 'Large' -Selected 'Huge' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiRadioGroupCommand
-Selected is Huge, which is not one of the choices: Small, Large
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | `-Selected` names no choice, as the buttons are made or updated |

Described in the [Error Reference](Error-Reference.md).
