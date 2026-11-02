---
title: How To Use An Existing Image
weight: 7
---

Load a picture that is already on disk into a texture, run a compute shader over it, draw it on a shape by sampling it in a fragment shader, and save what comes out as image files. The picture here is [scene.png](/images/scene.png), 256 by 160 pixels:

![A landscape: a blue sky fading to white at the horizon, a pale yellow sun at the top right, and two green hills](/images/scene.png)

## Load it

`Import-WgpuImage` reads the file into a texture on the GPU. An 8-bit PNG loads as Rgba8Unorm: red, green, blue and alpha, 8 bits each, which a shader reads as numbers from 0 to 1:

```powershell
PS> $scene = Import-WgpuImage ./scene.png
PS> $scene | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
  256    160 Rgba8Unorm
```

[Load and save image files](Images.md#load-and-save-image-files) lists the formats it reads.

## Run a compute shader over it

This shader finds the edges in the picture. One run for each texel weighs the brightness of the 3 by 3 texels around it with Sobel's weights, across and down, and writes how sharply the brightness changes there, from black where it is even to white at a sharp edge:

```powershell
PS> Get-Content ./edges.wgsl
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var edges: texture_storage_2d<rgba8unorm, write>;
fn brightness(x: i32, y: i32) -> f32 {
    let size = vec2<i32>(textureDimensions(source));
    let c = textureLoad(source, clamp(vec2<i32>(x, y), vec2<i32>(0), size - 1), 0).rgb;
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(source);
    if (id.x >= size.x || id.y >= size.y) { return; }
    let x = i32(id.x);
    let y = i32(id.y);
    let across = brightness(x + 1, y - 1) + 2.0 * brightness(x + 1, y) + brightness(x + 1, y + 1)
        - brightness(x - 1, y - 1) - 2.0 * brightness(x - 1, y) - brightness(x - 1, y + 1);
    let down = brightness(x - 1, y + 1) + 2.0 * brightness(x, y + 1) + brightness(x + 1, y + 1)
        - brightness(x - 1, y - 1) - 2.0 * brightness(x, y - 1) - brightness(x + 1, y - 1);
    let edge = min(length(vec2<f32>(across, down)), 1.0);
    textureStore(edges, vec2<i32>(id.xy), vec4<f32>(edge, edge, edge, 1.0));
}
```

The texture it writes is a storage texture, so it needs the StorageBinding usage, and CopySrc to be saved. The cmdlet counts one run for each texel; the pipeline's `Dispatch` counts workgroups, which are 8 by 8 texels here:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $edges = New-WgpuTexture $scene.Width $scene.Height -Usage StorageBinding, CopySrc
PS> Invoke-WgpuCompute (Get-Content ./edges.wgsl -Raw) @{ source = $scene; edges = $edges } -Invocations $scene.Width, $scene.Height
PS> Export-WgpuImage $edges ./edges.png
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $finder = $gpu.CreateComputePipeline((Get-Content ./edges.wgsl -Raw), $null, $null, $null)
PS> $edges = New-WgpuTexture $scene.Width $scene.Height -Usage StorageBinding, CopySrc
PS> $finder.Dispatch(@{ source = $scene; edges = $edges }, $scene.Width / 8, $scene.Height / 8, 1)
PS> $edges.Save((Join-Path $PWD edges.png))
```
{{< /tab >}}
{{< /tabs >}}

![The edges of the landscape: the rim of the sun and the top of the far hill in white, the edge of the near hill in gray, the sky a faint dark gray and the hills black](/images/edges.png)

The sky's gradient changes slowly, so it shows only as a faint gray, under 5 percent of white. The rim of the sun and the top of the far hill are white, and the near hill's edge is gray, since its two greens are close in brightness. [Process an image on the GPU](Images.md#process-an-image-on-the-gpu) has more on storage textures.

## Draw it on a shape

A fragment shader reads a picture through a sampler, which blends the texels around the point it asks for, so the picture can be drawn at any size and angle. This shader draws it on a card 200 by 125 pixels, turned 12 degrees, in a 320 by 240 target. The vertex shader takes the card's corners, from -1 to 1, and gives each the point of the picture it shows, from 0 to 1, top left first:

```powershell
PS> Get-Content ./card.wgsl
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
@vertex
fn vs(@location(0) corner: vec2<f32>) -> Out {
    let turn = radians(12.0);
    let p = corner * vec2<f32>(100.0, 62.5);
    let turned = vec2<f32>(p.x * cos(turn) - p.y * sin(turn), p.x * sin(turn) + p.y * cos(turn));
    var out: Out;
    out.position = vec4<f32>(turned / vec2<f32>(160.0, 120.0), 0.0, 1.0);
    out.uv = vec2<f32>(corner.x + 1.0, 1.0 - corner.y) / 2.0;
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return textureSample(picture, picture_sampler, input.uv); }
```

Four corners and six indices make the card of two triangles. A linear sampler blends neighboring texels where the card is smaller than the picture, `-SampleCount 4` smooths the card's slanted edges, and `-PassThru` writes the texture as well as saving the file:

```powershell
PS> $corners = @{ corner = [float[]](-1, -1, 1, -1, -1, 1, 1, 1) }
PS> $look = @{ picture = $scene; picture_sampler = (New-WgpuSampler -Filter Linear) }
PS> $card = Invoke-WgpuRender (Get-Content ./card.wgsl -Raw) 320 240 -Vertex $corners -Index ([uint32[]](0, 1, 2, 2, 1, 3)) -Binding $look -SampleCount 4 -ClearColor 0.9, 0.9, 0.9 -OutFile ./card.png -PassThru
PS> $card | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
  320    240 Rgba8Unorm
```

![The landscape on a card turned 12 degrees counterclockwise, on a light gray background](/images/card.png)

[Feed vertices from arrays](Rendering.md#feed-vertices-from-arrays), [Draw by indices, draw instances](Rendering.md#draw-by-indices-draw-instances) and [Sample a texture](Images.md#sample-a-texture) say more about each part.

## Save it in another format

`Export-WgpuImage` writes the image format the file's extension names:

```powershell
PS> Export-WgpuImage $card ./card.webp
PS> Export-WgpuImage $card ./card.jpg
PS> Get-Item ./card.png, ./card.webp, ./card.jpg | Select-Object Name, Length
Name      Length
----      ------
card.png   21708
card.webp  14764
card.jpg    6494
```

WebP keeps every texel, as PNG does, in less room. JPEG takes the least room and gives up some detail, and it holds no alpha. Loading each file back and comparing its texels with the card's shows which kept them:

```powershell
PS> $drawn = [byte[]](Read-WgpuTexture $card)
PS> [Linq.Enumerable]::SequenceEqual([byte[]](Read-WgpuTexture (Import-WgpuImage ./card.webp)), $drawn)
True

PS> [Linq.Enumerable]::SequenceEqual([byte[]](Read-WgpuTexture (Import-WgpuImage ./card.jpg)), $drawn)
False
```
