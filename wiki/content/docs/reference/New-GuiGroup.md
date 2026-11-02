---
title: New-GuiGroup
weight: 23
---

Describes widgets stacked in a frame, under a title when it is given. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiGroup`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiGroup -Syntax
```

```text
New-GuiGroup [[-Content] <scriptblock>] [-Title <string>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiGroup` writes a `PwrsGui.Widget` that contains the widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws stacked inside a frame, with `-Title` above them. In a `-Content` block `Group` stands for `New-GuiGroup`, ahead of PowerShell's own `group` alias for `Group-Object`, which a `-Content` block spells out in full.

The block runs once, as the window is described, and must write only widgets; `$null` is passed over. [Update-GuiWidget](Update-GuiWidget.md) changes the title.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty frame |

The block that writes the frame's widgets.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: no title |

The text above the widgets.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the frame where no widget inside it takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the frame.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Group`, its `Properties` with `Title` when it is given, and its `Children` the widgets the block wrote.

## Examples

Options in a titled frame, its title changed afterward:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Run' {
    Group -Title 'Options' -Name Options { CheckBox 'Dry run' -Name DryRun; CheckBox 'Verbose' -Name Verbose }
}
Update-GuiWidget -Window $w.Id -Name Options -Property @{ Title = 'Options for this run' }
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.title
Close-GuiWindow -Window $w.Id
```

```text
Options for this run
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotAWidget` | the block writes something other than a widget |

Described in the [Error Reference](Error-Reference.md).
