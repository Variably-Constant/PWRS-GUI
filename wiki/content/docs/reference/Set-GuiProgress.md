---
title: Set-GuiProgress
weight: 2
---

Updates a progress window [Show-GuiProgress](Show-GuiProgress.md) opened, and closes it. Source: `crates/pwrs-gui/src/progress.rs`, `SetGuiProgress`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same, except where its output is shown too.

## Syntax

```powershell
Get-Command Set-GuiProgress -Syntax
```

```text
Set-GuiProgress [-Window] <Object> [[-Status] <string>] [-Activity <string>] [-PercentComplete <int>] [-Completed] [<CommonParameters>]
```

## Description

`Set-GuiProgress` changes the activity, the status and how much is done in a progress window, and `-Completed` closes it and returns once it has closed, so the window's `State` reads `Closed` from then on. It takes the window [Show-GuiProgress](Show-GuiProgress.md) wrote, by position or from the pipeline, or its number.

A window that has closed, by `-Completed` or by the person, takes no update. `Set-GuiProgress` then does nothing and raises nothing, so a long script goes on after the person has closed its progress window. A `-PercentComplete` outside 0 to 100 is still refused.

## Parameters

### -Window

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | by value |
| Refused with | `GuiNoWindow` for an object that is neither a window's number nor has an `Id` property |

The progress window, or its number.

### -Status

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | none: the status stays as it was |

The activity's status now.

### -Activity

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the activity stays as it was |

What is in progress now.

### -PercentComplete

| | |
|---|---|
| Type | `System.Int32` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the bar stays as it was |
| Refused with | `GuiBadPercent` outside 0 to 100 |

How much of the activity is done now, from 0 to 100.

### -Completed

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the window stays open |

Closes the window, returning once it has closed.

## Inputs

`PwrsGui.ProgressWindow`, by value to `-Window`.

## Outputs

None.

## Examples

The examples from the command's help, in a loop of five steps:

```powershell
$p = Show-GuiProgress 'Copying files'
$total = 5
foreach ($n in 1..$total) {
    Set-GuiProgress $p -Status "Copied $n of $total" -PercentComplete (100 * $n / $total)
}
$p.State
$p | Set-GuiProgress -Completed
$p.State
```

```text
Open
Closed
```

A window that has closed, here by [Close-GuiWindow](Close-GuiWindow.md) as a person's close button would close it, takes no update, and the script goes on. Close-GuiWindow returns once it has sent the close, so the example first takes the window's `Close` event with [Wait-GuiEvent](Wait-GuiEvent.md), which comes once the window has closed:

```powershell
$p = Show-GuiProgress 'Indexing'
Close-GuiWindow -Window $p.Id
$null = Wait-GuiEvent -Window $p.Id
Set-GuiProgress $p -Status 'Still going' -PercentComplete 40
$p.State
```

```text
Closed
```
