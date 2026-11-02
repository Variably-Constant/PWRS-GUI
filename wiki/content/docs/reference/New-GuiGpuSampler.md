---
title: New-GuiGpuSampler
weight: 9
---

Creates a sampler in the egui renderer: how a shader filters and wraps when it samples a texture. Source: `crates/pwrs-gui/src/gpu.rs`, `NewGuiGpuSampler` and `GpuSampler`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/texture.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command New-GuiGpuSampler -Syntax
```

```text
New-GuiGpuSampler [-Filter <FilterMode>] [-AddressMode <AddressMode>] [-Label <string>] [<CommonParameters>]
```

## Description

`New-GuiGpuSampler` makes a sampler on the GPU device of the session's egui renderer and writes it as a `PwrsGui.GpuSampler`. A shader declares one as `var name: sampler;` and passes it to `textureSample` or `textureSampleLevel` with a texture, and [Set-GuiGpuSurface](Set-GuiGpuSurface.md) and [Invoke-GuiGpuCompute](Invoke-GuiGpuCompute.md) bind it by name as they bind a texture [New-GuiGpuTexture](New-GuiGpuTexture.md) or [Import-GuiGpuImage](Import-GuiGpuImage.md) made. One sampler can be used with any number of textures.

`-Filter` says how it picks between texels, the same when a texel covers more than a pixel and when it covers less, and between mip levels: `Nearest`, the default, takes the nearest texel, and `Linear` blends the nearest ones by distance. `-AddressMode` says what it reads past a texture's edges, the same across, down and through: `ClampToEdge`, the default, the edge texel; `Repeat`, the texture again; and `MirrorRepeat`, the texture mirrored. These defaults are `New-WgpuSampler`'s.

The renderer frees the sampler when the object is collected; a GpuSurface whose draws use it goes on using it until its draws change. A sampler is made again on a new device when the renderer's device is lost.

## Parameters

### -Filter

| | |
|---|---|
| Type | `PwrsGui.FilterMode`: `Nearest`, `Linear` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Nearest` |

How it picks between texels.

### -AddressMode

| | |
|---|---|
| Type | `PwrsGui.AddressMode`: `ClampToEdge`, `Repeat`, `MirrorRepeat` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `ClampToEdge` |

What it reads past a texture's edges.

### -Label

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `sampler` and the renderer's number for it |

The sampler's label, which the device's errors name it by.

## Inputs

None.

## Outputs

`PwrsGui.GpuSampler`:

| Property | Type | What it gives |
|---|---|---|
| `Label` | `System.String` | the sampler's label |
| `Filter` | `PwrsGui.FilterMode` | how it picks between texels |
| `AddressMode` | `PwrsGui.AddressMode` | what it reads past a texture's edges |
| `Adapter` | `System.String` | the adapter of the renderer's device; null for a renderer another module started |

## Examples

The examples from the command's help, each sampler then shown.

A sampler that blends texels:

```powershell
$linear = New-GuiGpuSampler -Filter Linear
$linear | Format-List Filter, AddressMode, Adapter
```

```text
Filter      : Linear
AddressMode : ClampToEdge
Adapter     : NVIDIA GeForce RTX 5070
```

A sampler for a texture that tiles, labeled:

```powershell
New-GuiGpuSampler -AddressMode Repeat -Label tiles
```

```text
Label Filter  AddressMode Adapter
----- ------  ----------- -------
tiles Nearest Repeat      NVIDIA GeForce RTX 5070
```

What each filter and address mode reads. A compute shader samples a texture of two texels, black then white, at four points: 0.375 and 0.625 of the way across it, a quarter of the way from each texel's middle to the other's, and the same two points a texture's width further on, past its right edge.

```powershell
$sample = @'
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var look: sampler;
@group(0) @binding(2) var<storage, read_write> seen: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    var at = array<f32, 4>(0.375, 0.625, 1.375, 1.625);
    seen[id.x] = textureSampleLevel(picture, look, vec2<f32>(at[id.x], 0.5), 0.0).r;
}
'@
$picture = New-GuiGpuTexture 2 1 -Format R8Unorm -Data ([byte[]](0, 255))
foreach ($look in (New-GuiGpuSampler), (New-GuiGpuSampler -Filter Linear), (New-GuiGpuSampler -AddressMode Repeat), (New-GuiGpuSampler -AddressMode MirrorRepeat)) {
    $seen = Invoke-GuiGpuCompute $sample @{ picture = $picture; look = $look; seen = [float[]]::new(4) } -Invocations 4
    '{0,-7} {1,-12} {2}' -f $look.Filter, $look.AddressMode, ($seen -join ', ')
}
```

```text
Nearest ClampToEdge  0, 1, 1, 1
Linear  ClampToEdge  0.2500038, 0.7499962, 1, 1
Nearest Repeat       0, 1, 0, 1
Nearest MirrorRepeat 0, 1, 1, 0
```

## Errors

`New-GuiGpuSampler` raises none of its own. The [Error Reference](Error-Reference.md) describes the session's errors, such as `GuiStartFailed`.
