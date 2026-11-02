---
title: Get-GuiCanvasView
weight: 9
---

Writes the view of a Canvas as it is drawn, where the person has moved it. Source: `crates/pwrs-gui/src/gpu.rs`, `GetGuiCanvasView` and `CanvasView`; the renderer reports the view `crates/pwrs-gui-egui/src/view.rs` moves, and in an HTML window where the page, which moves the view in `crates/pwrs-gui-web/src/runtime/runtime.js`, last reported it to be.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070 and a display at 100 percent scale. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Get-GuiCanvasView -Syntax
```

```text
Get-GuiCanvasView [-Name] <string> [-Window <ulong>] [<CommonParameters>]
```

## Description

`Get-GuiCanvasView` writes the view [Set-GuiCanvasView](Set-GuiCanvasView.md) gave the [Canvas](New-GuiCanvas.md) named `-Name`, as it is drawn. It gives where the person has moved the view, or, during a move, a point between where the move began and where it ends. It also gives how near the view is, the smallest and largest zoom the person may go to, its controls with the gestures and keys they take, and whether it is moving. A canvas without a view writes nothing. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

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

## Inputs

An object with a `Window` or `Id` property, such as the window `Show-GuiWindow -NoWait` writes.

## Outputs

A `PwrsGui.CanvasView`, or nothing for a canvas without a view:

| Property | Type | What it gives |
|---|---|---|
| `Name` | `System.String` | the canvas's name, as given |
| `Window` | `System.UInt64` | the window's number |
| `Center` | `System.Double[]` | the point of the items' coordinates at the canvas's middle, as x and y; null while the canvas has not been drawn and its view takes the canvas's own middle as its center |
| `Zoom` | `System.Double` | how many points on the canvas one unit of the items' coordinates takes |
| `MinZoom` | `System.Double` | the smallest zoom the person may go to; for a view given none, the smallest positive normal double, so any zoom above 0 is allowed |
| `MaxZoom` | `System.Double` | the largest zoom the person may go to; infinity for a view given none |
| `Controls` | `PwrsGui.CanvasViewControls` | `Map`, `Design`, `Custom` or `None` |
| `Bindings` | `System.Collections.Specialized.OrderedDictionary` | each gesture and key the controls take and the move it makes, in the order they are matched, its keys compared without regard to case |
| `Moving` | `System.Boolean` | whether a step, a glide or an eased change is moving the view, so it goes on changing |

## Examples

A view a script set, read back in full, and nothing for a canvas without one:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' {
    Canvas -Name Board -Width 200 -Height 100
    Canvas -Name Plain -Width 200 -Height 100
}
$null = Get-GuiSnapshot -Window $w.Id -Name Board
Set-GuiCanvasView Board -Center 50, 25 -Zoom 2 -MaxZoom 8 -Window $w.Id
Get-GuiCanvasView Board -Window $w.Id | Format-List Name, Center, Zoom, MinZoom, MaxZoom, Controls, Moving
'{0} views for Plain' -f @(Get-GuiCanvasView Plain -Window $w.Id).Count
Close-GuiWindow -Window $w.Id
```

```text
Name     : Board
Center   : {50, 25}
Zoom     : 2
MinZoom  : 2.2250738585072E-308
MaxZoom  : 8
Controls : Map
Moving   : False

0 views for Plain
```

The Map controls' gestures and keys, in the order they are matched:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { Canvas -Name Board -Width 200 -Height 100 }
Set-GuiCanvasView Board -Window $w.Id
(Get-GuiCanvasView Board -Window $w.Id).Bindings
Close-GuiWindow -Window $w.Id
```

```text
Name                           Value
----                           -----
Left                           Glide
Shift+Left                     BoxZoom
Wheel                          Zoom
Ctrl+Wheel                     Zoom
Pinch                          Zoom
DoubleClick                    ZoomIn
Shift+DoubleClick              ZoomOut
ArrowLeft                      Pan
ArrowRight                     Pan
ArrowUp                        Pan
ArrowDown                      Pan
Plus                           ZoomIn
Equals                         ZoomIn
Minus                          ZoomOut
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoCanvas` | the window or the canvas left the renderer before it replied to the command |
| `GuiNotACanvas` | the widget of that name is not a Canvas |
| `GuiNoWidget` | the window has no widget of that name |
| `GuiNoWindow` | no window described in PowerShell has that number, or `-Window` is left out outside a handler |

`GuiNoCanvas` is non-terminating, the others end the command. The [Error Reference](Error-Reference.md) describes each, and the session's errors, such as `GuiRendererGone`.
