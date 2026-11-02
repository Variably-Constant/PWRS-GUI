---
title: Get-GuiWindow
weight: 5
---

Gets the windows the session has open, in any runspace of the process. Source: `crates/pwrs-gui/src/inspect.rs`, `GetGuiWindow` and `GuiWindowInfo`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its names for unsigned integer types, `uint32` and `uint64` where PowerShell 7 writes `uint` and `ulong`, except where its output is shown too.

## Syntax

```powershell
Get-Command Get-GuiWindow -Syntax
```

```text
Get-GuiWindow [[-Title] <string>] [<CommonParameters>]
```

## Description

A command such as [Read-GuiChoice](Read-GuiChoice.md) waits in its runspace while its window is open, so a script that drives the window from outside runs in another runspace of the same process. PWRS-GUI has one session per renderer for the process, shared by every runspace, so `Get-GuiWindow` lists every window the sessions have open, whichever runspace opened it, and its `Id` is what [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) and [Get-GuiSnapshot](Get-GuiSnapshot.md) take as `-Window`. A window is listed once it has opened, with its first frame drawn and, in an HTML window, the fonts its text uses loaded; a window opened by another process is not listed.

When no window has been opened in the process there is no renderer for the command to reach, and `Get-GuiWindow` writes nothing; it never starts a renderer.

## Parameters

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: every window |

Only windows whose title matches this wildcard pattern, compared without regard to case, as PowerShell's own `-like` compares.

## Inputs

None.

## Outputs

`PwrsGui.Window`, one per open window:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the window's number in the session |
| `Title` | `System.String` | the window's title |
| `Names` | `System.String[]` | the names of its widgets, in tree order, which `-Name` takes on the other commands |
| `SourceIdentifier` | `System.String` | null in what `Get-GuiWindow` writes; in the `PwrsGui.Window` that `Show-GuiWindow -NoWait` writes for a window with handlers, the engine event they run on, which `Unregister-Event` removes |

Before any window has opened, nothing is written:

```powershell
Get-GuiWindow
```

A dialog started in a second runspace, and found by its title once it has opened. The loop gives up after 30 seconds:

```powershell
$dialog = [powershell]::Create().AddScript({
    Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production' -Title 'Deploy'
})
$running = $dialog.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($window = Get-GuiWindow -Title 'Deploy') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
$window
```

```text
Id Title  Names                SourceIdentifier
-- -----  -----                ----------------
 1 Deploy {Choice, Ok, Cancel}
```

```powershell
$window | Get-Member -MemberType Properties
```

```text
   TypeName: PwrsGui.Window

Name             MemberType Definition
----             ---------- ----------
Id               Property   ulong Id {get;set;}
Names            Property   string[] Names {get;set;}
SourceIdentifier Property   string SourceIdentifier {get;set;}
Title            Property   string Title {get;set;}
```

## Examples

The examples from the command's help.

Every window the session has open:

```powershell
Get-GuiWindow
```

```text
Id Title  Names                SourceIdentifier
-- -----  -----                ----------------
 1 Deploy {Choice, Ok, Cancel}
```

Windows by a wildcard pattern on their titles:

```powershell
Get-GuiWindow -Title 'dep*'
```

```text
Id Title  Names                SourceIdentifier
-- -----  -----                ----------------
 1 Deploy {Choice, Ok, Cancel}
```

Cancel the dialog, which writes nothing, and the window is gone:

```powershell
Invoke-GuiTestEvent -Window $window.Id -Name Cancel -Event Click
$dialog.EndInvoke($running)
$dialog.Dispose()
Get-GuiWindow
```

## Errors

None of its own. A renderer that has gone is reported as `GuiRendererGone`, described in the [Error Reference](Error-Reference.md).
