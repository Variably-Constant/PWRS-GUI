---
title: Read-GuiFolder
weight: 2
---

Shows the system's own dialog for a folder and writes its path. Source: `crates/pwrs-gui/src/dialogs.rs`, `ReadGuiFolder`, and the crate `pwrs-gui-dialogs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same, except where its output is shown too. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) `-Dialog`, in a second runspace of the same process, chose each dialog's folder. It closes a dialog as a person's choice does.

## Syntax

```powershell
Get-Command Read-GuiFolder -Syntax
```

```text
Read-GuiFolder [[-InitialDirectory] <string>] [-Title <string>] [<CommonParameters>]
```

## Description

`Read-GuiFolder` shows the system's dialog for a folder, starting in `-InitialDirectory`, and waits. It writes the full path of the folder chosen, as a string; nothing when the dialog is canceled. Ctrl+C in the console closes the dialog and writes nothing.

The dialog is the system's own on each platform, as [Read-GuiFile](Read-GuiFile.md) describes. It is the session's: [Get-GuiDialog](Get-GuiDialog.md) lists it while it is open, and `Invoke-GuiTestEvent -Dialog` chooses a folder in it. A [FilePicker](New-GuiFilePicker.md) with `-Folder -Multiple` chooses several folders through the same dialog. The wait has no time limit.

## Parameters

### -InitialDirectory

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
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

`System.String`, the full path of the folder chosen. Nothing when the dialog is canceled or the wait was stopped.

## Examples

The examples from the command's help.

Go to the folder chosen:

<!-- person: $d = Wait-Dialog ''; Invoke-GuiTestEvent -Dialog $d.Id -Value 'C:\Users\Public' -->
```powershell
Set-Location (Read-GuiFolder)
(Get-Location).Path
```

```text
C:\Users\Public
```

Let the person choose where the logs go, starting in the home folder:

<!-- person: $d = Wait-Dialog 'Where should the logs go?'; Invoke-GuiTestEvent -Dialog $d.Id -Value 'C:\Users\Public\Documents' -->
```powershell
Read-GuiFolder -InitialDirectory ~ -Title 'Where should the logs go?'
```

```text
C:\Users\Public\Documents
```
