---
title: How To Pack Channels Into One Texture
weight: 10
---

A material often comes as several grayscale masks, and a renderer reads them faster from one texture, one mask in each channel, so a shader samples once instead of three times. This packs an occlusion, a roughness and a metalness mask into the red, green and blue of one texture, the layout glTF uses, reads them back in a shader, builds the same texture from another packed one, and takes a mask out again. The masks are [occlusion.png](/images/occlusion.png), [roughness.png](/images/roughness.png) and [metalness.png](/images/metalness.png), 64 by 64 texels each:

![The occlusion mask: white in the middle, darkening toward the corners](/images/occlusion.png) ![The roughness mask: dark and light bands across](/images/roughness.png) ![The metalness mask: a black and white checkerboard](/images/metalness.png)

## Load the masks

A gray PNG loads with its value in red, green and blue alike:

```powershell
PS> $occlusion = Import-WgpuImage ./occlusion.png
PS> $roughness = Import-WgpuImage ./roughness.png
PS> $metalness = Import-WgpuImage ./metalness.png
PS> $occlusion, $roughness, $metalness | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
   64     64 Rgba8Unorm
   64     64 Rgba8Unorm
   64     64 Rgba8Unorm
```

## Pack them

`Join-WgpuTexture` reads the red channel of each texture it is given and writes it into the channel it was given for. Alpha, given nothing, is 1:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $packed = Join-WgpuTexture -Red $occlusion -Green $roughness -Blue $metalness
PS> $packed | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
   64     64 Rgba8Unorm

PS> Export-WgpuImage $packed ./packed.png
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $packed = [Wgpu.Texture]::Join($occlusion, $roughness, $metalness, $null, $null)
PS> $packed | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
   64     64 Rgba8Unorm

PS> $packed.Save((Join-Path $PWD packed.png))
```
{{< /tab >}}
{{< /tabs >}}

![The packed texture shown as colors: blue and purple squares where the metal is, red, yellow and green ones between them](/images/packed.png)

The colors mean nothing to the eye: each channel is a mask of its own. Masks of different sizes are packed at the largest one's size, the smaller ones resized to it first.

## Read them in a shader

A shader reads the three masks from one texel. This one reads the texel in the top left corner, where the occlusion is darkest, the roughness band is low and the checkerboard is metal:

```powershell
PS> Get-Content ./material.wgsl
@group(0) @binding(0) var material: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> values: array<f32>;
@compute @workgroup_size(1)
fn main() {
    let texel = textureLoad(material, vec2<i32>(0, 0), 0);
    values[0] = texel.r;
    values[1] = texel.g;
    values[2] = texel.b;
}
PS> Invoke-WgpuCompute (Get-Content ./material.wgsl -Raw) @{ material = $packed; values = [float[]]::new(3) } -Workgroups 1
0.2980392
0.2
1
```

Occlusion, roughness and metalness, each from 0 to 1.

## Build it from another packed texture

glTF's metallic-roughness textures keep roughness in green and metalness in blue, and leave red to the occlusion texture. A channel letter beside a texture takes that channel of it instead of red, so the occlusion mask and such a texture make the same packed texture:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $metallicRoughness = Join-WgpuTexture -Green $roughness -Blue $metalness
PS> $rebuilt = Join-WgpuTexture -Red $occlusion -Green ($metallicRoughness, 'g') -Blue ($metallicRoughness, 'b')
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $metallicRoughness = [Wgpu.Texture]::Join(0, $roughness, $metalness, $null, $null)
PS> $rebuilt = [Wgpu.Texture]::Join($occlusion, @($metallicRoughness, 'g'), @($metallicRoughness, 'b'), $null, $null)
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> [Linq.Enumerable]::SequenceEqual([byte[]](Read-WgpuTexture $rebuilt), [byte[]](Read-WgpuTexture $packed))
True
```

A number in place of a texture fills its channel with that value, as the 0 in red does for the Methods tab here.

## Take a mask out again

`-Swizzle` rearranges a texture's channels by four letters, one for each channel of the new texture. `ggg1` puts green in red, green and blue and 1 in alpha, which gives back the roughness mask exactly as it loaded:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $unpacked = $packed | Convert-WgpuTexture -Swizzle ggg1
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $unpacked = $packed.Swizzle('ggg1')
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> [Linq.Enumerable]::SequenceEqual([byte[]](Read-WgpuTexture $unpacked), [byte[]](Read-WgpuTexture $roughness))
True
```

## Compress the packed texture

`-Format` packs straight into another format, here BC7, which the GPU decodes as it samples: one byte for each texel in place of four, so 4 KB of GPU memory for these 64 by 64 texels in place of 16. A `.ktx2` file keeps it as it is, and `-Zstd` compresses the file further:

```powershell
PS> $compressed = Join-WgpuTexture -Red $occlusion -Green $roughness -Blue $metalness -Format Bc7RgbaUnorm
PS> $compressed.Format
Bc7RgbaUnorm

PS> Export-WgpuImage $compressed ./packed.ktx2
PS> Export-WgpuImage $compressed ./packed-zstd.ktx2 -Zstd
PS> Get-Item ./packed.png, ./packed.ktx2, ./packed-zstd.ktx2 | Select-Object Name, Length
Name             Length
----             ------
packed.png         4507
packed.ktx2        4272
packed-zstd.ktx2   2701
```

The PNG file is small too, but it is no use to the GPU as it is: the texture it loads into takes the full 16 KB.

[How To Prepare Textures For A Renderer](How-To-Prepare-Textures-For-A-Renderer.md) gives a texture its mip levels before it is compressed, and [Change a texture's channels and colors](Images.md#change-a-textures-channels-and-colors) has every form `Join-WgpuTexture` takes.
