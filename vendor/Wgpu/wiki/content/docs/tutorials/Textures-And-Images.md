---
title: Textures And Images
weight: 5
---

A texture is a grid of texels on the GPU, in one format, which a shader reads by its position. This chapter makes a texture from texels, saves it as an image file and loads it back, runs a shader over every texel, and resizes it.

## A texture of texels

`New-WgpuTexture` makes a 2D texture of the default format, Rgba8Unorm: four channels, red, green, blue and alpha, of 8 bits each. `-Data` gives its texels row by row, four numbers for each, from 0 to 255:

```powershell
PS> $pixels = New-WgpuTexture 2 2 -Data ([byte[]](255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255))
PS> $pixels
Label          Width Height Format
-----          ----- ------ ------
2x2 Rgba8Unorm     2      2 Rgba8Unorm

PS> (Read-WgpuTexture $pixels) -join ','
255,0,0,255,0,255,0,255,0,0,255,255,255,255,255,255
```

The top row is red and green, the bottom row blue and white. A shader reads each channel of an Rgba8Unorm texel as a float from 0 to 1, so 255 reads as 1.0; that is what Unorm, unsigned normalized, means.

## To an image file and back

`Export-WgpuImage` saves a texture as the image format its file's extension names, and `Import-WgpuImage` loads an image into a new texture. A .NET method resolves a relative path against the process's working directory, which PowerShell's location does not change, so the `Save` method takes the file's full path:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> Export-WgpuImage $pixels ./tiny.png
PS> Get-Item ./tiny.png | Select-Object Name, Length
Name     Length
----     ------
tiny.png     86
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $pixels.Save((Join-Path $PWD tiny.png))
PS> Get-Item ./tiny.png | Select-Object Name, Length
Name     Length
----     ------
tiny.png     86
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> $image = Import-WgpuImage ./tiny.png
PS> $image
Label                      Width Height Format
-----                      ----- ------ ------
C:\Temp\wgpu-demo\tiny.png     2      2 Rgba8Unorm

PS> (Read-WgpuTexture $image) -join ','
255,0,0,255,0,255,0,255,0,0,255,255,255,255,255,255
```

The image holds the same texels, and its label is the file it came from. PNG, JPEG, WebP, TIFF, BMP, OpenEXR and the other formats [Textures and images](Images.md) lists load the same way.

## A shader over every texel

A compute shader reads a texture through a `texture_2d` binding and writes one through a storage texture, one run for each texel. This one inverts the colors:

```powershell
PS> Get-Content ./invert.wgsl
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var inverted: texture_storage_2d<rgba8unorm, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(source);
    if (id.x < size.x && id.y < size.y) {
        let c = textureLoad(source, vec2<i32>(id.xy), 0);
        textureStore(inverted, vec2<i32>(id.xy), vec4<f32>(1.0 - c.rgb, c.a));
    }
}
```

The texture it writes needs the StorageBinding usage, and CopySrc to be read back. `-Invocations` gives the runs in two dimensions, the image's width and height:

```powershell
PS> $inverted = New-WgpuTexture $image.Width $image.Height -Usage StorageBinding, CopySrc
PS> Invoke-WgpuCompute (Get-Content ./invert.wgsl -Raw) @{ source = $image; inverted = $inverted } -Invocations $image.Width, $image.Height
PS> (Read-WgpuTexture $inverted) -join ','
0,255,255,255,255,0,255,255,255,255,0,255,0,0,0,255
```

Red became cyan, green magenta, blue yellow and white black, and the alpha stayed 255.

## Bigger and smaller

`Resize-WgpuTexture` makes a new texture of another size. `-Filter Nearest` repeats each texel, where the default filters blend them:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $bigger = Resize-WgpuTexture $image -Width 4 -Height 4 -Filter Nearest
PS> $bigger | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
    4      4 Rgba8Unorm

PS> (Read-WgpuTexture $bigger)[0..15] -join ','
255,0,0,255,255,0,0,255,0,255,0,255,0,255,0,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $bigger = $image.Resize(4, 4, 'Nearest')
PS> $bigger | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
    4      4 Rgba8Unorm

PS> $bigger.Read()[0..15] -join ','
255,0,0,255,255,0,0,255,0,255,0,255,0,255,0,255
```
{{< /tab >}}
{{< /tabs >}}

The first row of the bigger texture is red, red, green, green: each texel became two by two.

## Where next

[Drawing Offscreen](Drawing-Offscreen.md) draws shapes into a texture with a vertex and a fragment shader.
