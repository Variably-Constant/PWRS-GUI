---
title: Read-GuiSavePath
weight: 2
---

Shows the system's own dialog for a file to save and writes its path. Source: `crates/pwrs-gui/src/dialogs.rs`, `ReadGuiSavePath`, and the crate `pwrs-gui-dialogs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same, except where its output is shown too. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) `-Dialog`, in a second runspace of the same process, chose each dialog's path. It closes a dialog as a person's choice does.

## Syntax

```powershell
Get-Command Read-GuiSavePath -Syntax
```

```text
Read-GuiSavePath [[-Filter] <Object>] [-FileName <string>] [-InitialDirectory <string>] [-Title <string>] [<CommonParameters>]
```

## Description

`Read-GuiSavePath` shows the system's dialog for a file to save and waits. The dialog offers the kinds of files `-Filter` names, and starts in `-InitialDirectory` with `-FileName` filled in. The command writes the full path chosen, as a string, and nothing when the dialog is canceled. It creates no file; the script writes to the path. Ctrl+C in the console closes the dialog and writes nothing.

The dialog is the system's own on each platform, as [Read-GuiFile](Read-GuiFile.md) describes. It is the session's: [Get-GuiDialog](Get-GuiDialog.md) lists it while it is open, and `Invoke-GuiTestEvent -Dialog` chooses a path in it. The wait has no time limit.

## Parameters

### -Filter

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: every file |
| Refused with | `GuiBadFilter` for a filter that names no kind of file |

The kinds of files offered, the first chosen when the dialog opens, as [Read-GuiFile](Read-GuiFile.md) takes them.

### -FileName

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: an empty name |

The file name the dialog opens with.

### -InitialDirectory

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: where the system starts |

The folder the dialog starts in. A relative path is taken from the current location.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the system's own title |

The dialog's title, which [Get-GuiDialog](Get-GuiDialog.md) matches its `-Title` against.

## Inputs

None.

## Outputs

`System.String`, the full path chosen. Nothing when the dialog is canceled or the wait was stopped.

## Examples

The examples from the command's help. The page's folder goes in an environment variable the second runspace reads, so the paths it chooses name files here:

```powershell
$env:PWRS_GUI_EXAMPLE = (Get-Location).Path
```

Let the person choose where a report goes, offering a name:

<!-- person: $d = Wait-Dialog ''; Invoke-GuiTestEvent -Dialog $d.Id -Value (Join-Path $env:PWRS_GUI_EXAMPLE 'report.csv') -->
```powershell
$path = Read-GuiSavePath -Filter 'Spreadsheets|*.csv' -FileName 'report.csv'
Split-Path -Leaf $path
```

```text
report.csv
```

Save processes where the person says:

<!-- person: $d = Wait-Dialog ''; Invoke-GuiTestEvent -Dialog $d.Id -Value (Join-Path $env:PWRS_GUI_EXAMPLE 'processes.csv') -->
```powershell
Get-Process | Export-Csv -Path (Read-GuiSavePath -Filter 'Spreadsheets|*.csv')
(Get-Item processes.csv).Name
```

```text
processes.csv
```
