---
title: Read-GuiFile
weight: 2
---

Shows the system's own dialog for files to open and writes the path of each file chosen. Source: `crates/pwrs-gui/src/dialogs.rs`, `ReadGuiFile`, and the crate `pwrs-gui-dialogs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same, except where its output is shown too. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) `-Dialog`, in a second runspace of the same process, chose each dialog's files. It closes a dialog as a person's choice does.

## Syntax

```powershell
Get-Command Read-GuiFile -Syntax
```

```text
Read-GuiFile [[-Filter] <Object>] [-Multiple] [-InitialDirectory <string>] [-Title <string>] [<CommonParameters>]
```

## Description

`Read-GuiFile` shows the system's dialog for a file to open, or with `-Multiple` any number, and waits. The dialog offers the kinds of files `-Filter` names and starts in `-InitialDirectory`. The command writes the full path of each file chosen, as a string, in the order the dialog gives them, and nothing when the dialog is canceled. Ctrl+C in the console closes the dialog and writes nothing.

| System | The dialog |
|---|---|
| Windows | the common file dialog |
| macOS | an open panel, shown without blocking the renderer; its filters' extensions are offered together, as a panel without an accessory view offers them, and every file when a filter takes every file |
| Linux and the BSDs | the XDG desktop portal's, else zenity's; where neither is there, the renderer draws its own, as [Get-GuiDialog](Get-GuiDialog.md) says |

The dialog is the session's. [Get-GuiDialog](Get-GuiDialog.md) lists it while it is open, from any runspace of the process, and `Invoke-GuiTestEvent -Dialog` chooses paths in it, as a person would. The wait has no time limit.

## Parameters

### -Filter

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: every file |
| Refused with | `GuiBadFilter` for a filter that names no kind of file |

The kinds of files offered, the first chosen when the dialog opens: `'Name|*.ext;*.ext'` strings, or an ordered dictionary from each name to its extensions, as [New-GuiFilePicker](New-GuiFilePicker.md)'s `-Filter` takes them. `*` takes every file.

### -Multiple

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | one file |

Lets any number of files be chosen, and writes the path of each.

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

`System.String`, the full path of each file chosen. Nothing when the dialog is canceled or the wait was stopped.

## Examples

The examples from the command's help.

Choose a log, offered first, or any file:

<!-- person: $d = Wait-Dialog ''; Invoke-GuiTestEvent -Dialog $d.Id -Value 'C:\Users\Public\Documents\setup.log' -->
```powershell
Read-GuiFile -Filter 'Logs|*.log', 'All files|*'
```

```text
C:\Users\Public\Documents\setup.log
```

Choose any number of files and read them; here two of Windows' own:

<!-- person: $d = Wait-Dialog ''; Invoke-GuiTestEvent -Dialog $d.Id -Value "$env:windir\win.ini", "$env:windir\system.ini" -->
```powershell
Read-GuiFile -Multiple -InitialDirectory ~/Documents | Get-Item
```

```text
    Directory: C:\WINDOWS

Mode                 LastWriteTime         Length Name
----                 -------------         ------ ----
-a---            8/1/2026 12:45 AM             92 win.ini
-a---            8/1/2026 12:45 AM            219 system.ini
```

A canceled dialog writes nothing:

<!-- person: $d = Wait-Dialog 'Pick a report'; Invoke-GuiTestEvent -Dialog $d.Id -->
```powershell
$chosen = Read-GuiFile -Title 'Pick a report'
$null -eq $chosen
```

```text
True
```

## Errors

A dialog of one file refuses two paths from a test, and stays open for another try; here a second runspace tries two, then one:

<!-- varies: the dialog's number -->
```powershell
$runspace = [powershell]::Create().AddScript({
    $d = $null
    while (-not $d) { $d = Get-GuiDialog -Title 'One file'; Start-Sleep -Milliseconds 50 }
    try { Invoke-GuiTestEvent -Dialog $d.Id -Value 'a.txt', 'b.txt' -ErrorAction Stop } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
    Invoke-GuiTestEvent -Dialog $d.Id -Value 'C:\Users\Public\Documents\a.txt'
})
$refusing = $runspace.BeginInvoke()
Read-GuiFile -Title 'One file'
$runspace.EndInvoke($refusing)
$runspace.Dispose()
```

```text
C:\Users\Public\Documents\a.txt
GuiRefused,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand: the GUI renderer refused: dialog 17, Open "One file": it takes one path, not 2
```

A filter that names no kind of file is refused before any dialog opens:

```powershell
try { Read-GuiFile -Filter 'Logs' } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
```

```text
GuiBadFilter,Pwrs.Modules.PWRS_GUI.ReadGuiFileCommand: -Filter: Logs is no kind of file; write 'Name|*.txt;*.md'
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed` and `GuiRendererGone`.
