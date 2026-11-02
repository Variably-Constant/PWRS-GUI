---
title: New-GuiFilePicker
weight: 47
---

Describes a file or folder chooser: the path chosen, beside a button that opens the system's dialog. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiFilePicker`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The choices and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiFilePicker -Syntax
```

```text
New-GuiFilePicker [[-Path] <string[]>] [-Save] [-Folder] [-Multiple] [-Filter <Object>] [-Title <string>] [-Enabled] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiFilePicker` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as the path chosen beside a Browse button. The button opens the system's dialog for a file to open, with `-Save` a file to save, with `-Folder` a folder, and with `-Multiple` any number of files or folders. In a `-Content` block `FilePicker` stands for `New-GuiFilePicker`. The dialog is the system's own in both renderers, the same [Read-GuiFile](Read-GuiFile.md), [Read-GuiSavePath](Read-GuiSavePath.md) and [Read-GuiFolder](Read-GuiFolder.md) show: [Get-GuiDialog](Get-GuiDialog.md) lists it while it is open, under a number of its renderer's own, [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) `-Dialog` closes it with a choice, and closing the window closes it. Where the system has no dialog of the kind, the egui renderer draws its own; in an HTML window the chooser sends its command a request, and the command has the egui renderer draw the dialog. A test presses the Browse button with `Invoke-GuiTestEvent -Event Click`.

A choice raises `Change` with the path, or with `-Multiple` the paths, which `-OnChange` acts on, and `State` has the same under the widget's name, `$null` until one is chosen. `-Path` gives the path it opens with, resolved from the current location whether or not anything is there. `-Filter` limits the files the dialog shows, the first shown first: strings written `'Text files|*.txt;*.md'`, or an ordered dictionary from each name to its extensions, `[ordered]@{ 'Text files' = 'txt', 'md' }`. A plain hashtable is refused, since its keys have no order.

`-Save`, `-Folder` and `-Multiple` are set when the window opens. [Update-GuiWidget](Update-GuiWidget.md) sets `Path`, `Filter`, `Title` and `Enabled`. A chooser whose button has the focus uses Space and Enter itself; `-OnKey` gets the other keys.

## Parameters

### -Path

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none chosen |

The path chosen when the window opens, or with `-Multiple` the paths, resolved from the current location.

### -Save

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | a file to open |

Chooses a file to save rather than one to open.

### -Folder

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | a file |

Chooses a folder rather than a file.

### -Multiple

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | one |

Lets any number of files or folders be chosen rather than one.

### -Filter

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | every file |

The kinds of files the dialog shows: strings written `'Name|*.txt;*.md'`, or an ordered dictionary from each name to its extensions.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the system's own |

The dialog's title.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the path can be changed; `-Enabled:$false` makes it a chooser that cannot.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person chooses a path, with the event as its argument and as `$_`; its `Value` is the path, or with `-Multiple` the paths.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the chooser, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the chooser does not use itself is pressed while its button has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the chooser.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `FilePicker` and its `Properties` with `Path` when it is given, as full paths, and `Save`, `Folder`, `Multiple`, `Filter`, `Title` and `Enabled` when they are given.

## Examples

A log a person chooses, read back by its file name. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Logs'; Invoke-GuiTestEvent -Window $w.Id -Name Log -Event Change -Value (Join-Path ([IO.Path]::GetTempPath()) 'today.log'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Logs' -HandlerOutput Pipeline {
    FilePicker -Name Log -Filter 'Logs|*.log', 'All files|*' -OnChange { "chose $(Split-Path -Leaf $_.Value)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
Split-Path -Leaf $out[1].State.Log
```

```text
chose today.log
today.log
```

A chooser of several files, given two:

<!-- person: $w = Wait-Window 'Many'; $temp = [IO.Path]::GetTempPath(); Invoke-GuiTestEvent -Window $w.Id -Name Files -Event Change -Value (Join-Path $temp 'a.txt'), (Join-Path $temp 'b.txt'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Many' {
    FilePicker -Name Files -Multiple
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.State.Files | ForEach-Object { Split-Path -Leaf $_ }
```

```text
a.txt
b.txt
```

A plain hashtable as a filter, and a save dialog of a folder:

```powershell
try { New-GuiFilePicker -Filter @{ Text = 'txt' } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { New-GuiFilePicker -Save -Folder } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiFilePickerCommand
Filter: only an ordered dictionary has an order, and a dialog offers its first kind of file first; write [ordered]@{ 'Text files' = 'txt', 'md' } or 'Text files|*.txt;*.md'
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiFilePickerCommand
a FilePicker chooses a file to save or a folder, not both; it takes Save or Folder
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | `-Filter` is a plain hashtable or a string not written `'Name\|*.ext'`; `-Save` is given with `-Folder` or `-Multiple`; `-Folder` with `-Filter`; or several paths without `-Multiple` |
| `GuiOpeningOnly` | `Update-GuiWidget` sets `Save`, `Folder` or `Multiple`, which are taken when the window opens |

Each is described in the [Error Reference](Error-Reference.md).
