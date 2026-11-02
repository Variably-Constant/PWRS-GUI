---
title: Set-GuiGpuCamera
weight: 9
---

Sets the camera of a GpuSurface: where it looks from, its projection, and the controls the person moves it with. Source: `crates/pwrs-gui/src/gpu.rs`, `SetGuiGpuCamera`; the camera's pose, projection and controls are `crates/pwrs-gui-proto/src/camera.rs`, and `crates/pwrs-gui-egui/src/camera.rs` moves the camera by the person's pointer, as an HTML window's page does in `crates/pwrs-gui-web/src/runtime/runtime.js`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Set-GuiGpuCamera -Syntax
```

```text
Set-GuiGpuCamera [-Name] <string> [-Target <double[]>] [-Distance <double>] [-Yaw <double>] [-Pitch <double>] [-FieldOfView <double>] [-Near <double>] [-Far <double>] [-Controls <CameraControls>] [-Bindings <IDictionary>] [-PassThru] [-Window <ulong>] [<CommonParameters>]
```

## Description

`Set-GuiGpuCamera` sets the camera of the [GpuSurface](New-GuiGpuSurface.md) named `-Name`. The camera's view and projection fill the surface's frame uniform at bytes 32 and 96 each frame, which the draws [Set-GuiGpuSurface](Set-GuiGpuSurface.md) gives read at `@group(1) @binding(0)`. A surface without a camera starts from the default one: 5 units out along +Z from the origin, looking at it, with three.js PerspectiveCamera's projection, a 50 degree vertical field of view, near 0.1 and far 2000, and the Default controls. A parameter left out leaves that part of the camera as it is, so `-Controls` switches the controls without moving the camera. The person then moves the camera with the controls' gestures over the surface, and [Get-GuiGpuCamera](Get-GuiGpuCamera.md) reads where they left it; [Remove-GuiGpuCamera](Remove-GuiGpuCamera.md) takes it away. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

The scene is right-handed with +Y up. The yaw turns the camera around +Y from the target's +Z side toward +X, and the pitch raises it above the target's level, to 90 degrees straight over it. The projection takes depth from 0 at the near distance to 1 at the far one, as wgpu's clip space has it.

The controls:

- `Default`, three.js OrbitControls': the left button orbits, or pans with Shift or Ctrl; the right button pans, or orbits with Shift or Ctrl; the middle button and the wheel zoom.
- `Blender`: the middle button orbits, pans with Shift and zooms with Ctrl; the wheel zooms.
- `Custom`: the gestures `-Bindings` names, each a button, `Left`, `Right` or `Middle`, or the `Wheel`, after any of `Shift`, `Ctrl` and `Alt`, joined by `+`, each making the move it is given: `Orbit`, `Pan` or `Zoom`.
- `None`: no gesture; the camera moves only when a script sets it.

A drag the surface's height across orbits a full circle; a pan moves the scene so the point at the target's depth stays under the pointer; a wheel notch zooms by a factor of 0.95, as does a drag of 100 points. Modifier keys are matched exactly, so a gesture bound with Shift is not made with Shift and Ctrl, and Ctrl is Command on macOS. While the pointer is over a surface whose controls bind the wheel, the wheel moves the camera and nothing behind the surface scrolls.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotASurface` for a widget of another kind |

The surface's name, compared without regard to case.

### -Target

| | |
|---|---|
| Type | `System.Double[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the camera's; 0, 0, 0 for a surface without one |
| Refused with | `GuiGpuCamera` for other than three numbers |

The point the camera looks at and orbits, as x, y and z.

### -Distance

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the camera's; 5 for a surface without one |
| Refused with | `GuiGpuCamera` for 0 or less |

How far the camera is from the target.

### -Yaw

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the camera's; 0 for a surface without one |

Degrees around +Y from +Z toward +X; at 0 the camera is on the target's +Z side.

### -Pitch

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the camera's; 0 for a surface without one |
| Refused with | `GuiGpuCamera` outside -90 to 90 |

Degrees the camera is raised above the target's level, from -90, under it, to 90, over it.

### -FieldOfView

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the camera's; 50 for a surface without one |
| Refused with | `GuiGpuCamera` for 0 or less, or 180 or more |

The vertical field of view in degrees.

### -Near

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the camera's; 0.1 for a surface without one |
| Refused with | `GuiGpuCamera` for 0 or less |

The distance from the camera at which the scene starts to be drawn.

### -Far

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the camera's; 2000 for a surface without one |
| Refused with | `GuiGpuCamera` when it is not greater than the near distance |

The distance from the camera beyond which the scene is not drawn.

### -Controls

| | |
|---|---|
| Type | `PwrsGui.CameraControls` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the camera's; `Default` for a surface without one, or `Custom` with `-Bindings` |
| Refused with | `GuiGpuCamera` for `Custom` without `-Bindings`, and for any other beside it |

The controls the person moves the camera with: `Default`, `Blender`, `Custom` or `None`.

### -Bindings

| | |
|---|---|
| Type | `System.Collections.IDictionary` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiGpuCamera` for a key that names no gesture, a value that names no move, two keys that name one gesture, and an empty table |

Custom controls: a hashtable of gestures, such as `'Shift+Left'`, to the moves they make, compared without regard to case. The camera lists them in order of their gestures' names.

### -PassThru

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Writes the camera as it now is, as Get-GuiGpuCamera writes it.

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

None, or with `-PassThru` a `PwrsGui.GpuCamera`, which [Get-GuiGpuCamera](Get-GuiGpuCamera.md) describes.

## Examples

A red square two units on a side at the origin, drawn through the camera's view and projection. The example reads the texel at the surface's center and near its right edge as the camera changes:

- without a camera, the square fills the surface;
- the default camera shows it from 5 units out;
- at 1.5 units out it fills the surface again;
- turned a quarter circle, the camera looks along the square's plane and shows none of it.

```powershell
$square = @'
struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, view: mat4x4<f32>, projection: mat4x4<f32> }
@group(1) @binding(0) var<uniform> frame: Frame;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0));
    return frame.projection * frame.view * vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
'@
function Texel($Window, [double]$Across) {
    $shot = Get-GuiSnapshot -Window $Window.Id -Name View -Pixels
    $at = ([int][math]::Floor($shot.Height / 2) * $shot.Width + [int][math]::Floor(($shot.Width - 1) * $Across)) * 4
    $shot.Pixels[$at..($at + 2)] -join ', '
}
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 128 -Height 128 }
Set-GuiGpuSurface View $square -VertexCount 6 -Window $w.Id
'without a camera: center {0}; edge {1}' -f (Texel $w 0.5), (Texel $w 0.9)
Set-GuiGpuCamera View -Window $w.Id
'the default camera: center {0}; edge {1}' -f (Texel $w 0.5), (Texel $w 0.9)
Set-GuiGpuCamera View -Distance 1.5 -Window $w.Id
'1.5 units out: center {0}; edge {1}' -f (Texel $w 0.5), (Texel $w 0.9)
Set-GuiGpuCamera View -Yaw 90 -Window $w.Id
'a quarter circle around: center {0}' -f (Texel $w 0.5)
Close-GuiWindow -Window $w.Id
```

```text
without a camera: center 255, 0, 0; edge 255, 0, 0
the default camera: center 255, 0, 0; edge 0, 0, 0
1.5 units out: center 255, 0, 0; edge 255, 0, 0
a quarter circle around: center 0, 0, 0
```

Switching the controls leaves the pose as it is, and a custom set lists its gestures in order of their names:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 64 -Height 64 }
Set-GuiGpuCamera View -Distance 8 -Window $w.Id
$camera = Set-GuiGpuCamera View -Controls Blender -Window $w.Id -PassThru
'{0} controls, {1} units out' -f $camera.Controls, $camera.Distance
$camera.Bindings
$custom = Set-GuiGpuCamera View -Bindings @{ Wheel = 'Zoom'; 'shift+left' = 'pan'; Left = 'Orbit' } -Window $w.Id -PassThru
$custom.Bindings.Keys -join ', '
Close-GuiWindow -Window $w.Id
```

```text
Blender controls, 8 units out

Name                           Value
----                           -----
Middle                         Orbit
Shift+Middle                   Pan
Ctrl+Middle                    Zoom
Wheel                          Zoom

Left, Shift+Left, Wheel
```

## Errors

The command's errors are non-terminating, so each attempt here stops on its own with `-ErrorAction Stop`. A target of two numbers, a distance of 0, custom controls without their gestures, and a gesture with a key that is not a modifier key:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 64 -Height 64 }
foreach ($attempt in { Set-GuiGpuCamera View -Target 1, 2 -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuCamera View -Distance 0 -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuCamera View -Controls Custom -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuCamera View -Bindings @{ 'Meta+Left' = 'Pan' } -Window $w.Id -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
Close-GuiWindow -Window $w.Id
```

```text
GuiGpuCamera,Pwrs.Modules.PWRS_GUI.SetGuiGpuCameraCommand: -Target takes three numbers, x, y and z, and was given 2
GuiGpuCamera,Pwrs.Modules.PWRS_GUI.SetGuiGpuCameraCommand: the distance would be 0, and a camera's distance is greater than 0
GuiGpuCamera,Pwrs.Modules.PWRS_GUI.SetGuiGpuCameraCommand: custom controls are the gestures -Bindings gives
GuiGpuCamera,Pwrs.Modules.PWRS_GUI.SetGuiGpuCameraCommand: 'Meta+Left': Meta is not a modifier key; a gesture's modifiers are Shift, Ctrl and Alt
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiRendererGone`.
