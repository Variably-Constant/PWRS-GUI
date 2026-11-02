---
title: New-GuiTab
weight: 25
---

Describes one tab of a Tabs: its title, and the widgets its block writes, stacked, shown while the tab is chosen. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiTab`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiTab -Syntax
```

```text
New-GuiTab [-Title] <string> [[-Content] <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiTab` writes a `PwrsGui.Widget` that contains the widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws one under another while the tab is chosen, and its `-Title` among the titles of its [Tabs](New-GuiTabs.md). In a `-Content` block `Tab` stands for `New-GuiTab`. A Tab goes only in a Tabs, and Show-GuiWindow refuses one anywhere else.

The block runs once, as the window is described, and must write only widgets; `$null` is passed over. [Update-GuiWidget](Update-GuiWidget.md) changes the title.

## Parameters

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The tab's title, which its Tabs' `-Selected`, `State` and `Change` name it by.

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | none: an empty tab |

The block that writes the tab's widgets.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the tab where no widget inside it takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the tab.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Tab`, its `Properties` with `Title`, and its `Children` the widgets the block wrote.

## Examples

A tab whose title counts what it shows, changed while the window is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Jobs' {
    Tabs -Name Pages {
        Tab 'Queued' -Name QueuedTab { Text 'three jobs' }
        Tab 'Done' { Text 'none yet' }
    }
}
Update-GuiWidget -Window $w.Id -Name QueuedTab -Property @{ Title = 'Queued (3)' }
(Get-GuiSnapshot -Window $w.Id).Tree.Children | ForEach-Object { $_.Properties.title }
Close-GuiWindow -Window $w.Id
```

```text
Queued (3)
Done
```

A Tab outside a Tabs opens no window:

```powershell
try { Show-GuiWindow { Tab 'Lost' { Text 'x' } } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiTabOutsideTabs,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
the Tab Lost is not inside a Tabs; a Tab is one tab of a Tabs
```

## Errors

| Error ID | When |
|---|---|
| `GuiTabOutsideTabs` | Show-GuiWindow is given a Tab anywhere but in a Tabs |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
