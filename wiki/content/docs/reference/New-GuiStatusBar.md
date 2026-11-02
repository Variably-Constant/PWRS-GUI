---
title: New-GuiStatusBar
weight: 55
---

Describes a bar along the bottom of the window, with widgets side by side. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiStatusBar`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiStatusBar -Syntax
```

```text
New-GuiStatusBar [[-Content] <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiStatusBar` writes a `PwrsGui.Widget` with the widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws side by side along the window's bottom, with the window's other widgets above it and a [ButtonBar](New-GuiButtonBar.md), when the window has one, between them, wherever in the block the bar is written. It is one of the window's own widgets, at the block's first level, and Show-GuiWindow refuses one inside another widget. In a `-Content` block `StatusBar` stands for `New-GuiStatusBar`; a [MenuBar](New-GuiMenuBar.md) is its counterpart along the top.

The block runs once, as the window is described, and must write only widgets; `$null` is passed over. The widgets inside are updated by their own names.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty bar |

The block that writes the bar's widgets.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the bar where no widget inside it takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the bar.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `StatusBar` and its `Children` the widgets the block wrote.

## Examples

A bar whose text a script changes:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Editor' {
    Text 'The body of the window'
    StatusBar { Text 'Ready' -Name Status; Spacer 24; Text 'UTF-8' }
}
Update-GuiWidget -Window $w.Id -Name Status -Property @{ Text = 'Saved' }
$bar = (Get-GuiSnapshot -Window $w.Id).Tree.Children | Where-Object { $_.Kind -eq 'StatusBar' }
$bar.Children[0].Properties.text
Close-GuiWindow -Window $w.Id
```

```text
Saved
```

A bar inside another widget opens no window:

```powershell
try { Show-GuiWindow { Column { StatusBar { Text 'Ready' } } } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNotAtTop,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
a StatusBar stands among the window's own widgets, not inside another
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotAtTop` | Show-GuiWindow is given a StatusBar inside another widget |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
