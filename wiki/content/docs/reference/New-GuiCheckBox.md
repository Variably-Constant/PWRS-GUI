---
title: New-GuiCheckBox
weight: 39
---

Describes a check box with its text, checked or not. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiCheckBox`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The changes and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiCheckBox -Syntax
```

```text
New-GuiCheckBox [-Text] <string> [-Checked] [-Enabled] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiCheckBox` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a box beside `-Text`, checked when the window opens with `-Checked`. In a `-Content` block `CheckBox` stands for `New-GuiCheckBox`. A person checking or clearing it raises `Change`, whose `Value` is `[bool]`, which `-OnChange` acts on, and `State` has the same under the widget's name.

A check box with `-Enabled:$false` is drawn dimmed and takes no change, and [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) refuses to change it. A check box that has the focus uses Space and Enter itself; `-OnKey` gets the other keys. [Update-GuiWidget](Update-GuiWidget.md) sets `Text`, `Checked` and `Enabled`.

## Parameters

### -Text

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none; an empty string is allowed |

The text beside the box.

### -Checked

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | cleared |

Checks the box when the window opens.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the box can be changed; `-Enabled:$false` makes it one that cannot.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the box is checked or cleared, with the event as its argument and as `$_`; its `Value` is whether the box is checked.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the box, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the box does not use itself is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the box.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `CheckBox` and its `Properties` with `Text` and `Checked`, and `Enabled` when it is given.

## Examples

A box a person clears before pressing OK. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Run'; Invoke-GuiTestEvent -Window $w.Id -Name DryRun -Event Change -Value $false; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Run' -HandlerOutput Pipeline {
    CheckBox 'Dry run' -Name DryRun -Checked -OnChange { "dry run is now $($_.Value)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
$out[1].State.DryRun
```

```text
dry run is now False
False
```

A box a script checks while the window is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Options' { CheckBox 'Verbose' -Name Verbose }
(Get-GuiSnapshot -Window $w.Id).State.Verbose
Update-GuiWidget -Window $w.Id -Name Verbose -Property @{ Checked = $true }
(Get-GuiSnapshot -Window $w.Id).State.Verbose
Close-GuiWindow -Window $w.Id
```

```text
False
True
```

## Errors

None of its own; [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) writes `GuiRefused` for a change to a disabled box.
