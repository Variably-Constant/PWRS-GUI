---
title: Rendering
weight: 60
---

A draw runs a vertex shader once for each vertex, which places it, and a fragment shader once for each pixel a triangle covers, which colors it; both are entry points of one WGSL shader. `Invoke-WgpuRender` draws offscreen, into a texture or an image file: nothing is shown on a screen. Positions are in normalized device coordinates, x and y from -1 to 1 with y up, so a target's top row is at y = 1 and the first texel read back is its top-left pixel. A target is Rgba8Unorm unless `-Format` names another, and `-Target` draws into a texture there already is. The parameters are on [Rendering](Render-Cmdlets.md) under Cmdlets and types, and the `Wgpu.RenderPipeline` class on [Types](Types.md#wgpurenderpipeline).

## Draw with no vertex data

The simplest draw needs no vertex data: three corners chosen by `vertex_index` make a triangle that covers the whole target, here in orange:

```powershell
PS> Get-Content ./cover.wgsl
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.5, 0.0, 1.0); }
PS> $cover = Get-Content ./cover.wgsl -Raw
PS> $texture = Invoke-WgpuRender $cover 4 4 -VertexCount 3
PS> $texture
Label             Width Height Format
-----             ----- ------ ------
Invoke-WgpuRender     4      4 Rgba8Unorm

PS> (Read-WgpuTexture $texture)[0..3] -join ','
255,127,0,255
```

`-OutFile` saves the drawing as an image file in the format its extension names, in place of writing the texture, and `-PassThru` writes the texture as well:

```powershell
PS> Invoke-WgpuRender $cover 64 64 -VertexCount 3 -OutFile ./cover.png
PS> Import-WgpuImage ./cover.png | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
   64     64 Rgba8Unorm
```

## Feed vertices from arrays

Each `@location` input of the vertex shader reads an array of its own from `-Vertex`, by the input's name or in location order, as many numbers for each vertex as the input has components. The number of vertices comes from the arrays:

```powershell
PS> Get-Content ./colored.wgsl
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@location(0) position: vec3<f32>, @location(1) color: vec3<f32>) -> Out {
    var out: Out;
    out.position = vec4<f32>(position, 1.0);
    out.color = color;
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
PS> $colored = Get-Content ./colored.wgsl -Raw
PS> $green = @{ position = [float[]](-1, -1, 0, 3, -1, 0, -1, 3, 0); color = [float[]](0, 1, 0, 0, 1, 0, 0, 1, 0) }
PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 2 2 -Vertex $green))[0..3] -join ','
0,255,0,255

PS> Invoke-WgpuRender $colored 2 2
Invoke-WgpuRender: the vertex shader reads position, color; give them with -Vertex
```

## Draw by indices, draw instances

`-Index` draws by a list of vertex indices, so a square needs four vertices for its two triangles. Here a blue square over the bottom-left quarter of a white target: the top-left pixel stays white, and the bottom-left one is blue:

```powershell
PS> $square = @{ position = [float[]](-1, -1, 0, 0, -1, 0, -1, 0, 0, 0, 0, 0); color = [float[]](0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1) }
PS> $drawn = Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $square -Index ([uint32[]](0, 1, 2, 2, 1, 3)) -ClearColor 1, 1, 1, 1)
PS> $drawn[0..3] -join ','
255,255,255,255

PS> $drawn[48..51] -join ','
0,0,255,255
```

`-InstanceCount` draws the whole set again for each instance, which the shader tells apart by `instance_index`. Four squares, one for each instance:

```powershell
PS> Get-Content ./squares.wgsl
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@location(0) corner: vec2<f32>, @builtin(instance_index) n: u32) -> Out {
    let offset = vec2<f32>(f32(n % 2u) - 0.5, f32(n / 2u) - 0.5);
    var out: Out;
    out.position = vec4<f32>(corner * 0.4 + offset, 0.0, 1.0);
    out.color = vec3<f32>(f32(n) / 3.0, 0.6, 1.0 - f32(n) / 3.0);
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
PS> $corners = @{ corner = [float[]](-1, -1, 1, -1, -1, 1, 1, 1) }
PS> Invoke-WgpuRender (Get-Content ./squares.wgsl -Raw) 128 128 -Vertex $corners -Index ([uint32[]](0, 1, 2, 2, 1, 3)) -InstanceCount 4 -ClearColor 1, 1, 1 -OutFile ./squares.png
PS> (Import-WgpuImage ./squares.png).Width
128
```

![Four squares in a grid on white, shading from blue at the bottom left to orange at the top right, one per instance](/images/squares.png)

## Give the shaders resources

`-Binding` gives the resources both shaders read, as `Invoke-WgpuCompute` binds them. Here a uniform moves the triangle from the bottom-left quarter of the target to the top-right one and tints it, so it colors the pixel in the second row and third column and leaves the bottom-left pixel black:

```powershell
PS> Get-Content ./moved.wgsl
struct Look { shift: vec2<f32>, tint: vec4<f32> }
@group(0) @binding(0) var<uniform> look: Look;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(-1.0, 0.0));
    return vec4<f32>(corners[i] + look.shift, 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return look.tint; }
PS> $moved = Invoke-WgpuRender (Get-Content ./moved.wgsl -Raw) 4 4 -VertexCount 3 -Binding @{ shift = [float[]](1, 1); tint = [float[]](0, 1, 0, 1) }
PS> $texels = Read-WgpuTexture $moved
PS> $texels[24..27] -join ','
0,255,0,255

PS> $texels[48..51] -join ','
0,0,0,255
```

## Test depth

Two triangles over the whole target: a red one near, at depth 0.2, drawn first, and a blue one far, at 0.8, drawn after. Without a depth test the last drawn wins; with `-Depth` the nearer does. The depth target is Depth32Float, cleared to 1, the far end, and a fragment passes when it is nearer than what is there:

```powershell
PS> $pair = @{
>>     position = [float[]](-1, -1, 0.2, 3, -1, 0.2, -1, 3, 0.2, -1, -1, 0.8, 3, -1, 0.8, -1, 3, 0.8)
>>     color = [float[]](1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 1)
>> }
PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair))[0..3] -join ','
0,0,255,255

PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair -Depth))[0..3] -join ','
255,0,0,255
```

`-DepthCompare` names the comparison a fragment's depth must pass against the depth there, `Less` when omitted, and `-ClearDepth` the depth the target starts at, 1 when omitted. With `Greater` the farther fragment wins, and nothing is farther than 1, so the target starts at 0 for it:

```powershell
PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair -Depth -DepthCompare Greater))[0..3] -join ','
0,0,0,255

PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair -Depth -DepthCompare Greater -ClearDepth 0))[0..3] -join ','
0,0,255,255
```

`-DepthWrite:$false` tests each fragment without writing its depth, so the target stays at 1, every fragment passes, and the last one drawn wins:

```powershell
PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair -Depth -DepthWrite:$false))[0..3] -join ','
0,0,255,255
```

`-DepthFormat` names the depth target's format in place of Depth32Float: Stencil8, Depth16Unorm, Depth24Plus, Depth24PlusStencil8, Depth32Float, or Depth32FloatStencil8 on an adapter with that feature. `-DepthBias`, `-DepthBiasSlopeScale` and `-DepthBiasClamp` move each triangle's depth before the test, which keeps the surfaces of a shadow map from shadowing themselves.

## Mask with a stencil test

A depth target of Stencil8, Depth24PlusStencil8 or Depth32FloatStencil8 holds a stencil value for each pixel, which starts at `-ClearStencil`, 0 when omitted. A stencil test compares `-StencilReference`, 0 when omitted, with that value by `-StencilCompare`, and changes the value by `-StencilFail`, `-StencilDepthFail` or `-StencilPass`, as the stencil test and the depth test turn out. In one draw that keeps each pixel to the first fragment drawn there: the two triangles of `$pair`, added by an additive blend, make magenta, and a test that passes only where the stencil is still 0 and adds 1 to it keeps the red one, drawn first:

```powershell
PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair -Blend Additive))[0..3] -join ','
255,0,255,255

PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair -Blend Additive -DepthFormat Stencil8 -StencilCompare Equal -StencilPass IncrementClamp))[0..3] -join ','
255,0,0,255
```

Triangles facing away take the same test unless `-BackStencilCompare`, `-BackStencilFail`, `-BackStencilDepthFail` or `-BackStencilPass` names their own, and `-StencilReadMask` and `-StencilWriteMask` keep the test to some of the value's bits. `Invoke-WgpuRender` starts its stencil afresh each call, so a mask one draw writes for another is drawn in a render pass of your own, as [Recording](Recording.md#draw-in-a-render-pass) shows.

## Blend and cull

`-Blend` decides how a fragment combines with what the target holds: `Replace`, the default, `Alpha`, `Additive`, `Premultiplied`, `Constant` or `Custom`. Half-transparent blue over white, by alpha:

```powershell
PS> Get-Content ./glass.wgsl
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 0.0, 1.0, 0.5); }
PS> $glass = Get-Content ./glass.wgsl -Raw
PS> (Read-WgpuTexture (Invoke-WgpuRender $glass 2 2 -VertexCount 3 -ClearColor 1, 1, 1, 1 -Blend Alpha))[0..3] -join ','
128,128,255,255
```

`Constant` mixes the fragment over the target by the color `-BlendConstant` gives, as red, green, blue and alpha. A constant of a quarter keeps a quarter of the fragment and three quarters of what was there. A blend that reads the constant needs `-BlendConstant`, and a blend that reads none refuses it. A pass recorded by hand sets the constant with `SetBlendConstant`; [Recording](Recording.md) shows passes in full:

```powershell
PS> (Read-WgpuTexture (Invoke-WgpuRender $glass 2 2 -VertexCount 3 -ClearColor 1, 1, 1, 1 -Blend Constant -BlendConstant 0.25, 0.25, 0.25, 0.25))[0..3] -join ','
191,191,255,223

PS> Invoke-WgpuRender $glass 2 2 -VertexCount 3 -Blend Constant
Invoke-WgpuRender: the blend reads a constant: give -BlendConstant
```

`Custom` names the parts of the blend: `-BlendSource` is what the fragment's color is multiplied by, `-BlendDestination` what the color under it is multiplied by, and `-BlendOperation` how the two combine, one of the [Wgpu.BlendFactor](Types.md#wgpublendfactor) and [Wgpu.BlendOperation](Types.md#wgpublendoperation) values. `-AlphaBlendSource`, `-AlphaBlendDestination` and `-AlphaBlendOperation` do the same for alpha, and take the color's when omitted. Blue taken from white leaves yellow, with the larger alpha of the two:

```powershell
PS> $subtract = @{ Blend = 'Custom'; BlendSource = 'One'; BlendDestination = 'One'; BlendOperation = 'ReverseSubtract'; AlphaBlendOperation = 'Max' }
PS> (Read-WgpuTexture (Invoke-WgpuRender $glass 2 2 -VertexCount 3 -ClearColor 1, 1, 1, 1 @subtract))[0..3] -join ','
255,255,0,255
```

A triangle whose corners turn counterclockwise faces the viewer. `-CullMode Front` discards the triangles that face the viewer and `Back` the others; the covering triangle turns counterclockwise. Culled, it leaves the clear color, opaque black; drawn with Replace, it keeps the shader's alpha of one half as it is:

```powershell
PS> (Read-WgpuTexture (Invoke-WgpuRender $glass 2 2 -VertexCount 3 -CullMode Front))[0..3] -join ','
0,0,0,255

PS> (Read-WgpuTexture (Invoke-WgpuRender $glass 2 2 -VertexCount 3 -CullMode Back))[0..3] -join ','
0,0,255,127
```

## Smooth the edges

`-SampleCount 4` draws with four samples for each pixel and resolves them into the target, so a pixel an edge crosses takes the share of it the triangle covers. This triangle's long edge runs through the centers of the pixels on the target's diagonal, the top-left one among them, so with one sample that pixel is either drawn or not, and with four it is partly covered:

```powershell
PS> Get-Content ./half.wgsl
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 1.0, 1.0, 1.0); }
PS> $half = Get-Content ./half.wgsl -Raw
PS> (Read-WgpuTexture (Invoke-WgpuRender $half 4 4 -VertexCount 3))[0..3] -join ','
0,0,0,255

PS> (Read-WgpuTexture (Invoke-WgpuRender $half 4 4 -VertexCount 3 -SampleCount 4))[0..3] -join ','
127,127,127,255
```

## Build a render pipeline once

`New-WgpuRenderPipeline` builds the shader once for its target format, its depth and stencil tests, its blending and its sample count, which every draw through it then uses, and lists the vertex inputs it reads in location order:

```powershell
PS> $pipeline = New-WgpuRenderPipeline $colored
PS> $pipeline
Label             VertexEntryPoint FragmentEntryPoint TargetFormats
-----             ---------------- ------------------ -------------
Wgpu shader vs/fs vs               fs                 {Rgba8Unorm}

PS> $pipeline.VertexInputs
position
color

PS> New-WgpuRenderPipeline $colored -Format Rgba16Float -Blend Alpha -Depth -SampleCount 4 | Select-Object TargetFormats, DepthFormat, SampleCount
TargetFormats DepthFormat  SampleCount
------------- -----------  -----------
{Rgba16Float} Depth32Float           4

PS> New-WgpuRenderPipeline '@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(); }'
New-WgpuRenderPipeline: the shader has no Fragment entry point
```

`Invoke-WgpuRender` draws through a pipeline without building it again. A render pass recorded by hand draws through it too, with a vertex buffer for each input in location order; [Recording](Recording.md) shows passes in full:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $target = New-WgpuTexture 2 2
PS> $red = @{ position = [float[]](-1, -1, 0, 3, -1, 0, -1, 3, 0); color = [float[]](1, 0, 0, 1, 0, 0, 1, 0, 0) }
PS> $null = Invoke-WgpuRender $pipeline -Target $target -Vertex $red
PS> (Read-WgpuTexture $target)[0..3] -join ','
255,0,0,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $target = New-WgpuTexture 2 2
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $positions = $gpu.CreateBuffer(9, 'Float32', @('Vertex', 'CopyDst'), 'positions')
PS> $positions.Write([float[]](-1, -1, 0, 3, -1, 0, -1, 3, 0))
PS> $colors = $gpu.CreateBuffer(9, 'Float32', @('Vertex', 'CopyDst'), 'colors')
PS> $colors.Write([float[]](1, 0, 0, 1, 0, 0, 1, 0, 0))
PS> $encoder = $gpu.CreateCommandEncoder('draw')
PS> $pass = $encoder.BeginRenderPass($target)
PS> $pass.SetPipeline($pipeline)
PS> $pass.SetVertexBuffer(0, $positions)
PS> $pass.SetVertexBuffer(1, $colors)
PS> $pass.Draw(3)
PS> $pass.End()
PS> $gpu.Submit($encoder.Finish())
PS> $target.Read()[0..3] -join ','
255,0,0,255
```
{{< /tab >}}
{{< /tabs >}}

A shader that holds one stage cannot draw, so GLSL, which holds one stage, is refused:

```powershell
PS> Invoke-WgpuRender "#version 450`nvoid main() {}" 2 2 -VertexCount 3
Invoke-WgpuRender: the shader has no Vertex entry point
```
