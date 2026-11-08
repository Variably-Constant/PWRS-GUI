---
title: New-GuiGpuTexture
weight: 9
---

Creates a texture in the egui renderer: a grid of texels of one format, zeroed or filled from an array. Source: `crates/pwrs-gui/src/gpu.rs`, `NewGuiGpuTexture` and `GpuTexture`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/texture.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its name for an unsigned integer type, `uint32` where PowerShell 7 writes `uint`, except where its output is shown too.

## Syntax

```powershell
Get-Command New-GuiGpuTexture -Syntax
```

```text
New-GuiGpuTexture [-Width] <uint> [[-Height] <uint>] [-Format <TextureFormat>] [-Data <Object>] [-Label <string>] [<CommonParameters>]
```

## Description

`New-GuiGpuTexture` makes a two-dimensional texture on the GPU device of the session's egui renderer, which starts as [Set-GuiOption](Set-GuiOption.md) says when none is running, and writes it as a `PwrsGui.GpuTexture`. Its texels stay on the GPU until a command reads them, so a texture made once can be sampled by every frame of a GpuSurface and bound to many compute runs.

`-Format` names the format of its texels, `Rgba8Unorm` when omitted. `-Data` gives every texel, rows packed from the top, as the format's number type, converted as the Wgpu module converts values; without it the texels are zeroed:

| Formats | Channels | `-Data` and [Read-GuiGpuTexture](Read-GuiGpuTexture.md) as | A shader declares it as |
|---|---|---|---|
| `R8Unorm`, `Rg8Unorm`, `Rgba8Unorm` | 1, 2, 4 | `byte`, 0 to 255 | `texture_2d<f32>`, read as 0 to 1 |
| `Rgba8UnormSrgb` | 4 | `byte`, the color sRGB-encoded | `texture_2d<f32>`, the color read in linear light |
| `Bgra8Unorm`, `Bgra8UnormSrgb` | 4 | `byte`, blue, green, red and alpha | `texture_2d<f32>`, read as red, green, blue and alpha |
| `R16Float`, `Rg16Float`, `Rgba16Float` | 1, 2, 4 | `float`, stored as 16-bit floats | `texture_2d<f32>` |
| `R32Float`, `Rg32Float`, `Rgba32Float` | 1, 2, 4 | `float` | `texture_2d<f32>` |
| `R32Uint` | 1 | `uint` | `texture_2d<u32>`, read with `textureLoad` |
| `R32Sint` | 1 | `int` | `texture_2d<i32>`, read with `textureLoad` |

The texture may be used for every use its format allows on the adapter, of copying, sampling, storage and render attachment, as `New-WgpuTexture` gives a texture when none is named. It is freed in the renderer when the object is collected, or at once by [Remove-GuiGpuTexture](Remove-GuiGpuTexture.md); a GpuSurface whose draws read it uses it until its draws change. When the renderer's device is lost, the renderer goes on with a new one and fills the texture again from what the session wrote into it, as [Reset-GuiGpuDevice](Reset-GuiGpuDevice.md) describes.

[Write-GuiGpuTexture](Write-GuiGpuTexture.md) overwrites a texture's texels, [Import-GuiGpuImage](Import-GuiGpuImage.md) makes one from an image file, and [Set-GuiGpuSurface](Set-GuiGpuSurface.md) and [Invoke-GuiGpuCompute](Invoke-GuiGpuCompute.md) bind one by name to a shader's texture binding, with a sampler [New-GuiGpuSampler](New-GuiGpuSampler.md) makes.

## Parameters

### -Width

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiGpuTextureSize` for 0 |

Width in texels.

### -Height

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | 1 |
| Refused with | `GuiGpuTextureSize` for 0 |

Height in texels.

### -Format

| | |
|---|---|
| Type | `PwrsGui.TextureFormat`: `R8Unorm`, `Rg8Unorm`, `Rgba8Unorm`, `Rgba8UnormSrgb`, `Bgra8Unorm`, `Bgra8UnormSrgb`, `R16Float`, `Rg16Float`, `Rgba16Float`, `R32Float`, `Rg32Float`, `Rgba32Float`, `R32Uint`, `R32Sint` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Rgba8Unorm` |

The format of its texels.

### -Data

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | zeroed texels |
| Refused with | `GuiGpuTextureSize` for values short of or past every texel's, and the Wgpu module's conversion errors, as [Errors](#errors) shows |

Every texel, rows packed from the top, as the format's number type.

### -Label

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the size and the format, as `256 x 256 Rgba8Unorm` |

The texture's label, which the device's errors name it by.

## Inputs

None.

## Outputs

`PwrsGui.GpuTexture`:

| Property | Type | What it gives |
|---|---|---|
| `Label` | `System.String` | the texture's label |
| `Width` | `System.UInt32` | its width in texels |
| `Height` | `System.UInt32` | its height in texels |
| `Format` | `PwrsGui.TextureFormat` | the format of its texels |
| `Adapter` | `System.String` | the adapter of the renderer's device; null for a renderer another module started |

## Examples

The examples from the command's help, each texture then shown.

A texture of 256 by 256 zeroed texels:

```powershell
$tex = New-GuiGpuTexture -Width 256 -Height 256
$tex
```

```text
Label                Width Height Format     Adapter
-----                ----- ------ ------     -------
256 x 256 Rgba8Unorm 256   256    Rgba8Unorm NVIDIA GeForce RTX 5070
```

A 2 by 2 texture of floats, read back as they were written:

```powershell
$heights = New-GuiGpuTexture 2 2 -Format R32Float -Data ([float[]](0, 0.25, 0.5, 1))
(Read-GuiGpuTexture $heights) -join ', '
```

```text
0, 0.25, 0.5, 1
```

One red texel, labeled:

```powershell
New-GuiGpuTexture 1 1 -Data ([byte[]](255, 0, 0, 255)) -Label red
```

```text
Label Width Height Format     Adapter
----- ----- ------ ------     -------
red   1     1      Rgba8Unorm NVIDIA GeForce RTX 5070
```

## Errors

The command's errors are non-terminating: the pipeline goes on and the error is written, so each attempt here stops on its own with `-ErrorAction Stop`. A texture is at least one texel on a side, `-Data` gives every texel, and each value must fit the format's number type:

```powershell
foreach ($attempt in { New-GuiGpuTexture 0 4 -ErrorAction Stop },
                     { New-GuiGpuTexture 2 2 -Data ([byte[]](1, 2, 3)) -ErrorAction Stop },
                     { New-GuiGpuTexture 1 1 -Format R32Uint -Data (-1) -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
```

```text
GuiGpuTextureSize,Pwrs.Modules.PWRS_GUI.NewGuiGpuTextureCommand: a texture is at least 1 texel on a side, and 0 by 4 was requested
GuiGpuTextureSize,Pwrs.Modules.PWRS_GUI.NewGuiGpuTextureCommand: a 2 by 2 texture of Rgba8Unorm takes 16 bytes, 16 values of UInt8, and 3 bytes were given
WgpuOutOfRange,Pwrs.Modules.PWRS_GUI.NewGuiGpuTextureCommand: -1 does not fit in UInt32, which holds 0 to 4294967295
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed`.
