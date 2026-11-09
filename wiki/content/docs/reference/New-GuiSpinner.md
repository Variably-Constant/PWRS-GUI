---
title: New-GuiSpinner
weight: 34
---

Describes a spinner, which turns while it is shown, for work of unknown length. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiSpinner`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiSpinner -Syntax
```

```text
New-GuiSpinner [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiSpinner` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a turning ring, for work of unknown length; a [ProgressBar](New-GuiProgressBar.md) is for work whose steps can be counted. In a `-Content` block `Spinner` stands for `New-GuiSpinner`. It has no properties and raises no event: a script shows it while it works and closes the window when the work is done, or opens the window with a [Text](New-GuiText.md) beside it whose words it changes as the work goes on.

## Parameters

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the spinner, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the spinner.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Spinner`.

## Examples

A spinner beside a line of text while a job runs:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Working' { Row { Spinner -Name Busy; Text 'Waiting for the server' } }
$job = Start-Job { Start-Sleep -Seconds 1; 'the server replied' }
Receive-Job -Job $job -Wait
Close-GuiWindow -Window $w.Id
Remove-Job -Job $job
```

```text
the server replied
```

## Errors

None of its own.
