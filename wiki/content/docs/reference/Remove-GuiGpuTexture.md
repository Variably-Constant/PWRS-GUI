---
title: Remove-GuiGpuTexture
weight: 9
---

Takes a texture out of the egui renderer now, rather than when its object is collected, and frees its memory. Source: `crates/pwrs-gui/src/gpu.rs`, `RemoveGuiGpuTexture`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/texture.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Remove-GuiGpuTexture -Syntax
```

```text
Remove-GuiGpuTexture [-Texture] <GpuTexture> [-WhatIf] [-Confirm] [<CommonParameters>]
```

## Description

`Remove-GuiGpuTexture` frees the GPU memory of a `PwrsGui.GpuTexture` that [New-GuiGpuTexture](New-GuiGpuTexture.md) or [Import-GuiGpuImage](Import-GuiGpuImage.md) made. Without it, the renderer frees a texture when PowerShell collects its object, which may be long after a script has done with it; a script that loads many images frees each as it goes.

A GpuSurface whose draws read the texture goes on drawing with it, and the texture's memory is freed only once [Set-GuiGpuSurface](Set-GuiGpuSurface.md) changes the surface's draws or the surface's window closes.

The object stays, and every command given it afterward refuses it with `GuiGpuRemoved`. Removing a texture already removed does nothing.

## Parameters

### -Texture

| | |
|---|---|
| Type | `PwrsGui.GpuTexture` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | by value |

The texture to free.

### -WhatIf and -Confirm

The common risk-mitigation parameters: `-WhatIf` says what the command would do without doing it, and `-Confirm` waits for the person to confirm first.

## Inputs

`PwrsGui.GpuTexture`, by value to `-Texture`.

## Outputs

None.

## Examples

The example from the command's help, then the texture refused once it is gone, with a non-terminating error that `-ErrorAction Stop` turns into one the example catches:

```powershell
$tex = New-GuiGpuTexture 1024 1024
$tex | Remove-GuiGpuTexture
try { Read-GuiGpuTexture $tex -ErrorAction Stop } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
```

```text
GuiGpuRemoved,Pwrs.Modules.PWRS_GUI.ReadGuiGpuTextureCommand: the GPU texture 1024 x 1024 Rgba8Unorm was removed
```

What a removal would do, without doing it:

```powershell
$kept = New-GuiGpuTexture 4 4 -Label 'scratch'
Remove-GuiGpuTexture $kept -WhatIf
(Read-GuiGpuTexture $kept).Length
```

```text
What if: Performing the operation "Free the texture's GPU memory" on target "scratch".
64
```

## Errors

`Remove-GuiGpuTexture` raises none of its own. The [Error Reference](Error-Reference.md) describes the session's errors, such as `GuiRendererGone`.
