---
title: New-GuiGpuSurface
weight: 37
---

Describes a surface the session's WGSL draws on, with the egui renderer's GPU or an HTML window's page. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiGpuSurface`; the renderer's side is `crates/pwrs-gui-egui/src/surface.rs`, and the plans its draws are recorded from, `crates/pwrs-gui-gpu/src/surface.rs`; an HTML window's page draws it in `crates/pwrs-gui-web/src/runtime/runtime.js`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070 and a display at 100 percent scale. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiGpuSurface -Syntax
```

```text
New-GuiGpuSurface [-Name] <string> [-Width <double>] [-Height <double>] [-OnPointerDown <scriptblock>] [-OnPointerUp <scriptblock>] [-OnPointerMove <scriptblock>] [-OnWheel <scriptblock>] [-OnKey <scriptblock>] [-OnDrop <scriptblock>] [<CommonParameters>]
```

## Description

`New-GuiGpuSurface` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a texture of its own in an egui window, and as a canvas of its own in an HTML one: `-Width` by `-Height` points, or without them the room its place leaves across and a share of the height its column leaves. In a `-Content` block `GpuSurface` stands for `New-GuiGpuSurface`.

The surface shows its window's own background, light or dark, until [Set-GuiGpuSurface](Set-GuiGpuSurface.md) gives it its draws, so a window that gives them a moment after it opens does not flash black. From then on each frame of its window clears the texture to the surface's clear color and draws them, with the renderer's GPU, inside the window's own frame, and [Save-GuiGpuSurface](Save-GuiGpuSurface.md) writes what it drew last to a PNG. Both find the surface by its `-Name`, which the command requires for that reason. The texture is the widget's size in physical pixels, made again when the widget resizes, so a draw's clip-space coordinates span the surface at any size.

The surface reports the pointer through a handler for each kind of event, `-OnPointerDown`, `-OnPointerUp`, `-OnPointerMove` and `-OnWheel`, which in a window opened with `-NoWait` [Wait-GuiEvent](Wait-GuiEvent.md) also reads. Each kind is sent only when its handler is given:

- a press over the surface;
- that button's release, wherever the pointer then is;
- a move over the surface, or anywhere while a button pressed over it is still down, once a frame at most;
- a turn of the wheel over it.

A gesture the surface's [camera](Set-GuiGpuCamera.md) makes is reported too. The event's `Value` gives:

| Property | What it gives |
|---|---|
| `X`, `Y` | where, in physical pixels from the surface's top left, as the frame uniform's `pointer` |
| `U`, `V` | the same place as fractions of the surface's width and height, the same at any display scale |
| `PixelsPerPoint` | the display's scale, so `X / PixelsPerPoint` is in points |
| `Button` | `Left`, `Right` or `Middle`; for a move the one pressed, and null for none or a wheel turn |
| `Shift`, `Ctrl`, `Alt` | the modifier keys pressed, Ctrl counting Command on macOS |
| `Notches` | for a wheel turn, the notches turned, positive away from the person; a line or a page counts one, and 100 points of a touchpad's scrolling one |

`-OnKey` runs for a key pressed while the surface has the focus, which a press on it gives it. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) raises each kind as a person would, its `-Value` a hashtable of `U` and `V`, with any of `Button`, `Shift`, `Ctrl`, `Alt` and `Notches`, once the surface has been drawn.

In an HTML window the page draws the surface with its engine's WebGPU, or with WebGL 2 where the engine offers none. It draws from copies of the buffers, textures and samplers its draws name, which the module sends with `Set-GuiGpuSurface` and again after each change. [Get-GuiOption](Get-GuiOption.md)'s `WebGpu` says which the pages draw with. Its events, its camera and its saves are the same as an egui window's.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |

The name `Set-GuiGpuSurface`, `Save-GuiGpuSurface`, the handlers' events and the update commands use for the surface.

### -Width

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the room its place leaves across |
| Refused with | `GuiBadSize` when not above 0 |

The surface's width in points.

### -Height

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: a share of the height its column leaves |
| Refused with | `GuiBadSize` when not above 0 |

The surface's height in points.

### -OnPointerDown

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a pointer button is pressed over the surface, with the event as its argument.

### -OnPointerUp

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a button pressed over the surface is released, wherever the pointer is, with the event as its argument.

### -OnPointerMove

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the pointer moves over the surface, or anywhere while a button pressed over it is still down, once a frame at most, with the event as its argument.

### -OnWheel

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the wheel turns over the surface, with the event as its argument.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key is pressed while the surface has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the surface, with the event as its argument; its `Value` is the paths dropped, or the text.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `GpuSurface`, its `Name` the surface's, and its `Properties` with `Width` and `Height` when they are given.

## Examples

The widget the command writes:

```powershell
New-GuiGpuSurface View -Width 320 -Height 240 | Format-List Kind, Name, Properties
```

```text
Kind       : GpuSurface
Name       : View
Properties : {[Width, 320], [Height, 240]}
```

<!-- powershell -->

```text
Kind       : GpuSurface
Name       : View
Properties : {Width, Height}
```

The surface the command's help opens, given a gradient to draw; a snapshot of it gives the surface's size in physical pixels:

```powershell
$gradient = @'
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(at.x / 320.0, at.y / 240.0, 0.5, 1.0);
}
'@
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 320 -Height 240 }
Set-GuiGpuSurface View $gradient -VertexCount 3 -Window $w.Id
$shot = Get-GuiSnapshot -Window $w.Id -Name View -Pixels
'{0} by {1} pixels' -f $shot.Width, $shot.Height
Close-GuiWindow -Window $w.Id
```

```text
320 by 240 pixels
```

A press and a wheel turn, raised as a person's would be and read back with Wait-GuiEvent:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 320 -Height 240 -OnPointerDown {} -OnWheel {} }
$null = Get-GuiSnapshot -Window $w.Id -Name View
Invoke-GuiTestEvent -Window $w.Id -Name View -Event PointerDown -Value @{ U = 0.25; V = 0.5 }
$pressed = Wait-GuiEvent -Window $w.Id -Name View -Timeout 20
$pressed.Kind
$pressed.Value
Invoke-GuiTestEvent -Window $w.Id -Name View -Event Wheel -Value @{ U = 0.5; V = 0.5; Notches = 3 }
$turned = Wait-GuiEvent -Window $w.Id -Name View -Timeout 20
'{0}: {1} notches' -f $turned.Kind, $turned.Value.Notches
Close-GuiWindow -Window $w.Id
```

```text
PointerDown

Name                           Value
----                           -----
Alt                            False
Button                         Left
Ctrl                           False
PixelsPerPoint                 1
Shift                          False
U                              0.25
V                              0.5
X                              80
Y                              120

Wheel: 3 notches
```

A side of 0 points:

```powershell
try { New-GuiGpuSurface View -Width 0 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadSize,Pwrs.Modules.PWRS_GUI.NewGuiGpuSurfaceCommand
a GpuSurface's Width is 0; it must be above 0 points
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadSize` | `-Width` or `-Height` is not above 0 |

Each is described in the [Error Reference](Error-Reference.md).
