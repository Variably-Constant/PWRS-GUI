---
title: Set-GuiGpuSurface
weight: 9
---

Gives a GpuSurface what it draws each frame, in the Wgpu module's `Invoke-WgpuRender` terms. Source: `crates/pwrs-gui/src/gpu.rs`, `SetGuiGpuSurface`. On the renderer's side, `plan` in `crates/pwrs-gui-gpu/src/surface.rs` takes a surface's draws from the renderer's table and records them once on the GPU thread, and `crates/pwrs-gui-egui/src/surface.rs` records them into each frame. An HTML window's page draws them in `crates/pwrs-gui-web/src/runtime/runtime.js`, with each draw's WGSL written as GLSL ES 3.00 for WebGL 2 in `crates/pwrs-gui-web/src/glsl.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command Set-GuiGpuSurface -Syntax
```

```text
Set-GuiGpuSurface [-Name] <string> [[-Shader] <Object>] [-Vertex <Object>] [-VertexCount <uint>] [-InstanceCount <uint>] [-Index <Object>] [-Binding <Object>] [-BlendConstant <double[]>] [-StencilReference <uint>] [-Depth] [-DepthFormat <DepthFormat>] [-DepthCompare <CompareFunction>] [-DepthWrite] [-DepthBias <int>] [-DepthBiasSlopeScale <double>] [-DepthBiasClamp <double>] [-StencilCompare <CompareFunction>] [-StencilFail <StencilOperation>] [-StencilDepthFail <StencilOperation>] [-StencilPass <StencilOperation>] [-StencilReadMask <uint>] [-StencilWriteMask <uint>] [-BackStencilCompare <CompareFunction>] [-BackStencilFail <StencilOperation>] [-BackStencilDepthFail <StencilOperation>] [-BackStencilPass <StencilOperation>] [-Topology <PrimitiveTopology>] [-CullMode <CullMode>] [-Blend <BlendMode>] [-BlendSource <BlendFactor>] [-BlendDestination <BlendFactor>] [-BlendOperation <BlendOperation>] [-AlphaBlendSource <BlendFactor>] [-AlphaBlendDestination <BlendFactor>] [-AlphaBlendOperation <BlendOperation>] [-SampleCount <uint>] [-VertexEntry <string>] [-FragmentEntry <string>] [-Constant <IDictionary>] [-ClearColor <double[]>] [-ClearDepth <double>] [-ClearStencil <uint>] [-Uniform <Object>] [-Compute <Object[]>] [-Window <ulong>] [<CommonParameters>]

Set-GuiGpuSurface [-Name] <string> -Draw <Object[]> [-ClearColor <double[]>] [-ClearDepth <double>] [-ClearStencil <uint>] [-Uniform <Object>] [-Compute <Object[]>] [-Window <ulong>] [<CommonParameters>]
```

## Description

`Set-GuiGpuSurface` gives the [GpuSurface](New-GuiGpuSurface.md) named `-Name` what each frame of its window draws: its texture cleared to `-ClearColor`, then one draw from the command's own parameters, or the draws `-Draw` gives, in order. Each call replaces all the surface's draws at once, so no frame shows part of them, and a call with no draw leaves the surface its clear color alone. The renderer records every draw once before the surface takes them, so a draw it cannot record is refused by this command with the renderer's reason, and the surface's draws stay as they were. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

A draw takes, in `Invoke-WgpuRender`'s terms:

| Parameter | What the draw takes |
|---|---|
| `-Shader` | a `PwrsGui.GpuRenderPipeline` from [New-GuiGpuRenderPipeline](New-GuiGpuRenderPipeline.md), or WGSL text, which the pipeline parameters build a pipeline from for this draw alone |
| `-Vertex` | the values of the vertex inputs: a hashtable by input name, without regard to case, or values in location order, each a buffer or an array of numbers |
| `-Index` | the indices to draw by: a buffer of `UInt16` or `UInt32` elements, or an array of indices, stored as `UInt16` for a `ushort[]` and as `UInt32` for any other |
| `-VertexCount` | the vertices to draw; when not given, the indices' count with `-Index`, or else as many as the first vertex input has values for |
| `-InstanceCount` | the instances to draw; 1 when not given |
| `-Binding` | the values of the bindings: a hashtable by binding name or `group:binding`, or values in the order the shaders declare their bindings, each a buffer or an array of numbers |
| `-BlendConstant` | the color a blend factor of `Constant` reads, given when the pipeline's blend reads one and only then |
| `-StencilReference` | the value the pipeline's stencil test compares with and `Replace` writes, 0 when not given; a pipeline with no stencil test refuses one |

Buffers are `PwrsGui.GpuBuffer` objects from [New-GuiGpuBuffer](New-GuiGpuBuffer.md), whose default usage covers every way a draw reads a buffer. An array of numbers, typed or not, goes to the GPU with the draw. It is converted to the type the shader declares for the input or binding, as New-GuiGpuBuffer converts one; for a binding of a struct it is sent as its own type, `Float32` for an untyped array. An array of numbers given outside a hashtable is one value, for the first input or binding. `-Draw` takes an array of hashtables, one per draw, whose keys are these parameters' names and the pipeline parameters', without regard to case; a key given `$null` is not given.

A pipeline built with `-Depth` or `-DepthFormat` tests depth, stencil or both in the surface's depth attachment. Each frame clears it to `-ClearDepth` and `-ClearStencil`, 1 and 0 when not given, so `-DepthCompare Greater` with `-ClearDepth 0` draws reversed depth. Each draw whose pipeline tests stencil compares its own `-StencilReference`, 0 when not given, whatever a draw before it gave. So a stencil test keeps a draw to where earlier draws of the same frame wrote their references.

The draws of one surface share its attachments, so their pipelines draw with the same samples per pixel and the same depth format. The buffers and pipelines its draws name, the copies of its arrays, and the buffer an array of indices is put in stay alive until the surface's draws change or its window closes: garbage collection frees none of them meanwhile, and a buffer [Remove-GuiGpuBuffer](Remove-GuiGpuBuffer.md) takes away is freed then. When the renderer's device is lost, the renderer plans the surface's draws again on the new device.

A draw whose shaders declare a binding at `@group(1) @binding(0)` reads the surface's frame uniform there, one buffer that every such draw of the surface shares. The renderer writes its first 160 bytes before each frame, `-Uniform` gives the fields after them, and a surface whose draws read it draws every frame. A struct that declares these fields first, in this order, lays them out at these bytes:

| Byte | Field | Type | What the renderer writes |
|---|---|---|---|
| 0 | time | `f32` | seconds since the surface's first frame |
| 4 | delta | `f32` | seconds since the frame before, 0 at the first |
| 8 | resolution | `vec2<f32>` | the surface's width and height in physical pixels |
| 16 | pointer | `vec2<f32>` | the pointer in physical pixels from the surface's top left; -1, -1 while it is outside the surface |
| 24 | dark | `f32` | 1 while the window draws in its dark theme, 0 in its light one |
| 28 | scale | `f32` | the display's pixels per point, as a pointer event's `PixelsPerPoint` |
| 32 | view | `mat4x4<f32>` | the view matrix of the surface's [camera](Set-GuiGpuCamera.md); the identity matrix without one |
| 96 | projection | `mat4x4<f32>` | the camera's projection for the surface's width over its height; the identity matrix without a camera |
| 160 | the session's fields | as the shader declares them | `-Uniform`'s values, then zeros to a multiple of 16 bytes |

A struct may leave out `dark` and `scale`: WGSL starts a `mat4x4<f32>` on a multiple of 16 bytes, so `view` stays at byte 32 either way. With `dark`, a shader draws in colors that suit its window's theme, as the rest of the window does.

`-Compute` gives compute passes each frame runs before its draws, in order, in the frame's own commands, so what a pass writes into a buffer the draws read without it leaving the GPU. Each pass is a hashtable of [Invoke-GuiGpuCompute](Invoke-GuiGpuCompute.md)'s parameters, `Shader`, `Binding`, `Workgroups`, `Invocations`, `Indirect`, `IndirectOffset`, `Entry`, `Constant` and `Language`, and `Every`: `Frame`, when not given, runs the pass before every frame's draws, and `Once` before the first frame's alone. A pass may read the frame uniform too. Each call replaces the passes with the draws, so a call without `-Compute` leaves the surface none.

In an HTML window the page draws the surface. It has its own copy of each buffer, texture and sampler the draws and passes name, which the module sends with this command and again after anything changes the object: [Write-GuiGpuBuffer](Write-GuiGpuBuffer.md), [Write-GuiGpuTexture](Write-GuiGpuTexture.md), a compute run that writes it, and a reset of the renderer's device. Where the engine offers WebGPU, the page runs the same WGSL with the same bindings, and runs the passes over its copies; what they write stays in those copies, so [Read-GuiGpuBuffer](Read-GuiGpuBuffer.md) reads what the session and its compute runs wrote. Where the engine offers no WebGPU, the page draws with WebGL 2, the helper writing each draw's WGSL as GLSL ES 3.00. This command then refuses a surface WebGL 2 cannot draw with `GuiGpuNoWebGpu`, naming the engine. That is one with a compute pass, a storage buffer or texture, a blend with a second source, or a depth bias clamp where the engine's WebGL 2 lacks the EXT_polygon_offset_clamp extension. A window opened with `-Renderer Egui` draws it. [Get-GuiOption](Get-GuiOption.md)'s `WebGpu` says which the pages draw with.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotASurface` for a widget of another kind |

The surface's name, compared without regard to case.

### -Shader

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `One`: position 1 |
| Pipeline input | no |
| Default | none: no draw |
| Refused with | `GuiGpuNoShader` beside other draw parameters when not given, or given as anything else; `GuiGpuShaderLanguage` for GLSL; `GuiGpuPipelineOptions` for a pipeline object beside the pipeline parameters |

The draw's pipeline: a `PwrsGui.GpuRenderPipeline`, or WGSL text with a vertex and a fragment entry point.

### -Vertex

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `One`: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiGpuVertex` for anything but buffers and arrays of numbers, `GuiGpuEmpty` for an empty array, `GuiGpuVertexInput` for an input the shader lacks or one given twice |

The vertex inputs' values: a hashtable by input name, or values in location order, each a `PwrsGui.GpuBuffer` or an array of numbers copied as the input's type.

### -VertexCount

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | `One`: named |
| Pipeline input | no |
| Default | the indices' count, with `-Index`; otherwise as many vertices as the first vertex input's buffer or array has values for |
| Refused with | `GuiGpuVertexCount` when it is not given and the draw has no `-Index` and no value for a first vertex input |

The vertices to draw.

### -InstanceCount

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | `One`: named |
| Pipeline input | no |
| Default | 1 |

The instances to draw.

### -Index

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `One`: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiGpuIndexType` for a buffer of other elements than `UInt16` or `UInt32` |

The indices to draw by: a `PwrsGui.GpuBuffer` of `UInt16` or `UInt32` elements, or an array of indices.

### -Binding

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `One`: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiGpuBinding` for anything but buffers and arrays of numbers, a binding the shaders lack, one given twice, or the frame uniform's at `1:0`; `GuiGpuEmpty` for an empty array; the Wgpu module's `WgpuBindingMissing` for a binding left out |

The bindings' values: a hashtable by binding name or `group:binding`, or values in the order the shaders declare their bindings, each a `PwrsGui.GpuBuffer` or an array of numbers copied as the binding's type.

### -BlendConstant

| | |
|---|---|
| Type | `System.Double[]` |
| Parameter sets | `One`: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiGpuBlendConstant` for other than four numbers; the Wgpu module's `WgpuBlendConstant` beside a blend that reads none, and when left out of a draw whose blend reads one |

The color a blend factor of `Constant` reads, as red, green, blue and alpha.

### -StencilReference

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | `One`: named |
| Pipeline input | no |
| Default | 0 |
| Refused with | the Wgpu module's `WgpuStencilReference` beside a pipeline with no stencil test |

The value the pipeline's stencil test compares with and `Replace` writes.

### The pipeline parameters

`-Depth`, `-DepthFormat`, `-DepthCompare`, `-DepthWrite`, `-DepthBias`, `-DepthBiasSlopeScale`, `-DepthBiasClamp`, `-StencilCompare`, `-StencilFail`, `-StencilDepthFail`, `-StencilPass`, `-StencilReadMask`, `-StencilWriteMask`, `-BackStencilCompare`, `-BackStencilFail`, `-BackStencilDepthFail`, `-BackStencilPass`, `-Topology`, `-CullMode`, `-Blend`, `-BlendSource`, `-BlendDestination`, `-BlendOperation`, `-AlphaBlendSource`, `-AlphaBlendDestination`, `-AlphaBlendOperation`, `-SampleCount`, `-VertexEntry`, `-FragmentEntry` and `-Constant` are [New-GuiGpuRenderPipeline](New-GuiGpuRenderPipeline.md)'s. They take the same types and defaults, in the parameter set `One`, named. They build the pipeline of a draw whose `-Shader` is WGSL text, and a draw through a pipeline object refuses them with `GuiGpuPipelineOptions`. That command's page lists what each refuses, such as the Wgpu module's `WgpuDepthTest` for a depth test without a depth format that holds depth.

### -Draw

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | `Draw`: mandatory |
| Pipeline input | no |
| Refused with | `GuiGpuDrawKey` for a draw that is not a hashtable or a key no draw takes, `GuiGpuDrawValue` for a value its key cannot take; each error starts with the draw's place, from 0 |

The draws, in order, each a hashtable whose keys are this command's draw parameters' names: `Shader`, `Vertex`, `VertexCount`, `InstanceCount`, `Index`, `Binding`, `BlendConstant`, `StencilReference` and the pipeline parameters'.

### -Compute

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: no pass |
| Refused with | `GuiGpuPassKey` for a pass that is not a hashtable or a key no pass takes, `GuiGpuPassValue` for a value its key cannot take, `GuiGpuNoShader` for a pass without a shader; in an HTML window whose engine offers no WebGPU, `GuiGpuNoWebGpu` |

The compute passes, in order, each a hashtable whose keys are Invoke-GuiGpuCompute's parameters' names, `Shader`, `Binding`, `Workgroups`, `Invocations`, `Indirect`, `IndirectOffset`, `Entry`, `Constant` and `Language`, and `Every`, `Frame` or `Once`.

### -ClearColor

| | |
|---|---|
| Type | `System.Double[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | opaque black |
| Refused with | `GuiGpuClearColor` for other than three or four numbers |

The color the surface is cleared to each frame, as red, green, blue and alpha, from 0 to 1; the alpha 1 when three are given.

### -ClearDepth

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 1 |
| Refused with | `GuiGpuClearDepth` outside 0 to 1, and for draws whose depth format holds no depth or that have no depth attachment |

The depth the surface's depth attachment is cleared to each frame, from 0 to 1.

### -ClearStencil

| | |
|---|---|
| Type | `System.UInt32` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0 |
| Refused with | `GuiGpuClearStencil` for draws whose depth format holds no stencil or that have no depth attachment |

The value the surface's stencil is cleared to each frame.

### -Uniform

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the frame uniform is the renderer's 160 bytes alone |
| Refused with | `GuiGpuUniform` for anything but an array of numbers, and when no draw reads the frame uniform |

The frame uniform's own fields, from its byte 160: an array of numbers, sent as its own element type, or as `Float32` for an untyped array.

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

The shaders the examples draw with: one triangle covering the surface in red, and triangles at the positions in a buffer, in the color in a uniform buffer:

```powershell
$fill = @'
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
'@
$tinted = @'
struct Tint { color: vec4<f32> }
@group(0) @binding(0) var<uniform> tint: Tint;
@vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return tint.color; }
'@
function Texel($Window, [double]$Across) {
    $shot = Get-GuiSnapshot -Window $Window.Id -Name View -Pixels
    $at = ([int][math]::Floor($shot.Height / 2) * $shot.Width + [int][math]::Floor(($shot.Width - 1) * $Across)) * 4
    $shot.Pixels[$at..($at + 2)] -join ', '
}
```

A pipeline drawn on a surface, which turns red:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 128 -Height 128 }
$pipeline = New-GuiGpuRenderPipeline $fill
Set-GuiGpuSurface View $pipeline -VertexCount 3 -Window $w.Id
Texel $w 0.5
Close-GuiWindow -Window $w.Id
```

```text
255, 0, 0
```

Two draws at once: the red triangle, then the left half drawn green from a buffer of positions by input name, a uniform buffer by binding name and an array of indices:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 128 -Height 128 }
$left = New-GuiGpuBuffer ([float[]](-1, -1, 0, -1, -1, 3))
$green = New-GuiGpuBuffer ([float[]](0, 1, 0, 1))
Set-GuiGpuSurface View -Window $w.Id -Draw @{ Shader = $fill; VertexCount = 3 }, @{ Shader = $tinted; Vertex = @{ position = $left }; Binding = @{ tint = $green }; Index = [uint32[]](0, 1, 2) }
'left {0}; right {1}' -f (Texel $w 0.25), (Texel $w 0.75)
Set-GuiGpuSurface View -ClearColor 0, 0, 1 -Window $w.Id
'after the draws were replaced by the clear color alone: {0}' -f (Texel $w 0.25)
Close-GuiWindow -Window $w.Id
```

```text
left 0, 255, 0; right 255, 0, 0
after the draws were replaced by the clear color alone: 0, 0, 255
```

A draw from arrays alone: the positions as doubles, which go to the GPU as the `vec2<f32>` the shader declares, and the tint as an untyped array, which a binding of a struct takes as `Float32`. With no `-VertexCount`, the six values of the first input make three vertices:

```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 128 -Height 128 }
Set-GuiGpuSurface View $tinted -Vertex ([double[]](-1, -1, 0, -1, -1, 3)) -Binding (0, 1, 0, 1) -ClearColor 1, 0, 0 -Window $w.Id
'left {0}; right {1}' -f (Texel $w 0.25), (Texel $w 0.75)
Close-GuiWindow -Window $w.Id
```

```text
left 0, 255, 0; right 255, 0, 0
```

A draw that reads the frame uniform: the surface's left half, up to half the resolution the renderer writes, in the color `-Uniform` gives, and its right half blue:

```powershell
$framed = @'
struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, view: mat4x4<f32>, projection: mat4x4<f32>, tint: vec4<f32> }
@group(1) @binding(0) var<uniform> frame: Frame;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    if (at.x < frame.resolution.x / 2.0) { return frame.tint; }
    return vec4<f32>(0.0, 0.0, 1.0, 1.0);
}
'@
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 128 -Height 128 }
Set-GuiGpuSurface View $framed -VertexCount 3 -Uniform 0, 1, 0, 1 -Window $w.Id
'left {0}; right {1}' -f (Texel $w 0.25), (Texel $w 0.75)
Close-GuiWindow -Window $w.Id
```

```text
left 0, 255, 0; right 0, 0, 255
```

A compute pass that paints the buffer the draw reads its tint from, once, before the first frame:

```powershell
$paint = @'
@group(0) @binding(0) var<storage, read_write> tint: array<f32, 4>;
@compute @workgroup_size(1) fn main() { tint = array<f32, 4>(0.0, 0.0, 1.0, 1.0); }
'@
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 128 -Height 128 }
$tint = New-GuiGpuBuffer -Length 4
Set-GuiGpuSurface View $tinted -Vertex ([double[]](-1, -1, 3, -1, -1, 3)) -Binding @{ tint = $tint } -Compute @{ Shader = $paint; Binding = $tint; Workgroups = 1; Every = 'Once' } -Window $w.Id
Texel $w 0.5
Close-GuiWindow -Window $w.Id
```

```text
0, 0, 255
```

Reversed depth: two triangles over the whole surface, a near red one at depth 0.2 drawn first and a far blue one at 0.8, each tested `Greater` against a depth cleared to 0. The far one is the greater, so it covers the near one; with depth cleared to 1, as it is when not given, neither is drawn:

```powershell
$deep = @'
override depth: f32 = 0.5;
override red: f32 = 1.0;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], depth, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(red, 0.0, 1.0 - red, 1.0); }
'@
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 128 -Height 128 }
$near = @{ Shader = $deep; Constant = @{ depth = 0.2; red = 1 }; VertexCount = 3; Depth = $true; DepthCompare = 'Greater' }
$far = @{ Shader = $deep; Constant = @{ depth = 0.8; red = 0 }; VertexCount = 3; Depth = $true; DepthCompare = 'Greater' }
Set-GuiGpuSurface View -Draw $near, $far -ClearColor 0, 1, 0 -ClearDepth 0 -Window $w.Id
'cleared to 0: {0}' -f (Texel $w 0.5)
Set-GuiGpuSurface View -Draw $near, $far -ClearColor 0, 1, 0 -Window $w.Id
'cleared to 1: {0}' -f (Texel $w 0.5)
Close-GuiWindow -Window $w.Id
```

```text
cleared to 0: 0, 0, 255
cleared to 1: 0, 255, 0
```

A stencil mask: the first draw writes its stencil reference, 1, where its green triangle covers the left half, and the second draws red over the whole surface where the stencil equals its own reference. Given 1, the red keeps to the left half; given none, it tests 0, and covers the right half:

```powershell
$leftHalf = @'
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 1.0, 0.0, 1.0); }
'@
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 128 -Height 128 }
$writer = @{ Shader = $leftHalf; VertexCount = 3; DepthFormat = 'Stencil8'; StencilPass = 'Replace'; StencilReference = 1 }
$masked = @{ Shader = $fill; VertexCount = 3; DepthFormat = 'Stencil8'; StencilCompare = 'Equal' }
Set-GuiGpuSurface View -Draw $writer, ($masked + @{ StencilReference = 1 }) -ClearColor 0, 0, 1 -Window $w.Id
'reference 1: left {0}; right {1}' -f (Texel $w 0.25), (Texel $w 0.75)
Set-GuiGpuSurface View -Draw $writer, $masked -ClearColor 0, 0, 1 -Window $w.Id
'no reference: left {0}; right {1}' -f (Texel $w 0.25), (Texel $w 0.75)
Close-GuiWindow -Window $w.Id
```

```text
reference 1: left 255, 0, 0; right 0, 0, 255
no reference: left 0, 255, 0; right 255, 0, 0
```

Inside a handler the command acts on the handler's window, as the update commands do:

<!-- person: $w = Wait-Window 'Paint'; Invoke-GuiTestEvent -Window $w.Id -Name Red -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Done -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Paint' {
    GpuSurface View -Width 64 -Height 64
    Button 'Red' -Name Red -OnClick { Set-GuiGpuSurface View $fill -VertexCount 3 }
    ButtonBar { Button 'Done' -Name Done -Default -Result Done }
}
$result.Result
```

```text
Done
```

## Errors

The command's errors are non-terminating, so each attempt here stops on its own with `-ErrorAction Stop`. The attempts, in order:

- a widget that is not a surface;
- a hashtable given for a vertex input;
- a pipeline object beside a pipeline parameter;
- a draw with nothing to count its vertices from;
- a key no draw takes;
- a key no pass takes;
- a uniform no draw reads;
- a binding given where the frame uniform is;
- a depth clear for draws whose pipelines have no depth attachment;
- a stencil reference that no stencil test reads.

<!-- varies: the window's number -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 64 -Height 64; Text 'a label' -Name Note }
foreach ($attempt in { Set-GuiGpuSurface Note $fill -VertexCount 3 -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View $tinted -Vertex @{ position = @{ x = 0; y = 0 } } -VertexCount 3 -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View $pipeline -Depth -VertexCount 3 -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View $pipeline -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View -Draw @{ Shader = $fill; Count = 3 } -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View -Compute @{ Shader = $fill; Often = 1 } -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View $fill -VertexCount 3 -Uniform 0, 1, 0, 1 -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View $framed -VertexCount 3 -Binding @{ frame = 0, 0, 0, 0 } -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View $fill -VertexCount 3 -ClearDepth 0 -Window $w.Id -ErrorAction Stop },
                     { Set-GuiGpuSurface View $fill -VertexCount 3 -Depth -StencilReference 1 -Window $w.Id -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
Close-GuiWindow -Window $w.Id
```

```text
GuiNotASurface,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: Note in window 48 is a Text, not a GpuSurface
GuiGpuVertex,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: Vertex position takes a PwrsGui.GpuBuffer or an array of numbers, not a hashtable
GuiGpuPipelineOptions,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: a PwrsGui.GpuRenderPipeline draws as New-GuiGpuRenderPipeline built it, so the pipeline parameters go to that command; a draw takes them to build a pipeline of its own from WGSL text
GuiGpuVertexCount,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: draw 0 gives no vertex count, no index buffer and no values for a first vertex input; give -VertexCount
GuiGpuDrawKey,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: draw 0: a draw takes Shader, Vertex, VertexCount, InstanceCount, Index, Binding, BlendConstant, StencilReference, Topology, CullMode, Blend, BlendSource, BlendDestination, BlendOperation, AlphaBlendSource, AlphaBlendDestination, AlphaBlendOperation, Depth, DepthFormat, DepthCompare, DepthWrite, DepthBias, DepthBiasSlopeScale, DepthBiasClamp, StencilCompare, StencilFail, StencilDepthFail, StencilPass, StencilReadMask, StencilWriteMask, BackStencilCompare, BackStencilFail, BackStencilDepthFail, BackStencilPass, SampleCount, VertexEntry, FragmentEntry, Constant; it was given Count
GuiGpuPassKey,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: pass 0: a pass takes Shader, Binding, Workgroups, Invocations, Indirect, IndirectOffset, Entry, Constant, Language, Every; it was given Often
GuiGpuUniform,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: -Uniform was given, and no pass or draw reads the frame uniform at @group(1) @binding(0)
GuiGpuBinding,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: binding frame (1:0) is the surface's frame uniform, which the renderer fills each frame; its own fields, from byte 160, are given with -Uniform
GuiGpuClearDepth,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: -ClearDepth sets the depth the depth target starts at, and the draws' pipelines have no depth target
WgpuStencilReference,Pwrs.Modules.PWRS_GUI.SetGuiGpuSurfaceCommand: a draw's stencil_reference sets the value a stencil test compares with and Replace writes, and this program has no stencil test
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiRendererGone`.
