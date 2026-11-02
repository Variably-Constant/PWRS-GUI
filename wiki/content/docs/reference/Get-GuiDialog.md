---
title: Get-GuiDialog
weight: 5
---

Gets the system's dialogs showing in the session, in any runspace of the process and in either renderer. Source: `crates/pwrs-gui/src/dialogs.rs`, `GetGuiDialog` and `GuiDialogInfo`, and the crate `pwrs-gui-dialogs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its name for unsigned integer types, except where its output is shown too.

## Syntax

```powershell
Get-Command Get-GuiDialog -Syntax
```

```text
Get-GuiDialog [[-Title] <string>] [<CommonParameters>]
```

## Description

[Show-GuiMessage](Show-GuiMessage.md), [Read-GuiFile](Read-GuiFile.md), [Read-GuiSavePath](Read-GuiSavePath.md) and [Read-GuiFolder](Read-GuiFolder.md) wait in their runspace while the system's dialog is open, and so does a [FilePicker](New-GuiFilePicker.md)'s Browse button while its dialog is. `Get-GuiDialog` lists every such dialog the session's renderers show, whichever runspace opened it, with its number, its kind and its title. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) `-Dialog` takes the number and closes the dialog with a choice, as a person would.

Each renderer tracks the dialogs it shows by the request that opened them, until they close. A dialog a FilePicker opens is numbered by its renderer, from bit 63 up, so its number never matches a request of the session's. Where the system has no dialog of the kind, as on a Linux or BSD desktop with neither the XDG portal nor zenity, the egui renderer draws its own in its place under the same number. It draws a message with its icon, its text and its buttons, and a file dialog of folders and files with the dialog's filters, from the colors and spacing its windows use. `Get-GuiDialog` lists it and `-Dialog` closes it as it would the system's. An HTML window's FilePicker has its command request that dialog from the egui renderer, which lists it under the command's number.

A dialog is listed once it shows: on Windows once its window is shown, on macOS once its alert or panel is up, and on Linux and the BSDs once the XDG portal has taken it or zenity has started. One the system cannot show is never listed; the dialog drawn in its place is listed once it is drawn.

When no renderer is running there is nothing to list, and `Get-GuiDialog` writes nothing; it never starts a renderer.

## Parameters

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: every dialog |

Only dialogs whose title matches this wildcard pattern, compared without regard to case, as PowerShell's own `-like` compares. A dialog given no title shows the system's own and lists an empty one.

## Inputs

None.

## Outputs

`PwrsGui.Dialog`, one per dialog showing:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the dialog's number, which `Invoke-GuiTestEvent -Dialog` takes |
| `Kind` | `System.String` | `Message`, `Open`, `Save` or `Folder` |
| `Title` | `System.String` | the dialog's title; empty where the system's own shows |

## Examples

The examples from the command's help. A second runspace shows a message first, so a dialog is open:

```powershell
$showing = [powershell]::Create().AddScript({ Show-GuiMessage 'Delete the old logs?' -Title 'Delete logs' -Icon Question -Buttons YesNo })
$running = $showing.BeginInvoke()
while (-not (Get-GuiDialog -Title 'Delete logs')) { Start-Sleep -Milliseconds 50 }
```

List the dialogs showing:

<!-- varies: the dialog's number -->
```powershell
Get-GuiDialog
```

```text
Id Kind    Title
-- ----    -----
 2 Message Delete logs
```

Press Yes in every dialog whose title starts with Delete, as a person would; the runspace showing it then has the button pressed:

```powershell
Get-GuiDialog -Title 'Delete*' | ForEach-Object { Invoke-GuiTestEvent -Dialog $_.Id -Value Yes }
$showing.EndInvoke($running)
$showing.Dispose()
```

```text
Yes
```

## Errors

`Get-GuiDialog` raises none of its own. `Invoke-GuiTestEvent -Dialog` refuses a number no dialog showing has, with `GuiNoDialog`, and a choice the dialog does not offer, with `GuiBadAnswer` or `GuiRefused`, as [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) shows. The [Error Reference](Error-Reference.md) describes these and the session's errors.
