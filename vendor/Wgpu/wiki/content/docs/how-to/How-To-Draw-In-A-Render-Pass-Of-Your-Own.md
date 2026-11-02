---
title: How To Draw In A Render Pass Of Your Own
weight: 14
---

`Invoke-WgpuRender` builds a pipeline, draws once and waits. Drawing in a render pass of your own builds the pipeline and the buffers once, draws as many times as a frame needs with whatever changes between draws, and submits the frame in one go. This draws one scene twice into one picture: the whole of it on the left, and a close view of its middle on the right, kept inside a frame. Then a compute shader decides how many shapes the next frame draws, and a copy keeps the first frame.

## Build the pipeline and the buffers once

The scene is sixteen squares in a grid, each turned a little more than the one before and colored by its number, the same square drawn as sixteen instances. Each square is at a depth of its number over 16, so a lower number covers a higher one where they overlap. A uniform holds the view: the point at its middle, and how far it is zoomed in:

```powershell
PS> Get-Content ./tiles.wgsl
struct View { center: vec2<f32>, zoom: f32 }
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@group(0) @binding(0) var<uniform> view: View;
@vertex
fn vs(@location(0) corner: vec2<f32>, @builtin(instance_index) n: u32) -> Out {
    let cell = vec2<f32>(f32(n % 4u), f32(n / 4u)) - vec2<f32>(1.5);
    let turn = f32(n) * 0.2;
    let c = corner * 0.22;
    let turned = vec2<f32>(c.x * cos(turn) - c.y * sin(turn), c.x * sin(turn) + c.y * cos(turn));
    var out: Out;
    out.position = vec4<f32>((cell * 0.42 + turned - view.center) * view.zoom, f32(n) / 16.0, 1.0);
    let t = f32(n) / 15.0;
    out.color = vec3<f32>(t, 0.4 + 0.5 * sin(t * 3.0), 1.0 - t);
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
```

`New-WgpuRenderPipeline` builds the pipeline once, here with a depth test and four samples a pixel. It reads the vertex input and the binding from the shader:

```powershell
PS> $pipeline = New-WgpuRenderPipeline (Get-Content ./tiles.wgsl -Raw) -Depth -SampleCount 4
PS> $pipeline | Select-Object VertexInputs, TargetFormats, DepthFormat, SampleCount
VertexInputs TargetFormats DepthFormat  SampleCount
------------ ------------- -----------  -----------
{corner}     {Rgba8Unorm}  Depth32Float           4
```

The square's four corners and six indices, and the two views, each in a bind group of its own. A view is a uniform buffer of the struct's three numbers, and a fourth, 0, since WGSL rounds the struct's 12 bytes up to 16:

```powershell
PS> $corners = New-WgpuBuffer ([float[]](-1, -1, 1, -1, -1, 1, 1, 1))
PS> $indices = New-WgpuBuffer ([uint32[]](0, 1, 2, 2, 1, 3))
PS> $whole = $pipeline.BindGroup(0, @{ view = (New-WgpuBuffer ([float[]](0, 0, 1, 0))) })
PS> $close = $pipeline.BindGroup(0, @{ view = (New-WgpuBuffer ([float[]](-0.21, -0.21, 2.5, 0))) })
```

## Make the targets

A pipeline with four samples draws into a texture with four samples, and its depth test needs a Depth32Float texture with as many. The pass resolves the samples into a plain texture, the picture:

```powershell
PS> $samples = New-WgpuTexture 256 128 -SampleCount 4 -Usage RenderAttachment
PS> $depth = New-WgpuTexture 256 128 -Format Depth32Float -SampleCount 4 -Usage RenderAttachment
PS> $frame = New-WgpuTexture 256 128 -Usage RenderAttachment, CopySrc
```

## Draw both views in one pass

The pass clears the targets to white and the farthest depth, then draws twice. A viewport maps a draw onto part of the target, its left half and then its right half, and a scissor rectangle keeps the second draw inside a frame 8 pixels in from the right half's edges. Between the draws only the viewport, the scissor and the bind group change:

```powershell
PS> $encoder = New-WgpuCommandEncoder -Label frame
PS> $pass = $encoder.BeginRenderPass($samples, @(1, 1, 1, 1), $depth, $frame)
PS> $pass.SetPipeline($pipeline)
PS> $pass.SetVertexBuffer(0, $corners)
PS> $pass.SetIndexBuffer($indices)
PS> $pass.SetViewport(0, 0, 128, 128)
PS> $pass.SetBindGroup(0, $whole)
PS> $pass.DrawIndexed(6, 16)
PS> $pass.SetViewport(128, 0, 128, 128)
PS> $pass.SetScissorRect(136, 8, 112, 112)
PS> $pass.SetBindGroup(0, $close)
PS> $pass.DrawIndexed(6, 16)
PS> $pass.End()
PS> Submit-WgpuCommand $encoder.Finish()
PS> Export-WgpuImage $frame ./two-views.png
```

![Two views of sixteen turned squares, blue at the bottom to orange at the top: the whole grid on the left, and the middle squares close up on the right, inside a white frame](/images/two-views.png)

The pixel at the top left of the right half is outside the scissor rectangle, so it keeps the white the pass cleared it to:

```powershell
PS> ($frame | Resize-WgpuTexture -Crop 130, 2, 1, 1 | Read-WgpuTexture) -join ','
255,255,255,255
```

## Let the GPU decide how many to draw

`DrawIndexedIndirect` reads its index count, instance count, first index, base vertex and first instance from a buffer on the GPU, so a compute shader can decide what a draw draws, and the CPU never reads it. This shader counts the sizes above a half, adding one to the instance count for each:

```powershell
PS> Get-Content ./count.wgsl
@group(0) @binding(0) var<storage, read> sizes: array<f32>;
@group(0) @binding(1) var<storage, read_write> draw: array<atomic<u32>, 5>;
@compute @workgroup_size(16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&sizes) && sizes[id.x] > 0.5) { atomicAdd(&draw[1], 1u); }
}
PS> $sizes = New-WgpuBuffer ([float[]](0.9, 0.2, 0.7, 0.6, 0.1, 0.8, 0.3, 0.95, 0.4, 0.55, 0.05, 0.75, 0.35, 0.65, 0.15, 0.85))
PS> $draw = New-WgpuBuffer ([uint32[]](6, 0, 0, 0, 0))
PS> Invoke-WgpuCompute (Get-Content ./count.wgsl -Raw) @{ sizes = $sizes; draw = $draw } -Workgroups 1
PS> (Read-WgpuBuffer $draw) -join ','
6,9,0,0,0
```

The next frame draws that many squares in the left half, reading the count where the shader left it:

```powershell
PS> $encoder = New-WgpuCommandEncoder -Label counted
PS> $pass = $encoder.BeginRenderPass($samples, @(1, 1, 1, 1), $depth, $frame)
PS> $pass.SetPipeline($pipeline)
PS> $pass.SetVertexBuffer(0, $corners)
PS> $pass.SetIndexBuffer($indices)
PS> $pass.SetViewport(0, 0, 128, 128)
PS> $pass.SetBindGroup(0, $whole)
PS> $pass.DrawIndexedIndirect($draw, 0)
PS> $pass.End()
```

## Keep the first frame

The next frame draws into the same texture, so a copy keeps the first one. `CopyTextureToTexture` copies between textures of one size and format. Recorded in an encoder of its own and submitted before the next frame, it copies the first frame before the next one clears it:

```powershell
PS> $kept = New-WgpuTexture 256 128
PS> $copy = New-WgpuCommandEncoder -Label keep
PS> $copy.CopyTextureToTexture($frame, $kept)
PS> Submit-WgpuCommand $copy.Finish(), $encoder.Finish()
PS> $first = [byte[]](Read-WgpuTexture (Import-WgpuImage ./two-views.png))
PS> [Linq.Enumerable]::SequenceEqual([byte[]](Read-WgpuTexture $kept), $first)
True

PS> [Linq.Enumerable]::SequenceEqual([byte[]](Read-WgpuTexture $frame), $first)
False
```

Command buffers submitted together run in the order given, so the kept texture holds the first frame as it was saved, and the frame texture now holds the counted one.

[Draw in a render pass](Recording.md#draw-in-a-render-pass) has the rest of a pass's methods, `DrawIndirect`, `SetBlendConstant` and `SetStencilReference`, and the hashtable `BeginRenderPass` takes in place of its arguments, which can load what a texture holds instead of clearing it.
