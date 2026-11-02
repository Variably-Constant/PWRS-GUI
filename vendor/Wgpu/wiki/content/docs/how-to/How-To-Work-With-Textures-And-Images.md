---
title: How To Work With Textures And Images
weight: 8
---

Make a texture on the GPU and sample it as a renderer does: two kinds of floor tile written by a compute shader into the layers of one texture, given mip levels so they stay smooth into the distance, and drawn as an endless floor through three samplers, to see what each sampler setting does. [Textures and images](Images.md) has each part on its own, and [How To Use An Existing Image](How-To-Use-An-Existing-Image.md) starts from a picture on disk.

## Write the tiles with a compute shader

A compute shader writes a storage texture, one invocation for each texel. These tiles are 16 texels square, light and dark in turn, with a line of grout along their top and left edges. A texture can hold layers of one size and format; this one holds warm tiles in layer 0 and cool ones in layer 1, and the third coordinate of each invocation is the layer it writes:

```powershell
PS> Get-Content ./tiles.wgsl
@group(0) @binding(0) var tiles: texture_storage_2d_array<rgba8unorm, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(tiles);
    if (id.x >= size.x || id.y >= size.y) { return; }
    var shade = select(0.3, 0.8, (id.x / 16u + id.y / 16u) % 2u == 0u);
    if (id.x % 16u == 0u || id.y % 16u == 0u) { shade = 0.1; }
    let tint = select(vec3<f32>(1.0, 0.85, 0.7), vec3<f32>(0.7, 0.85, 1.0), id.z == 1u);
    textureStore(tiles, vec2<i32>(id.xy), id.z, vec4<f32>(shade * tint, 1.0));
}
```

The texture is 128 texels square and two layers deep, with room for the 8 mip levels that halve it down to a single texel. A storage binding writes one mip level, so the shader is given a view of level 0, both layers:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $tiles = New-WgpuTexture 128 128 -Layers 2 -MipLevels 8
PS> $level0 = $tiles.CreateView($null, 'D2Array', 0, 1)
PS> Invoke-WgpuCompute (Get-Content ./tiles.wgsl -Raw) @{ tiles = $level0 } -Invocations 128, 128, 2
PS> Export-WgpuImage $tiles ./tiles.png
PS> Export-WgpuImage $tiles ./tiles-cool.png -Layer 1
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $tiles = New-WgpuTexture 128 128 -Layers 2 -MipLevels 8
PS> $level0 = $tiles.CreateView($null, 'D2Array', 0, 1)
PS> $writer = [Wgpu.Device]::GetDefault().CreateComputePipeline((Get-Content ./tiles.wgsl -Raw), $null, $null, $null)
PS> $writer.Dispatch(@{ tiles = $level0 }, 16, 16, 2)
PS> $tiles.Save((Join-Path $PWD tiles.png))
```

`Dispatch` counts workgroups, 8 by 8 texels here, and `Save` writes layer 0.
{{< /tab >}}
{{< /tabs >}}

![Warm tiles: light and dark squares in turn, each with dark grout along its top and left edges](/images/tiles.png) ![The same tiles in cool blue](/images/tiles-cool.png)

## Give it mip levels

`GenerateMips` renders mip levels 1 to 7 of both layers on the GPU, each texel the average of the four under it in the level above. By level 4 a texel covers a whole tile, grout and all, and level 7 is a single texel, the average color of its layer, which the far floor should fade to:

```powershell
PS> $tiles.GenerateMips()
PS> (Read-WgpuTexture $tiles -MipLevel 7) -join ','
126,107,89,255

PS> (Read-WgpuTexture $tiles -MipLevel 7 -Layer 1) -join ','
89,107,126,255
```

## Draw the floor

The vertex shader makes one triangle that covers the target, and the fragment shader finds the point of an endless floor under each pixel, with the horizon three quarters of the way up: the nearer a pixel is to the horizon, the farther away its point, so the tiles shrink toward it. The floor's coordinates run on past the texture's edges, and the sampler repeats it. A path of the cool tiles in layer 1 runs down the middle to the horizon, between the warm ones:

```powershell
PS> Get-Content ./floor.wgsl
@group(0) @binding(0) var tiles: texture_2d_array<f32>;
@group(0) @binding(1) var tile_sampler: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) screen: vec2<f32> }
@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    return Out(vec4<f32>(p, 0.0, 1.0), p);
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> {
    let below = max(0.5 - input.screen.y, 0.001);
    let uv = vec2<f32>(input.screen.x, 1.0) / below;
    let layer = select(0, 1, abs(uv.x) < 0.5);
    let ground = textureSample(tiles, tile_sampler, uv, layer);
    return select(ground, vec4<f32>(0.6, 0.75, 0.95, 1.0), input.screen.y > 0.5);
}
PS> $floor = Get-Content ./floor.wgsl -Raw
```

First through a sampler held to level 0, as if the texture had no mip levels. Near the bottom each pixel blends a few texels and the tiles are sharp; toward the horizon one pixel spans many tiles but samples a single point of them, and the grout and the tiles break up into a pattern of their own:

```powershell
PS> $level0Only = New-WgpuSampler -Filter Linear -AddressMode Repeat -LodMaxClamp 0
PS> Invoke-WgpuRender $floor 256 128 -VertexCount 3 -Binding @{ tiles = $tiles; tile_sampler = $level0Only } -OutFile ./floor-level0.png
```

![The floor sampled from level 0 alone: sharp tiles near the bottom, breaking up into a busy pattern toward the horizon](/images/floor-level0.png)

With every level in reach, the GPU picks for each pixel the level whose texels are about the size of the pixel, and `-Filter Linear` blends the two nearest levels as well as the texels in each. The far floor fades smoothly to the average colors, but sooner than it needs to: a pixel looking along the floor covers far more of its depth than its width, and the level is picked for the longer side:

```powershell
PS> $mipmapped = New-WgpuSampler -Filter Linear -AddressMode Repeat
PS> Invoke-WgpuRender $floor 256 128 -VertexCount 3 -Binding @{ tiles = $tiles; tile_sampler = $mipmapped } -OutFile ./floor-mipmapped.png
```

![The floor sampled with mip levels: the far tiles fade smoothly into their average colors](/images/floor-mipmapped.png)

`-Anisotropy 16` lets the GPU take up to 16 samples along the longer side, each from a level picked for the shorter one, so the tiles keep their lines much farther back:

```powershell
PS> $anisotropic = New-WgpuSampler -Filter Linear -AddressMode Repeat -Anisotropy 16
PS> Invoke-WgpuRender $floor 256 128 -VertexCount 3 -Binding @{ tiles = $tiles; tile_sampler = $anisotropic } -OutFile ./floor-anisotropic.png
```

![The floor sampled with mip levels and anisotropic filtering: the tiles keep their lines farther toward the horizon](/images/floor-anisotropic.png)

[Sample a texture](Images.md#sample-a-texture) has the address modes and filters on their own, [Give a texture mip levels](Images.md#give-a-texture-mip-levels) the filters `GenerateMips` takes, and [Bind part of a texture](Images.md#bind-part-of-a-texture) more on views.
