---
title: Save-GuiGpuSurface
weight: 9
---

Writes a GpuSurface to a PNG file. Source: `crates/pwrs-gui/src/gpu.rs`, `SaveGuiGpuSurface`; the renderer's side is `read_due_saves` in `crates/pwrs-gui-egui/src/lib.rs`, which waits for a frame to paint the surface, and `crates/pwrs-gui-egui/src/gpu_thread.rs`, which reads its texture back on the renderer's GPU thread; an HTML window's page draws a frame and reads it back in `crates/pwrs-gui-web/src/runtime/runtime.js`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070 and a display at 100 percent scale. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Save-GuiGpuSurface -Syntax
```

```text
Save-GuiGpuSurface [-Name] <string> [-Path] <string> [-Window <ulong>] [<CommonParameters>]
```

## Description

`Save-GuiGpuSurface` reads back the texture of the [GpuSurface](New-GuiGpuSurface.md) named `-Name` once a frame of its window drawn after the command was given has painted it, so the draws [Set-GuiGpuSurface](Set-GuiGpuSurface.md) gave before are in it. It writes the texture to `-Path` as a PNG of red, green, blue and alpha, at the surface's size in physical pixels. A file already at the path is replaced. The texture is read on the renderer's GPU thread, so a window's frames go on while it is read. A surface no frame draws within 30 seconds, as one on a tab not shown or in a minimized window, is refused with `GuiGpuNotDrawn`. In an HTML window the page draws a frame of the surface when the command requests one and reads it back itself, refusing with `GuiGpuNotDrawn` a surface on a tab not shown or given no room. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotASurface` for a widget of another kind, `GuiGpuNotDrawn` for a surface no frame draws within 30 seconds |

The surface's name, compared without regard to case.

### -Path

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 1 |
| Pipeline input | no |
| Refused with | `GuiGpuSaveFormat` for a path that does not end in `.png`, `GuiGpuSave` for a file that cannot be written |

The PNG file to write, from the current location when relative.

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

None.

## Examples

A surface of 96 by 64 points drawn red and saved; the PNG's header gives its size in pixels:

```powershell
$fill = @'
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
'@
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 96 -Height 64 }
Set-GuiGpuSurface View $fill -VertexCount 3 -Window $w.Id
Save-GuiGpuSurface View .\frame.png -Window $w.Id
$bytes = [IO.File]::ReadAllBytes((Resolve-Path .\frame.png))
'a PNG of {0} by {1} pixels' -f ((([int]$bytes[16] * 256 + $bytes[17]) * 256 + $bytes[18]) * 256 + $bytes[19]), ((([int]$bytes[20] * 256 + $bytes[21]) * 256 + $bytes[22]) * 256 + $bytes[23])
Close-GuiWindow -Window $w.Id
```

```text
a PNG of 96 by 64 pixels
```

## Errors

The command's errors are non-terminating, so each attempt here stops on its own with `-ErrorAction Stop`. A file of another kind than PNG, refused before anything is written, and a widget that is not a surface:

<!-- varies: the window's number -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 64 -Height 64; Text 'a label' -Name Note }
foreach ($attempt in { Save-GuiGpuSurface View C:\Users\Public\frame.bmp -Window $w.Id -ErrorAction Stop },
                     { Save-GuiGpuSurface Note .\frame.png -Window $w.Id -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
Close-GuiWindow -Window $w.Id
```

```text
GuiGpuSaveFormat,Pwrs.Modules.PWRS_GUI.SaveGuiGpuSurfaceCommand: Save-GuiGpuSurface writes a PNG, and C:\Users\Public\frame.bmp does not end in .png
GuiNotASurface,Pwrs.Modules.PWRS_GUI.SaveGuiGpuSurfaceCommand: Note in window 7 is a Text, not a GpuSurface
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiRendererGone`.
