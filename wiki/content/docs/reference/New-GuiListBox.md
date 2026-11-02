---
title: New-GuiListBox
weight: 48
---

Describes a list of items to select from, one or any number. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiListBox`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The selections and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiListBox -Syntax
```

```text
New-GuiListBox [-Item] <string[]> [-Multiple] [-Selected <string[]>] [-OnSelect <scriptblock>] [-OnActivate <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiListBox` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a list of `-Item`, of which one can be selected, or with `-Multiple` any number, each item with a check box. In a `-Content` block `ListBox` stands for `New-GuiListBox`. A change of selection raises `Select`, which `-OnSelect` acts on, and in a list without `-Multiple` a double click or Enter raises `Activate` with the item, which `-OnActivate` acts on. `State` has the item selected as text under the widget's name, or with `-Multiple` the items. [Get-GuiSnapshot](Get-GuiSnapshot.md) gives the items' numbers instead, from 0, and [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) selects and activates items by those numbers.

The list is drawn in a frame, each item a row as wide as the frame, and an item too long for it is cut short. It shares the height its column leaves, so a long list scrolls in its place rather than push the widgets after it out of the window.

[Add-GuiItem](Add-GuiItem.md) adds items and [Remove-GuiItem](Remove-GuiItem.md) removes them while the window is open. [Update-GuiWidget](Update-GuiWidget.md) sets `Item`, `Multiple` and `Selected`; new items replace the ones the list had, and the selection stays on items that are still there.

A click, the Tab key or [Set-GuiFocus](Set-GuiFocus.md) gives the list the keyboard focus, as one widget. The item the keyboard is on scrolls into sight and is outlined while the window is the active one. Up, Down, Home, End, PageUp and PageDown move to an item and select it, as a click there would; Space selects the item, and Enter raises `Activate` when the item is selected. With `-Multiple` those keys move without checking anything, Space checks or clears the item, Ctrl+A checks every item, and Enter presses the window's default button. `-OnKey` gets every key but the arrows, Home, End, the page keys, Space, Enter and Ctrl+A.

## Parameters

### -Item

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The items listed, as text.

### -Multiple

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | one item |

Lets any number of items be selected rather than one.

### -Selected

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none selected |

The item, or with `-Multiple` the items, selected when the window opens, named by their text without regard to case.

### -OnSelect

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the selection changes, with the event as its argument and as `$_`; its `Value` is the item selected, or with `-Multiple` the items.

### -OnActivate

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when an item of a list without `-Multiple` is double-clicked or chosen with Enter, with the event as its argument; its `Value` is the item.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the list, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the list does not use itself is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the list.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `ListBox` and its `Properties` with `Item` and `Multiple`, and `Selected` when it is given.

## Examples

A target a person selects before pressing OK:

<!-- person: $w = Wait-Window 'Deploy to'; Invoke-GuiTestEvent -Window $w.Id -Name Target -Event Select -Value 1; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Deploy to' {
    ListBox 'staging', 'production' -Name Target
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.State.Target
```

```text
production
```

A list of several, its selection and an activation each handled. With `-HandlerOutput Pipeline` the handlers' output comes before the result:

<!-- person: $w = Wait-Window 'Hosts'; Invoke-GuiTestEvent -Window $w.Id -Name Hosts -Event Select -Value 0, 2; Invoke-GuiTestEvent -Window $w.Id -Name Hosts -Event Activate -Value 1; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Hosts' -HandlerOutput Pipeline {
    ListBox 'web01', 'web02', 'web03' -Multiple -Name Hosts -OnSelect { 'selected ' + ($_.Value -join ', ') } -OnActivate { "opened $($_.Value)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0..1]
$out[2].State.Hosts
```

```text
selected web01, web03
opened web02
web02
```

Items added and removed while the window is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Queue' { ListBox 'first', 'second' -Name Queue }
Add-GuiItem -Window $w.Id -Name Queue -Item 'third'
Remove-GuiItem -Window $w.Id -Name Queue -Item 'first'
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.items
Close-GuiWindow -Window $w.Id
```

```text
second
third
```

A list of one takes one selection:

```powershell
try { New-GuiListBox 'a', 'b' -Selected 'a', 'b' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiTooManySelected,Pwrs.Modules.PWRS_GUI.NewGuiListBoxCommand
2 items were given to select in a list that selects one; -Multiple selects several
```

## Errors

| Error ID | When |
|---|---|
| `GuiTooManySelected` | `-Selected` names several items without `-Multiple` |
| `GuiBadProperty` | `-Selected` names no item of the list, as the window opens or the list is updated |

Each is described in the [Error Reference](Error-Reference.md).
