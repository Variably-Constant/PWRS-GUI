---
title: New-GuiGpuRenderPipeline
weight: 9
---

Builds a render pipeline in the egui renderer for a GpuSurface's draws, as the Wgpu module's `New-WgpuRenderPipeline` builds one. Source: `crates/pwrs-gui/src/gpu.rs`, `NewGuiGpuRenderPipeline` and `GpuRenderPipeline`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and `new_pipeline` in `crates/pwrs-gui-gpu/src/surface.rs`, which builds it through the Wgpu module's library.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its names for unsigned integer types, `uint32` and `uint64` where PowerShell 7 writes `uint` and `ulong`, except where its output is shown too.

## Syntax

```powershell
Get-Command New-GuiGpuRenderPipeline -Syntax
```

```text
New-GuiGpuRenderPipeline [-Shader] <string> [-Depth] [-DepthFormat <DepthFormat>] [-DepthCompare <CompareFunction>] [-DepthWrite] [-DepthBias <int>] [-DepthBiasSlopeScale <double>] [-DepthBiasClamp <double>] [-StencilCompare <CompareFunction>] [-StencilFail <StencilOperation>] [-StencilDepthFail <StencilOperation>] [-StencilPass <StencilOperation>] [-StencilReadMask <uint>] [-StencilWriteMask <uint>] [-BackStencilCompare <CompareFunction>] [-BackStencilFail <StencilOperation>] [-BackStencilDepthFail <StencilOperation>] [-BackStencilPass <StencilOperation>] [-Topology <PrimitiveTopology>] [-CullMode <CullMode>] [-Blend <BlendMode>] [-BlendSource <BlendFactor>] [-BlendDestination <BlendFactor>] [-BlendOperation <BlendOperation>] [-AlphaBlendSource <BlendFactor>] [-AlphaBlendDestination <BlendFactor>] [-AlphaBlendOperation <BlendOperation>] [-SampleCount <uint>] [-VertexEntry <string>] [-FragmentEntry <string>] [-Constant <IDictionary>] [-Label <string>] [<CommonParameters>]
```

## Description

`New-GuiGpuRenderPipeline` compiles `-Shader`, WGSL text with a vertex and a fragment entry point, on the device of the session's egui renderer, and builds a pipeline that draws into a GpuSurface's texture, whose format is `Rgba8Unorm`. The shader is validated first, and an error in it names its line and column; a GLSL shader has one stage, so it cannot draw. The pipeline's vertex layout comes from the vertex entry point's `@location` inputs, each read from a buffer of its own, in location order, as the Wgpu module's is.

The parameters are `New-WgpuRenderPipeline`'s but `-Format` and `-Device`: a surface's texture has one format, and the renderer one device. `-Blend` says how a fragment's color combines with what the surface already has; `Custom` takes its factors and operations from `-BlendSource`, `-BlendDestination` and `-BlendOperation`, and their alpha forms, which no other blend takes. A blend of `Constant`, or a factor of `Constant` or `OneMinusConstant`, reads the constant each draw gives with `-BlendConstant`.

`-Depth` tests and writes depth in a `Depth32Float` attachment, and `-DepthFormat` names another format for it, of depth, stencil or both. `-DepthCompare`, `-DepthWrite` and the depth bias say how the pipeline tests depth, and need a format that holds depth. The stencil parameters say how it tests stencil, and need a format that holds stencil. Triangles facing away take the `-BackStencil` parameters, each the front one's when not given. [Set-GuiGpuSurface](Set-GuiGpuSurface.md) clears the attachment each frame and gives each draw its stencil reference.

[Set-GuiGpuSurface](Set-GuiGpuSurface.md) draws with the pipeline, which any number of draws and surfaces can use; a draw may also build a pipeline of its own from WGSL text. The renderer frees the pipeline when its object is collected, and a surface drawing with it uses it until the surface's draws change. When the renderer's device is lost, the renderer builds the pipeline again on the new device.

## Parameters

### -Shader

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | by value |
| Refused with | `GuiGpuShaderLanguage` for GLSL, and the Wgpu module's shader errors for WGSL that does not validate or lacks an entry point it names |

WGSL text with a vertex and a fragment entry point.

### -Depth

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | no depth |

Test and write depth in a `Depth32Float` attachment.

### -DepthFormat

| | |
|---|---|
| Type | `PwrsGui.DepthFormat`: `Stencil8`, `Depth16Unorm`, `Depth24Plus`, `Depth24PlusStencil8`, `Depth32Float`, `Depth32FloatStencil8` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Depth32Float` with `-Depth`; no depth attachment without it |

Test depth, stencil or both in an attachment of this format, which turns depth on. `Stencil8` holds stencil alone, `Depth16Unorm`, `Depth24Plus` and `Depth32Float` depth alone, and the other two both. `Depth32FloatStencil8` needs a device with the Depth32FloatStencil8 feature.

### -DepthCompare

| | |
|---|---|
| Type | `PwrsGui.CompareFunction`: `Never`, `Less`, `Equal`, `LessEqual`, `Greater`, `NotEqual`, `GreaterEqual`, `Always` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Less` |
| Refused with | the Wgpu module's `WgpuDepthTest` without a depth format that holds depth |

How a fragment's depth compares with the depth under it, for the fragment to be drawn. `Greater`, with Set-GuiGpuSurface's `-ClearDepth 0`, draws reversed depth.

### -DepthWrite

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | writes |
| Refused with | the Wgpu module's `WgpuDepthTest` without a depth format that holds depth |

Whether a fragment that passes the depth test writes its depth; `-DepthWrite:$false` tests depth without changing it.

### -DepthBias

| | |
|---|---|
| Type | `System.Int32` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0 |
| Refused with | the Wgpu module's `WgpuDepthTest` without a depth format that holds depth, and `WgpuDepthBias` beside a topology of points or lines |

A constant added to each triangle's depth, in the depth format's smallest steps.

### -DepthBiasSlopeScale

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0 |
| Refused with | as `-DepthBias` |

A bias this many times the triangle's depth slope.

### -DepthBiasClamp

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | no limit |
| Refused with | as `-DepthBias`; in an HTML window whose engine offers WebGL 2 alone without the EXT_polygon_offset_clamp extension, Set-GuiGpuSurface's `GuiGpuNoWebGpu` |

The largest bias `-DepthBias` and `-DepthBiasSlopeScale` give together, or the most negative when below 0.

### -StencilCompare

| | |
|---|---|
| Type | `PwrsGui.CompareFunction` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Always` |
| Refused with | the Wgpu module's `WgpuStencil` without a depth format that holds stencil |

How the stencil test compares the stencil reference, which each draw gives with Set-GuiGpuSurface's `-StencilReference`, with the stencil value under a fragment, the reference on the left.

### -StencilFail

| | |
|---|---|
| Type | `PwrsGui.StencilOperation`: `Keep`, `Zero`, `Replace`, `Invert`, `IncrementClamp`, `DecrementClamp`, `IncrementWrap`, `DecrementWrap` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Keep` |
| Refused with | the Wgpu module's `WgpuStencil` without a depth format that holds stencil |

What a failed stencil test does to the stencil value. `Replace` writes the stencil reference.

### -StencilDepthFail

| | |
|---|---|
| Type | `PwrsGui.StencilOperation` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Keep` |
| Refused with | as `-StencilFail` |

What a passed stencil test whose depth test fails does to the stencil value.

### -StencilPass

| | |
|---|---|
| Type | `PwrsGui.StencilOperation` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Keep` |
| Refused with | as `-StencilFail` |

What passing the stencil test and the depth test does to the stencil value.

### -StencilReadMask

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | every bit |
| Refused with | as `-StencilFail` |

The bits of the reference and of the stencil value the stencil test compares.

### -StencilWriteMask

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | every bit |
| Refused with | as `-StencilFail` |

The bits of the stencil value the stencil operations change.

### -BackStencilCompare

| | |
|---|---|
| Type | `PwrsGui.CompareFunction` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `-StencilCompare` |
| Refused with | as `-StencilFail` |

`-StencilCompare` for triangles facing away, whose corners turn clockwise on the surface.

### -BackStencilFail

| | |
|---|---|
| Type | `PwrsGui.StencilOperation` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `-StencilFail` |
| Refused with | as `-StencilFail` |

`-StencilFail` for triangles facing away.

### -BackStencilDepthFail

| | |
|---|---|
| Type | `PwrsGui.StencilOperation` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `-StencilDepthFail` |
| Refused with | as `-StencilFail` |

`-StencilDepthFail` for triangles facing away.

### -BackStencilPass

| | |
|---|---|
| Type | `PwrsGui.StencilOperation` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `-StencilPass` |
| Refused with | as `-StencilFail` |

`-StencilPass` for triangles facing away.

### -Topology

| | |
|---|---|
| Type | `PwrsGui.PrimitiveTopology`: `PointList`, `LineList`, `LineStrip`, `TriangleList`, `TriangleStrip` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `TriangleList` |

How vertices make primitives.

### -CullMode

| | |
|---|---|
| Type | `PwrsGui.CullMode`: `None`, `Front`, `Back` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `None` |

Which faces to discard. A triangle whose corners turn counterclockwise on the surface faces the viewer.

### -Blend

| | |
|---|---|
| Type | `PwrsGui.BlendMode`: `Replace`, `Alpha`, `Additive`, `Premultiplied`, `Constant`, `Custom` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Replace` |

How fragments combine with the surface.

### -BlendSource

| | |
|---|---|
| Type | `PwrsGui.BlendFactor`: `Zero`, `One`, `Src`, `OneMinusSrc`, `SrcAlpha`, `OneMinusSrcAlpha`, `Dst`, `OneMinusDst`, `DstAlpha`, `OneMinusDstAlpha`, `SrcAlphaSaturated`, `Constant`, `OneMinusConstant`, `Src1`, `OneMinusSrc1`, `Src1Alpha`, `OneMinusSrc1Alpha` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `One` |
| Refused with | `WgpuBlend` beside a `-Blend` other than `Custom` |

With `-Blend Custom`, what the fragment's color is multiplied by. `Src1` and the factors after it read the fragment's second color, `@blend_src(1)`, and need a device with the DualSourceBlending feature.

### -BlendDestination

| | |
|---|---|
| Type | `PwrsGui.BlendFactor` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Zero` |
| Refused with | `WgpuBlend` beside a `-Blend` other than `Custom` |

With `-Blend Custom`, what the color under the fragment is multiplied by.

### -BlendOperation

| | |
|---|---|
| Type | `PwrsGui.BlendOperation`: `Add`, `Subtract`, `ReverseSubtract`, `Min`, `Max` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Add` |
| Refused with | `WgpuBlend` beside a `-Blend` other than `Custom` |

With `-Blend Custom`, how the two products combine. `Min` and `Max` take both factors as `One`.

### -AlphaBlendSource

| | |
|---|---|
| Type | `PwrsGui.BlendFactor` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `-BlendSource` |
| Refused with | `WgpuBlend` beside a `-Blend` other than `Custom` |

With `-Blend Custom`, what the fragment's alpha is multiplied by.

### -AlphaBlendDestination

| | |
|---|---|
| Type | `PwrsGui.BlendFactor` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `-BlendDestination` |
| Refused with | `WgpuBlend` beside a `-Blend` other than `Custom` |

With `-Blend Custom`, what the alpha under the fragment is multiplied by.

### -AlphaBlendOperation

| | |
|---|---|
| Type | `PwrsGui.BlendOperation` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `-BlendOperation` |
| Refused with | `WgpuBlend` beside a `-Blend` other than `Custom` |

With `-Blend Custom`, how the two alpha products combine.

### -SampleCount

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 1 |

Samples per pixel, resolved into the surface's texture.

### -VertexEntry

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the shader's only vertex entry point |

The vertex entry point.

### -FragmentEntry

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the shader's only fragment entry point |

The fragment entry point.

### -Constant

| | |
|---|---|
| Type | `System.Collections.IDictionary` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Override constant values, by name or `@id`.

### -Label

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `render pipeline` and the pipeline's number in the renderer |

The pipeline's label.

## Inputs

WGSL text, by value to `-Shader`.

## Outputs

`PwrsGui.GpuRenderPipeline`:

| Property | Type | What it gives |
|---|---|---|
| `Label` | `System.String` | the pipeline's label |
| `VertexEntryPoint` | `System.String` | the vertex entry point it was given; null when the shader's only one draws |
| `FragmentEntryPoint` | `System.String` | the fragment entry point it was given; null when the shader's only one draws |
| `VertexInputs` | `System.String[]` | the vertex inputs, in location order, each read from a buffer of its own |
| `Bindings` | `System.String[]` | the bindings both stages use, in group and binding order |
| `DepthFormat` | `System.String` | the depth attachment's format: `-DepthFormat`'s, or `Depth32Float` with `-Depth` alone; null without either |
| `SampleCount` | `System.UInt32` | samples per pixel |
| `Adapter` | `System.String` | the adapter of the renderer's device; null for a renderer another module started |

## Examples

The examples from the command's help, with the shader they name:

```powershell
$wgsl = @'
struct Tint { color: vec4<f32> }
@group(0) @binding(0) var<uniform> tint: Tint;
@vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return tint.color; }
'@
```

A pipeline, which says what its shader reads:

```powershell
$pipeline = New-GuiGpuRenderPipeline $wgsl
$pipeline | Format-List
```

```text
Label              : render pipeline 1
VertexEntryPoint   :
FragmentEntryPoint :
VertexInputs       : {position}
Bindings           : {tint}
DepthFormat        :
SampleCount        : 1
Adapter            : NVIDIA GeForce RTX 5070
```

One that tests depth, draws four samples per pixel and blends by alpha:

```powershell
New-GuiGpuRenderPipeline $wgsl -Depth -SampleCount 4 -Blend Alpha
```

```text
Label             VertexInputs Bindings SampleCount
-----             ------------ -------- -----------
render pipeline 2 {position}   {tint}             4
```

One that tests stencil in a `Stencil8` attachment, writing each draw's stencil reference wherever it draws:

```powershell
$stenciled = New-GuiGpuRenderPipeline $wgsl -DepthFormat Stencil8 -StencilPass Replace
$stenciled.DepthFormat
```

```text
Stencil8
```

## Errors

The command's errors are non-terminating, so each attempt here stops on its own with `-ErrorAction Stop`. GLSL, WGSL that does not validate, a custom blend's factor beside another blend, a depth test without a depth attachment, a stencil test in a format that holds no stencil, and a depth bias on lines:

```powershell
foreach ($attempt in { New-GuiGpuRenderPipeline "#version 450`nvoid main() {}" -ErrorAction Stop },
                     { New-GuiGpuRenderPipeline '@vertex fn vs() -> @builtin(position) vec4<f32> { return 1.0; }' -ErrorAction Stop },
                     { New-GuiGpuRenderPipeline $wgsl -Blend Alpha -BlendSource One -ErrorAction Stop },
                     { New-GuiGpuRenderPipeline $wgsl -DepthCompare Greater -ErrorAction Stop },
                     { New-GuiGpuRenderPipeline $wgsl -Depth -StencilCompare Equal -ErrorAction Stop },
                     { New-GuiGpuRenderPipeline $wgsl -Depth -Topology LineList -DepthBias 1 -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
```

```text
GuiGpuShaderLanguage,Pwrs.Modules.PWRS_GUI.NewGuiGpuRenderPipelineCommand: a render pipeline's shader is WGSL with a vertex and a fragment entry point; GLSL has one stage, so it cannot draw
WgpuShaderInvalid,Pwrs.Modules.PWRS_GUI.NewGuiGpuRenderPipelineCommand: the shader is invalid: line 1, column 1: automatic conversions cannot convert `{AbstractFloat}` to `vec4<f32>`
WgpuBlend,Pwrs.Modules.PWRS_GUI.NewGuiGpuRenderPipelineCommand: -BlendSource, -BlendDestination, -BlendOperation and their alpha forms name a Custom blend, and the blend is Alpha: give -Blend Custom
WgpuDepthTest,Pwrs.Modules.PWRS_GUI.NewGuiGpuRenderPipelineCommand: -DepthCompare, -DepthWrite and the depth bias test depth, and there is no depth target: give -Depth or a -DepthFormat with depth
WgpuStencil,Pwrs.Modules.PWRS_GUI.NewGuiGpuRenderPipelineCommand: a stencil test needs a depth target with a stencil aspect, and the depth target, Depth32Float, has no stencil aspect: give -DepthFormat Stencil8, Depth24PlusStencil8 or Depth32FloatStencil8
WgpuDepthBias,Pwrs.Modules.PWRS_GUI.NewGuiGpuRenderPipelineCommand: a depth bias moves triangles, and the topology is LineList
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed`.
