---
title: Show-GuiProgress
weight: 2
---

Opens a window showing an activity's progress, and writes it for [Set-GuiProgress](Set-GuiProgress.md) to update. Source: `crates/pwrs-gui/src/progress.rs`, `ShowGuiProgress` and `GuiProgressWindow`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its name for unsigned integer types, except where its output is shown too.

## Syntax

```powershell
Get-Command Show-GuiProgress -Syntax
```

```text
Show-GuiProgress [-Activity] <string> [[-Status] <string>] [-PercentComplete <int>] [-Ring <Object>] [-Title <string>] [<CommonParameters>]
```

## Description

`Show-GuiProgress` opens a window and writes it as a `PwrsGui.ProgressWindow` once it has opened. The window is titled with `-Activity`, or `-Title`. It shows the activity as its main line, and under it a [ProgressBar](New-GuiProgressBar.md): `-Status` with the percentage done at its right, over a thin bar filled to `-PercentComplete`. The window is 380 points across and as high as they are, its widgets inside a dialog's 12-point margin. [Set-GuiProgress](Set-GuiProgress.md) changes the activity, the status and the bar, and its `-Completed` closes the window and returns once it has closed.

The window stays open until `Set-GuiProgress -Completed` closes it or the person does. Its `State` is `Open` until then and `Closed` after, read from the session each time the property is read. A closed window takes no update: `Set-GuiProgress` on it does nothing, so the script goes on. A script that should stop when the person closes the window reads `State`.

With `-Ring`, a SubEtha ring that another process writes fills the bar, as a [ProgressBar](New-GuiProgressBar.md)'s `-Ring` does. Each item is a JSON object of `Value`, the bar's value from 0 to 100, and `Text`, the status. The renderer reads the items on a thread of its own, with no PowerShell call per item.

The session's renderer draws the window, as [Set-GuiOption](Set-GuiOption.md) says, and [Get-GuiWindow](Get-GuiWindow.md) lists it as it lists any window. Its widgets are named `Activity` and `Progress`, the status being the progress bar's `Text`.

## Parameters

### -Activity

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |

What is in progress, the window's main line.

### -Status

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | none: no status |

The activity's status, shown over the bar with the percentage done at its right.

### -PercentComplete

| | |
|---|---|
| Type | `System.Int32` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: an empty bar |
| Refused with | `GuiBadPercent` outside 0 to 100 |

How much of the activity is done, from 0 to 100.

### -Ring

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A SubEtha ring, the object `New-SubEthaRing`, `New-SubEthaLocaleRing` or `New-SubEthaBroadcastRing` wrote, whose items fill the bar.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the activity |

The window's title, which [Get-GuiWindow](Get-GuiWindow.md) matches its `-Title` against.

## Inputs

None.

## Outputs

`PwrsGui.ProgressWindow`:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the window's number, which `Set-GuiProgress` and the window commands take |
| `Title` | `System.String` | the window's title |
| `State` | `System.String` | `Open` or `Closed`, read from the session each time the property is read |

## Examples

The examples from the command's help.

Open the window and move the bar half way:

<!-- keep: show-guiprogress-half.png images/show-guiprogress-half.png -->
<!-- person: $w = Wait-Window 'Copying files'; Wait-Property $w Progress value 50; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'show-guiprogress-half.png') -->
<!-- varies: the window's number -->
```powershell
$p = Show-GuiProgress 'Copying files' -Status 'Starting' -PercentComplete 0
$p
Set-GuiProgress $p -Status 'Half way' -PercentComplete 50
```

```text
Id Title
-- -----
 1 Copying files
```

The window half way, as [Get-GuiSnapshot](Get-GuiSnapshot.md) `-Pixels` captured it in that run:

![The Copying files window with the main line Copying files, the status Half way below it, and a bar filled half way](/images/show-guiprogress-half.png)

Close it; its `State` follows:

```powershell
Set-GuiProgress $p -Completed
$p.State
```

```text
Closed
```

## Errors

A percentage outside 0 to 100 is refused before the window opens:

```powershell
try { Show-GuiProgress 'Copying files' -PercentComplete 150 } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
```

```text
GuiBadPercent,Pwrs.Modules.PWRS_GUI.ShowGuiProgressCommand: -PercentComplete is 150; it runs from 0 to 100
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed` and `GuiRendererGone`.
