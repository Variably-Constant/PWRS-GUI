---
title: New-GuiSeparator
weight: 29
---

Describes a rule between widgets, or between the items of a menu. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiSeparator`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiSeparator -Syntax
```

```text
New-GuiSeparator [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiSeparator` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a line across the container it is in, between the widgets before and after it. Inside a [Menu](New-GuiMenu.md) or a [ContextMenu](New-GuiContextMenu.md) it is the line between groups of items. In a `-Content` block `Separator` stands for `New-GuiSeparator`. It has no properties, takes no drops and raises no event.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands use for the rule.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Separator`.

## Examples

A rule between two texts, and one between a menu's groups:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Ruled' {
    MenuBar { Menu 'File' { MenuItem 'Open' -Name Open; Separator; MenuItem 'Quit' -Name Quit } }
    Text 'Above'
    Separator
    Text 'Below'
}
(Get-GuiSnapshot -Window $w.Id).Tree.Children | ForEach-Object { $_.Kind }
Close-GuiWindow -Window $w.Id
```

```text
MenuBar
Text
Separator
Text
```

## Errors

None of its own.
