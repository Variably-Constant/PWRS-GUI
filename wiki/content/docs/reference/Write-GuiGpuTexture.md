---
title: Write-GuiGpuTexture
weight: 9
---

Writes every texel of a texture in the egui renderer. Source: `crates/pwrs-gui/src/gpu.rs`, `WriteGuiGpuTexture`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/texture.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Write-GuiGpuTexture -Syntax
```

```text
Write-GuiGpuTexture [-Texture] <GpuTexture> [-Data] <Object> [<CommonParameters>]
```

## Description

`Write-GuiGpuTexture` converts `-Data` to the texture's number type, as [New-GuiGpuTexture](New-GuiGpuTexture.md) converts values, and writes it over every texel, rows packed from the top. The values must be every texel's, as many as the texture's width times its height times its format's channels; nothing is written when they are not, and the texture is left as it was.

A GpuSurface that samples the texture shows what was written from its next frame. The renderer stores what the session writes, so when its device is lost it fills the texture again from these writes on the new device, as [Reset-GuiGpuDevice](Reset-GuiGpuDevice.md) describes.

## Parameters

### -Texture

| | |
|---|---|
| Type | `PwrsGui.GpuTexture` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiGpuRemoved` once [Remove-GuiGpuTexture](Remove-GuiGpuTexture.md) has freed it |

The texture to write.

### -Data

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: mandatory, position 1 |
| Pipeline input | no |
| Refused with | `GuiGpuTextureSize` for values short of or past every texel's, and the Wgpu module's conversion errors |

Every texel, rows packed from the top, as the format's number type.

## Inputs

None.

## Outputs

None.

## Examples

The examples from the command's help, on a texture of two zeroed texels of red, green, blue and alpha bytes:

```powershell
$tex = New-GuiGpuTexture 2 1
```

Opaque red then opaque blue:

```powershell
$pixels = [byte[]](255, 0, 0, 255, 0, 0, 255, 255)
Write-GuiGpuTexture $tex $pixels
(Read-GuiGpuTexture $tex) -join ', '
```

```text
255, 0, 0, 255, 0, 0, 255, 255
```

Numbers of any type, converted to the format's, here into a texture of floats:

```powershell
$heights = New-GuiGpuTexture 2 2 -Format R32Float
Write-GuiGpuTexture $heights (1, 2, 3, 4)
(Read-GuiGpuTexture $heights) -join ', '
```

```text
1, 2, 3, 4
```

## Errors

The command's errors are non-terminating, so the example stops on its error with `-ErrorAction Stop`. Values short of every texel's are refused, and the texture is left as it was:

```powershell
try { Write-GuiGpuTexture $tex ([byte[]](1, 2, 3, 4)) -ErrorAction Stop } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
(Read-GuiGpuTexture $tex) -join ', '
```

```text
GuiGpuTextureSize,Pwrs.Modules.PWRS_GUI.WriteGuiGpuTextureCommand: a 2 by 1 texture of Rgba8Unorm takes 8 bytes, 8 values of UInt8, and 4 bytes were given
255, 0, 0, 255, 0, 0, 255, 255
```

The [Error Reference](Error-Reference.md) describes these and the session's errors.
