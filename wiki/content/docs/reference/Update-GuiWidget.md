---
title: Update-GuiWidget
weight: 14
---

Changes properties of a widget in an open window described in PowerShell, found by its name, or one item of a canvas. Source: `crates/pwrs-gui/src/window.rs`, `UpdateGuiWidget` and `set_canvas_item`; the properties each widget takes, `props_of` in `crates/pwrs-gui/src/widgets.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Update-GuiWidget -Syntax
```

```text
Update-GuiWidget [-Name] <string> [-Property] <Object> [-Item <string>] [-Window <ulong>] [<CommonParameters>]
```

## Description

`Update-GuiWidget` sets properties of the widget `-Name` names in a window [Show-GuiWindow](Show-GuiWindow.md) opened from a `-Content` block, and the window redraws it. `-Property` is a hashtable of the properties to set, under the names the widget's `New-Gui<Widget>` command gives its parameters, without regard to case: `@{ Text = 'Done' }` for a [Text](New-GuiText.md), `@{ Value = 40 }` for a [ProgressBar](New-GuiProgressBar.md), `@{ Enabled = $false }` for a [Button](New-GuiButton.md). Properties it leaves out stay as they were, and each is checked as the widget's command checks it: a value the widget refuses changes nothing.

A few properties are taken only when the window opens, such as a combo box's `Editable`, a widget's `-Ring` and a canvas's `-SharedMemory`; setting one is refused with `GuiOpeningOnly`. A list's new items replace the ones it had, and the selection stays on those of its items still there.

With `-Item`, the command changes one item of the [Canvas](New-GuiCanvas.md) `-Name` names, found by its name at any depth without regard to case. `-Property` then gives the item's keys, as `New-GuiCanvas -Item` takes them: they join the item's own, a `$null` taking one away, and the item is checked again, a change it cannot take changing nothing. An item's `Kind` and `Name` stay those it was made with, and a Group's items change only through `Add-GuiItem` and `Remove-GuiItem`. A key set this way ends any animation of it, and the new value is drawn at once; an `Animate` among the keys starts each of its animations from the value drawn then, replacing only an animation of the same key, and `Animate = $null` ends them all.

Inside a handler, `-Window` may be left out: the handler's window is the one it acts on, whose number is in `$PwrsGuiWindow`. Anywhere else, `-Window` names the window, by the number `Show-GuiWindow -NoWait` and [Get-GuiWindow](Get-GuiWindow.md) give; a window object piped in gives its `Id`.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The widget's name, compared without regard to case.

### -Property

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: position 1, mandatory |
| Pipeline input | no |
| Default | none |

A hashtable of the properties to set, by the names the widget's `New-Gui<Widget>` parameters give them; with `-Item`, the item's keys.

### -Item

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the widget's own properties |

For a canvas, the name of the item to change, compared without regard to case.

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | yes, by property name, as `Window` or `Id` |
| Default | inside a handler, the handler's window |

The window's number.

## Inputs

An object with a `Window` or `Id` property, such as the window `Show-GuiWindow -NoWait` writes.

## Outputs

None.

## Examples

A status line a script changes, read back from the window:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Status' { Text 'Starting' -Name Status }
Update-GuiWidget -Window $w.Id -Name Status -Property @{ Text = 'Done' }
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.text
Close-GuiWindow -Window $w.Id
```

```text
Done
```

Inside a handler the window is the handler's own, so `-Window` is left out. The click is sent by `Invoke-GuiTestEvent` from a second runspace, as a person's would be:

<!-- person: $w = Wait-Window 'Enable'; Invoke-GuiTestEvent -Window $w.Id -Name Go -Event Click; $deadline = [DateTime]::UtcNow.AddSeconds(20); while (-not (Get-GuiSnapshot -Window $w.Id).Tree.Children[1].Children[0].Properties.enabled -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Enable' {
    Button 'Go' -Name Go -OnClick { Update-GuiWidget Ok @{ Enabled = $true } }
    ButtonBar { Button 'OK' -Name Ok -Default -Enabled:$false -Result 'pressed' }
}
$result.Result
```

```text
pressed
```

One item of a canvas, found by its name, read back from the window:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Board' {
    Canvas -Name Board -Width 200 -Height 100 @{ Kind = 'Rect'; Name = 'Box'; X = 10; Y = 10; Width = 40; Height = 40; Fill = 'Tomato' }
}
Update-GuiWidget -Window $w.Id -Name Board -Item box -Property @{ X = 120; Fill = 'Gold' }
$box = @((Get-GuiSnapshot -Window $w.Id).Tree.Properties.items)[0]
'{0} at {1}, {2}' -f $box.name, $box.x, $box.fill
try { Update-GuiWidget -Window $w.Id -Name Board -Item Box -Property @{ Width = $null } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { Update-GuiWidget -Window $w.Id -Name Board -Item Lid -Property @{ X = 0 } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Close-GuiWindow -Window $w.Id
```

```text
Box at 120, Gold
GuiBadItem,Pwrs.Modules.PWRS_GUI.UpdateGuiWidgetCommand
Board: Rect Box: a Rect needs Width
GuiNoItem,Pwrs.Modules.PWRS_GUI.UpdateGuiWidgetCommand
Board: no item is named Lid
```

A property the widget does not take names the ones it does:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Wrong' { ProgressBar -Name Copy }
try { Update-GuiWidget -Window $w.Id -Name Copy -Property @{ Speed = 3 } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Close-GuiWindow -Window $w.Id
```

```text
GuiUnknownProperty,Pwrs.Modules.PWRS_GUI.UpdateGuiWidgetCommand
a ProgressBar takes Value, Maximum, Text, Ring; it was given Speed
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoWindow` | no window described in PowerShell has the number `-Window` gives, or `-Window` is left out outside a handler |
| `GuiNoWidget` | the window has no widget of that name; the message names its widgets |
| `GuiUnknownProperty` | `-Property` names a property the widget does not take; the message names those it does |
| `GuiBadProperty` | a value the widget's command would refuse, or two properties that set the same thing |
| `GuiOpeningOnly` | a property taken only when the window opens |
| `GuiBadNumber` | a number outside the widget's range, as its command checks |
| `GuiNotACanvas` | `-Item` is given for a widget other than a Canvas |
| `GuiNoItem` | the canvas has no item of the name `-Item` gives |
| `GuiBadItem` | a change the item cannot take, or one that sets its `Kind`, `Name` or `Children`; the message names the canvas, the item and the key |

Each is described in the [Error Reference](Error-Reference.md).
