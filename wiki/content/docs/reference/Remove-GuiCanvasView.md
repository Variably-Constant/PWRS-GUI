---
title: Remove-GuiCanvasView
weight: 9
---

Takes the view from a Canvas. Source: `crates/pwrs-gui/src/gpu.rs`, `RemoveGuiCanvasView`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070 and a display at 100 percent scale. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Remove-GuiCanvasView -Syntax
```

```text
Remove-GuiCanvasView [-Name] <string> [-Window <ulong>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

## Description

`Remove-GuiCanvasView` takes the view [Set-GuiCanvasView](Set-GuiCanvasView.md) gave the [Canvas](New-GuiCanvas.md) named `-Name`, so its items' coordinates are its own points again and no gesture or key moves them; a move under way ends with it. A canvas without a view is left as it is. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotACanvas` for a widget of another kind |

The canvas's name, compared without regard to case.

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | yes, by property name, as `Window` or `Id` |
| Default | inside a handler, the handler's window |
| Refused with | `GuiNoWindow` outside a handler when not given, or when no window described in PowerShell is open with the number it gives |

The window's number.

### -WhatIf and -Confirm

The common risk-mitigation parameters: `-WhatIf` says what the command would do without doing it, and `-Confirm` waits for the person to confirm first.

## Inputs

An object with a `Window` or `Id` property, such as the window `Show-GuiWindow -NoWait` writes.

## Outputs

None.

## Examples

A press at the canvas's middle under a view centered on 0, 0, what the command would take away, and the same press once the view is taken away, landing on the canvas's own middle. A test event lands on the canvas as it was last drawn, so a snapshot waits for each frame first:

<!-- varies: the window's number -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { Canvas -Name Board -Width 200 -Height 100 -OnPointerDown {} }
$press = {
    $null = Get-GuiSnapshot -Window $w.Id -Name Board
    Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.5; V = 0.5 }
    $down = Wait-GuiEvent -Window $w.Id -Name Board -Timeout 20
    'pressed at {0}, {1} in the items'' coordinates' -f $down.Value.SceneX, $down.Value.SceneY
}
Set-GuiCanvasView Board -Center 0, 0 -Zoom 2 -Window $w.Id
& $press
Remove-GuiCanvasView Board -Window $w.Id -WhatIf
Remove-GuiCanvasView Board -Window $w.Id
'{0} views left' -f @(Get-GuiCanvasView Board -Window $w.Id).Count
& $press
Close-GuiWindow -Window $w.Id
```

```text
pressed at 0, 0 in the items' coordinates
What if: Performing the operation "Remove the view" on target "Canvas Board in window 1".
0 views left
pressed at 100, 50 in the items' coordinates
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoCanvas` | the window or the canvas left the renderer before it replied to the command |
| `GuiNotACanvas` | the widget of that name is not a Canvas |
| `GuiNoWidget` | the window has no widget of that name |
| `GuiNoWindow` | no window described in PowerShell has that number, or `-Window` is left out outside a handler |

`GuiNoCanvas` is non-terminating, the others end the command. The [Error Reference](Error-Reference.md) describes each.
