---
title: New-GuiComboBox
weight: 43
---

Describes a drop-down of choices, or with `-Editable` a text field that offers them. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiComboBox`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The choices and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiComboBox -Syntax
```

```text
New-GuiComboBox [-Choice] <string[]> [-Selected <string>] [-Editable] [-Enabled] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiComboBox` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a drop-down of `-Choice`, a field in a text field's look with a triangle at its right, of which one can be chosen; none is until a person picks one, unless `-Selected` names it. In a `-Content` block `ComboBox` stands for `New-GuiComboBox`. A pick raises `Change` with the choice, which `-OnChange` acts on, and `State` has the choice as text under the widget's name, `$null` for none. [Get-GuiSnapshot](Get-GuiSnapshot.md) gives the choice's number instead, from 0, and [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) picks a choice by that number.

With `-Editable` it is a text field that offers the choices as a person types: `State` and `Change` give the text, `-Selected` gives any text it opens with, and [Update-GuiWidget](Update-GuiWidget.md) sets it with `Text` and refuses `Selected`; whether it is editable is set when the window opens. A drop-down that is not editable takes `Selected` and refuses `Text`.

[Update-GuiWidget](Update-GuiWidget.md) also sets `Choice` and `Enabled`; when the new choices still include the one chosen, it stays chosen. [Add-GuiItem](Add-GuiItem.md) and [Remove-GuiItem](Remove-GuiItem.md) add and remove choices. A drop-down that has the focus uses Space, Enter, its arrow keys, Home, End and Escape itself; `-OnKey` gets the others.

## Parameters

### -Choice

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The choices, as text.

### -Selected

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none chosen |

The choice chosen when the window opens, named by its text without regard to case; with `-Editable`, the text it opens with, any text.

### -Editable

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | a drop-down of the choices alone |

Lets a person type any text, offering the choices as they type. Taken when the window opens.

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

Runs when a person picks a choice, or with `-Editable` changes the text, with the event as its argument and as `$_`; its `Value` is the choice or the text.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the drop-down, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the drop-down does not use itself is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the drop-down.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `ComboBox` and its `Properties` with `Choice`, and `Selected` and `Enabled` when they are given; with `-Editable`, `Editable` and `Text` in place of `Selected`.

## Examples

A choice a person picks before pressing OK. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Target'; Invoke-GuiTestEvent -Window $w.Id -Name Target -Event Change -Value 1; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Target' -HandlerOutput Pipeline {
    ComboBox 'staging', 'production' -Name Target -OnChange { "picked $($_.Value)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
$out[1].State.Target
```

```text
picked production
production
```

An editable drop-down opening with text that is none of its choices, then set by a script:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Server' { ComboBox 'web01', 'web02' -Editable -Selected 'web03' -Name Server }
(Get-GuiSnapshot -Window $w.Id).State.Server
Update-GuiWidget -Window $w.Id -Name Server -Property @{ Text = 'web04' }
(Get-GuiSnapshot -Window $w.Id).State.Server
Close-GuiWindow -Window $w.Id
```

```text
web03
web04
```

A choice added while the window is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Pick' { ComboBox 'a', 'b' -Name Pick }
Add-GuiItem -Window $w.Id -Name Pick -Item 'c'
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.items
Close-GuiWindow -Window $w.Id
```

```text
a
b
c
```

`-Selected` names one of the choices:

```powershell
try { New-GuiComboBox 'staging', 'production' -Selected 'lab' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiComboBoxCommand
-Selected is lab, which is not one of the choices: staging, production
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | `-Selected` names no choice, or `Update-GuiWidget` sets `Selected` on an editable drop-down or `Text` on one that is not |
| `GuiOpeningOnly` | `Update-GuiWidget` sets `Editable`, which is taken when the window opens |

Each is described in the [Error Reference](Error-Reference.md).
