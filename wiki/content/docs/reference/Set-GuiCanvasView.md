---
title: Set-GuiCanvasView
weight: 9
---

Sets the view of a Canvas: the point of its items' coordinates at its middle, how near, the zoom the person may go to, a fit to its items, and the controls the person pans and zooms with. Source: `crates/pwrs-gui/src/gpu.rs`, `SetGuiCanvasView`; the view, its gestures and its moves are `crates/pwrs-gui-proto/src/view.rs`, and `crates/pwrs-gui-egui/src/view.rs` moves the view by the person's pointer and keys, as an HTML window's page does in `crates/pwrs-gui-web/src/runtime/runtime.js` and `canvas.js`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070 and a display at 100 percent scale. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Set-GuiCanvasView -Syntax
```

```text
Set-GuiCanvasView [-Name] <string> [-Center <double[]>] [-Zoom <double>] [-MinZoom <double>] [-MaxZoom <double>] [-Controls <CanvasViewControls>] [-Bindings <IDictionary>] [-Fit] [-FitItem <string[]>] [-Duration <Object>] [-Easing <string>] [-PassThru] [-Window <ulong>] [<CommonParameters>]
```

## Description

`Set-GuiCanvasView` sets the view of the [Canvas](New-GuiCanvas.md) named `-Name`. A view puts the point `-Center` gives, in the items' coordinates, at the canvas's middle, and draws a unit of those coordinates `-Zoom` points across. The canvas's hit tests follow it, and its events' `SceneX` and `SceneY` give the pointer in the items' coordinates under it. A canvas without a view starts with its items in their own coordinates: zoom 1, the canvas's own middle at its middle, any zoom allowed, and the `Map` controls. A parameter left out leaves that part of the view as it is, so `-Controls` switches the controls without moving the view. `-Fit` and `-FitItem` are applied after the other parameters. They set the center and the zoom, within `-MinZoom` and `-MaxZoom`, so the items fit the canvas. Each item is measured by its outline's box as drawn, grown on each side by half its stroke, a Text by the box it is laid out in and an Image by its box, cut to the clips of the groups around it.

With `-Duration`, the view moves to the change over that many seconds along `-Easing`, `Ease` when not given, from the view drawn when the command runs, and a move under way ends where it is. A gesture or another set during the move ends it the same way. A canvas not yet drawn takes the change at once, and its fit at its first frame. [Get-GuiCanvasView](Get-GuiCanvasView.md) reads the view as the person left it, and [Remove-GuiCanvasView](Remove-GuiCanvasView.md) takes it away. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

The controls:

- `Map`, Leaflet's and Google Maps'. A left drag where no named item takes the drag pans, and glides on, slowing, when let go while it moves. Shift and a left drag there draw a box and zoom to it when let go. The wheel, with or without Ctrl, and a pinch zoom about the pointer. A double-click where no named item takes it zooms in a step, and with Shift out. The arrow keys pan, and `+` and `=` zoom in a step and `-` out.
- `Design`, Figma's and Inkscape's. The wheel scrolls, and with Shift scrolls sideways. Ctrl and the wheel, and a pinch, zoom about the pointer. A middle drag, or a left drag while the space bar is pressed, pans. `+` and `=` zoom in a step and `-` out. Shift and 1 fit every item, Shift and 0 return to zoom 1, and Ctrl and the arrow keys pan; the plain arrow keys stay the canvas's own.
- `Custom`: the gestures and keys `-Bindings` gives, each making the move it is given.
- `None`: no gesture moves the view; only a script does.

A named item takes a drag when the canvas has `-OnDragStart`, `-OnDragMove` or `-OnDragEnd`, and a double-click when it has `-OnDoubleClick`. Keys move the view while the canvas has the focus, which a press on it gives it when its controls take keys. A wheel notch zooms by a factor of 1.2 and scrolls 100 points, a notch being a line of the wheel or 100 points of a trackpad's scroll. A drag zooms a notch nearer for each 100 points up. A step zooms by a factor of 2 and an arrow key pans 80 points, and both ease over a quarter of a second, as a box zoom and a key's fit do. Escape ends the view's drag where it is, a box zoom's box going without its zoom and a pan without its glide. The gestures reach the canvas's pointer and wheel handlers too.

A gesture in `-Bindings` is written as any of `Shift`, `Ctrl` and `Alt`, and for a drag `Space`, joined by `+` to one of:

| Gesture | Is |
|---|---|
| `Left`, `Right`, `Middle` | a drag with that button |
| `Wheel` | the wheel turned |
| `Pinch` | two fingers pinched, which takes no modifier key |
| `DoubleClick`, `RightDoubleClick`, `MiddleDoubleClick` | a double-click with that button |
| a key | `ArrowLeft`, `ArrowRight`, `ArrowUp`, `ArrowDown`, `Plus`, `Minus`, `Equals`, `Home`, `End`, `PageUp`, `PageDown`, a digit, a letter, or `F1` to `F24` |

`Control`, `Command` and `Cmd` are `Ctrl`, `Option` is `Alt`, and Ctrl is Command on macOS. The moves, and the gestures that make them:

| Move | Made by | Does |
|---|---|---|
| `Pan` | a drag, a pinch, an arrow key | the items follow the pointer, or the fingers' middle; an arrow key pans 80 points toward it |
| `Glide` | a drag | as `Pan`, and gliding on when let go while it moves |
| `Zoom` | a drag, the wheel, a pinch | nearer or farther about the pointer |
| `Scroll`, `ScrollAcross` | the wheel | the view moves as the wheel moves a page, or sideways |
| `ZoomIn`, `ZoomOut` | a double-click, a key | a step nearer or farther, about the pointer, or the canvas's middle for a key |
| `BoxZoom` | a drag | a box drawn over the items, which the view fits when let go |
| `Fit` | a double-click, a key | the view fits every item |
| `ActualSize` | a double-click, a key | zoom 1 |

`+`, `-` and `=` are matched with or without Shift, as keyboards type them either way, so a binding of Shift with them is refused; Ctrl with `+`, `-`, `=` or 0 zooms the window, and no binding takes it.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotACanvas` for a widget of another kind |

The canvas's name, compared without regard to case.

### -Center

| | |
|---|---|
| Type | `System.Double[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the view's; the canvas's own middle for a canvas without one |
| Refused with | `GuiCanvasView` for other than two numbers |

The point of the items' coordinates at the canvas's middle, as x and y.

### -Zoom

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the view's; 1 for a canvas without one |
| Refused with | `GuiCanvasView` outside `-MinZoom` and `-MaxZoom` |

How many points on the canvas one unit of the items' coordinates takes.

### -MinZoom

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the view's; no limit for a canvas without one |
| Refused with | `GuiCanvasView` for 0 or less |

The smallest zoom the person may go to.

### -MaxZoom

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the view's; no limit for a canvas without one |
| Refused with | `GuiCanvasView` below `-MinZoom` |

The largest zoom the person may go to.

### -Controls

| | |
|---|---|
| Type | `PwrsGui.CanvasViewControls` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the view's; `Map` for a canvas without one, or `Custom` with `-Bindings` |
| Refused with | `GuiCanvasView` for `Custom` without `-Bindings`, and for any other beside it |

The controls the person moves the view with: `Map`, `Design`, `Custom` or `None`.

### -Bindings

| | |
|---|---|
| Type | `System.Collections.IDictionary` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiCanvasView` for a key that names no gesture, a value that names no move or one its gesture cannot make, two keys that name one gesture, a key the window's zoom takes, and an empty table |

Custom controls: a hashtable of gestures and keys, such as `'Shift+Left'` or `'ArrowLeft'`, to the moves they make, compared without regard to case. The view lists them in order of their gestures' names.

### -Fit

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |
| Refused with | `GuiCanvasView` beside `-FitItem` |

Fits the view to every item, as the canvas draws them.

### -FitItem

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiCanvasView` for none, and for a name no item has |

Fits the view to the items these names name, compared without regard to case.

### -Duration

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the change is made at once |
| Refused with | `GuiCanvasView` for 0 or less, or a value that is neither seconds nor a [TimeSpan] |

How long the view takes to move to the change, as seconds or a [TimeSpan].

### -Easing

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Ease` |
| Refused with | `GuiCanvasView` for a name of no curve, and without `-Duration` |

The curve the move follows: `Linear`, `Ease`, `EaseIn`, `EaseOut` or `EaseInOut`, CSS's curves, or one of Penner's, `EaseIn`, `EaseOut` or `EaseInOut` followed by `Sine`, `Quad`, `Cubic`, `Quart`, `Quint`, `Expo`, `Circ`, `Back`, `Elastic` or `Bounce`, as `EaseInOutCubic`.

### -PassThru

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Writes the view as it is then drawn, as Get-GuiCanvasView writes it.

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

None, or with `-PassThru` a `PwrsGui.CanvasView`, which [Get-GuiCanvasView](Get-GuiCanvasView.md) describes.

## Examples

A view four times nearer, centered on a box's middle, and a press at the canvas's middle landing on that point of the items. A test event lands on the canvas as it was last drawn, so the snapshot after the set waits for a frame drawn under the view:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Map' {
    Canvas -Name Board -Width 200 -Height 100 -OnPointerDown {} @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 20; Height = 20; Fill = 'Tomato' }
}
$null = Get-GuiSnapshot -Window $w.Id -Name Board
Set-GuiCanvasView Board -Center 10, 10 -Zoom 4 -Window $w.Id -PassThru | Format-List Name, Center, Zoom, Controls, Moving
$null = Get-GuiSnapshot -Window $w.Id -Name Board
Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.5; V = 0.5 }
$down = Wait-GuiEvent -Window $w.Id -Name Board -Timeout 20
'{0} at {1}, {2} in the items'' coordinates' -f $down.Value.Item, $down.Value.SceneX, $down.Value.SceneY
Close-GuiWindow -Window $w.Id
```

```text
Name     : Board
Center   : {10, 10}
Zoom     : 4
Controls : Map
Moving   : False

Box at 10, 10 in the items' coordinates
```

A fit to one item over half a second, read while it moves and after:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Fit' {
    Canvas -Name Board -Width 200 -Height 100 @(
        @{ Kind = 'Rect'; Name = 'Wide'; X = 0; Y = 0; Width = 400; Height = 100; Fill = 'LightGray' }
        @{ Kind = 'Rect'; Name = 'Box'; X = 300; Y = 40; Width = 20; Height = 20; Fill = 'Tomato' }
    )
}
$null = Get-GuiSnapshot -Window $w.Id -Name Board
$moving = Set-GuiCanvasView Board -FitItem Box -Duration 0.5 -Easing EaseInOutCubic -Window $w.Id -PassThru
'moving: {0}' -f $moving.Moving
Start-Sleep -Seconds 1
$fitted = Get-GuiCanvasView Board -Window $w.Id
'moving: {0}; center {1}; zoom {2}' -f $fitted.Moving, ($fitted.Center -join ', '), $fitted.Zoom
Close-GuiWindow -Window $w.Id
```

```text
moving: True
moving: False; center 310, 50; zoom 5
```

The Design controls' gestures and keys, and custom ones, listed in order of their gestures' names:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Controls' { Canvas -Name Board -Width 200 -Height 100 }
(Set-GuiCanvasView Board -Controls Design -Window $w.Id -PassThru).Bindings
$custom = Set-GuiCanvasView Board -Bindings @{ Wheel = 'Zoom'; 'space+left' = 'pan'; F = 'Fit' } -Window $w.Id -PassThru
'{0}: {1}' -f $custom.Controls, ($custom.Bindings.Keys -join ', ')
Close-GuiWindow -Window $w.Id
```

```text
Name                           Value
----                           -----
Wheel                          Scroll
Shift+Wheel                    ScrollAcross
Ctrl+Wheel                     Zoom
Pinch                          Zoom
Middle                         Pan
Space+Left                     Pan
Plus                           ZoomIn
Equals                         ZoomIn
Minus                          ZoomOut
Shift+1                        Fit
Shift+0                        ActualSize
Ctrl+ArrowLeft                 Pan
Ctrl+ArrowRight                Pan
Ctrl+ArrowUp                   Pan
Ctrl+ArrowDown                 Pan

Custom: F, Space+Left, Wheel
```

## Errors

The command's errors about the view are non-terminating, so each attempt here stops on its own with `-ErrorAction Stop`. A center of one number, an easing without a duration, a gesture given a move it cannot make, and a zoom outside the view's limits:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { Canvas -Name Board -Width 200 -Height 100 }
foreach ($attempt in { Set-GuiCanvasView Board -Center 5 -Window $w.Id -ErrorAction Stop },
                     { Set-GuiCanvasView Board -Easing EaseIn -Window $w.Id -ErrorAction Stop },
                     { Set-GuiCanvasView Board -Bindings @{ Wheel = 'ZoomIn' } -Window $w.Id -ErrorAction Stop },
                     { Set-GuiCanvasView Board -MinZoom 1 -MaxZoom 4 -Zoom 8 -Window $w.Id -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
Close-GuiWindow -Window $w.Id
```

```text
GuiCanvasView,Pwrs.Modules.PWRS_GUI.SetGuiCanvasViewCommand: -Center takes two numbers, x and y, and was given 1
GuiCanvasView,Pwrs.Modules.PWRS_GUI.SetGuiCanvasViewCommand: -Easing shapes the move -Duration gives; give -Duration too
GuiCanvasView,Pwrs.Modules.PWRS_GUI.SetGuiCanvasViewCommand: Wheel cannot make ZoomIn: the wheel makes Zoom, Scroll or ScrollAcross
GuiCanvasView,Pwrs.Modules.PWRS_GUI.SetGuiCanvasViewCommand: the zoom would be 8, outside MinZoom 1 and MaxZoom 4
```

| Error ID | When |
|---|---|
| `GuiCanvasView` | a parameter the view cannot take, or a fit to an item the canvas does not have; the view stays as it was |
| `GuiNoCanvas` | the window or the canvas left the renderer before it replied to the command |
| `GuiNotACanvas` | the widget of that name is not a Canvas |
| `GuiNoWidget` | the window has no widget of that name |
| `GuiNoWindow` | no window described in PowerShell has that number, or `-Window` is left out outside a handler |

`GuiNotACanvas`, `GuiNoWidget` and `GuiNoWindow` end the command, the others are non-terminating. The [Error Reference](Error-Reference.md) describes each, and the session's errors, such as `GuiRendererGone`.
