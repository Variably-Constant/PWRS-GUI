---
title: Get-GuiGpuCamera
weight: 9
---

Writes the camera of a GpuSurface where the person has moved it. Source: `crates/pwrs-gui/src/gpu.rs`, `GetGuiGpuCamera` and `GpuCamera`; the renderer reports the camera `crates/pwrs-gui-egui/src/camera.rs` moves, and in an HTML window where the page, which moves the camera in `crates/pwrs-gui-web/src/runtime/runtime.js`, last reported the person left it.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Get-GuiGpuCamera -Syntax
```

```text
Get-GuiGpuCamera [-Name] <string> [-Window <ulong>] [<CommonParameters>]
```

## Description

`Get-GuiGpuCamera` writes the camera [Set-GuiGpuCamera](Set-GuiGpuCamera.md) gave the [GpuSurface](New-GuiGpuSurface.md) named `-Name`, as the person's gestures have left it: its target, distance and angles, its projection, its controls with the gestures they take, where the camera is, and its view matrix. A surface without a camera writes nothing. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotASurface` for a widget of another kind |

The surface's name, compared without regard to case.

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

A `PwrsGui.GpuCamera`, or nothing for a surface without a camera:

| Property | Type | What it gives |
|---|---|---|
| `Name` | `System.String` | the surface's name, as given |
| `Window` | `System.UInt64` | the window's number |
| `Target` | `System.Double[]` | the point the camera looks at and orbits, as x, y and z |
| `Distance` | `System.Double` | how far the camera is from the target |
| `Yaw` | `System.Double` | degrees around +Y from +Z toward +X, from -180 to 180 |
| `Pitch` | `System.Double` | degrees the camera is raised above the target's level, from -90 to 90 |
| `FieldOfView` | `System.Double` | the vertical field of view in degrees |
| `Near` | `System.Double` | the distance from the camera at which the scene starts to be drawn |
| `Far` | `System.Double` | the distance from the camera beyond which the scene is not drawn |
| `Eye` | `System.Double[]` | where the camera is, as x, y and z |
| `Controls` | `PwrsGui.CameraControls` | `Default`, `Blender`, `Custom` or `None` |
| `Bindings` | `System.Collections.Specialized.OrderedDictionary` | each gesture the controls take and the move it makes, its keys compared without regard to case |
| `View` | `System.Single[]` | the view matrix the frame uniform has at byte 32, sixteen numbers column by column |

## Examples

A camera set 10 units out and raised 30 degrees, read back with where that puts it, and nothing once it is removed:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 64 -Height 64 }
Set-GuiGpuCamera View -Distance 10 -Pitch 30 -Window $w.Id
$camera = Get-GuiGpuCamera View -Window $w.Id
'{0} units out, {1} degrees up, the eye at {2}' -f $camera.Distance, $camera.Pitch, (($camera.Eye | ForEach-Object { [math]::Round($_, 3) }) -join ', ')
Remove-GuiGpuCamera View -Window $w.Id
'{0} cameras once removed' -f @(Get-GuiGpuCamera View -Window $w.Id).Count
Close-GuiWindow -Window $w.Id
```

```text
10 units out, 30 degrees up, the eye at 0, 5, 8.66
0 cameras once removed
```
