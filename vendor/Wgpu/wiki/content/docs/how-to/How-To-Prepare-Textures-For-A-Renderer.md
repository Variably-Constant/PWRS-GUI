---
title: How To Prepare Textures For A Renderer
weight: 11
---

A renderer wants textures it can sample as they load: sides that halve evenly, every mip level made, edges that filter without dark fringes, normal maps for the bumps of a surface, and a compressed format the GPU decodes as it samples, all in one file. This takes a picture, a sprite, an HDR picture and a height map from the image files they are to KTX2 and DDS files a renderer loads whole: [scene.png](/images/scene.png), [sprite.png](/images/sprite.png), [scene.hdr](/images/scene.hdr) and [height.png](/images/height.png).

## Size it to a power of two

Each mip level halves the one above it down to 1 by 1, and the BC formats encode blocks of 4 by 4 texels, so sides that are powers of two halve evenly at every level, into whole blocks down to 4 by 4. The landscape is 256 by 160, and `-PowerOfTwo Nearest` makes it 256 by 128; a renderer that maps it onto a surface stretches it back:

```powershell
PS> $scene = Import-WgpuImage ./scene.png
PS> $scene | Resize-WgpuTexture -PowerOfTwo Nearest | Select-Object Width, Height
Width Height
----- ------
  256    128
```

`Import-WgpuImage` takes the same parameter, and resizes as the file loads.

## Make its mip levels

A texture drawn smaller than its size reads a smaller copy of itself, a mip level, so a distant surface does not shimmer. `-MipLevels 0` makes every level down to 1 by 1, and `-MipFilter Kaiser` makes each one from level 0 with a filter that keeps more detail than the plain average, the default:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $mipped = Import-WgpuImage ./scene.png -PowerOfTwo Nearest -MipLevels 0 -MipFilter Kaiser
PS> $mipped | Select-Object Width, Height, MipLevelCount
Width Height MipLevelCount
----- ------ -------------
  256    128             9

PS> (Read-WgpuTexture $mipped -MipLevel 8) -join ','
108,152,160,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $mipped = (Import-WgpuImage ./scene.png -PowerOfTwo Nearest).GenerateMips(0, 'Kaiser', $null)
PS> $mipped | Select-Object Width, Height, MipLevelCount
Width Height MipLevelCount
----- ------ -------------
  256    128             9

PS> $mipped.Read(8) -join ','
108,152,160,255
```
{{< /tab >}}
{{< /tabs >}}

The last level, 1 by 1, comes near the picture's average color of 103,149,157. A texture that tiles takes `-MipEdge Wrap` as well, so the filter reads across each edge from the other side; [Give a texture mip levels](Images.md#give-a-texture-mip-levels) has the other filters.

## Keep a cutout's edges clean

The sprite's texels outside its disc are transparent black. Making the levels averages that black into the disc's edge, which shows as a dark fringe when the sprite is drawn small. The 1 by 1 level shows it plainly, its color the average of the whole sprite, black included:

```powershell
PS> $plain = Import-WgpuImage ./sprite.png -MipLevels 0
PS> (Read-WgpuTexture $plain -MipLevel 7) -join ','
126,76,18,113
```

`-FixAlphaBorder` first gives each transparent texel the color of the nearest texel that is not transparent, and leaves its alpha at 0, so the levels average the sprite's own colors:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $fixed = Import-WgpuImage ./sprite.png -FixAlphaBorder -MipLevels 0
PS> (Read-WgpuTexture $fixed -MipLevel 7) -join ','
249,152,37,113
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $fixed = (Import-WgpuImage ./sprite.png).FixAlphaBorder($null).GenerateMips(0)
PS> $fixed.Read(7) -join ','
249,152,37,113
```
{{< /tab >}}
{{< /tabs >}}

The alpha is the same, the share of the sprite the disc covers, and the color is the disc's orange. A renderer that blends premultiplied colors takes `-Premultiply` instead: each color is multiplied by its alpha, so transparent black is the right color for a transparent texel, and averages to the right result:

```powershell
PS> (Read-WgpuTexture (Import-WgpuImage ./sprite.png -Premultiply -MipLevels 0) -MipLevel 7) -join ','
107,64,15,113
```

A cutout drawn with an alpha test rather than blended thins out in the distance as its levels average; `-KeepCoverage`, in [Give a texture mip levels](Images.md#give-a-texture-mip-levels), keeps it as solid at every level.

## Compress it

`-Format` encodes the texture as it loads, every level, on the GPU. A color picture takes the sRGB form of a format, so its levels are averaged in linear light and a shader reads linear colors. BC7 keeps red, green, blue and alpha in one byte a texel, a quarter of Rgba8's four. The first BC7 encode in a session also builds its encoder on the GPU, which makes it slower than the encodes after it:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $color = Import-WgpuImage ./scene.png -PowerOfTwo Nearest -MipLevels 0 -MipFilter Kaiser -Format Bc7RgbaUnormSrgb
PS> $color | Select-Object Width, Height, Format, MipLevelCount
Width Height Format           MipLevelCount
----- ------ ------           -------------
  256    128 Bc7RgbaUnormSrgb             9
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $color = (Import-WgpuImage ./scene.png -PowerOfTwo Nearest -Srgb).GenerateMips(0, 'Kaiser', $null).Compress('Bc7RgbaUnormSrgb', $null, $null)
PS> $color | Select-Object Width, Height, Format, MipLevelCount
Width Height Format           MipLevelCount
----- ------ ------           -------------
  256    128 Bc7RgbaUnormSrgb             9
```
{{< /tab >}}
{{< /tabs >}}

The sprite takes BC7 too, which keeps its alpha. The HDR picture takes BC6H, which keeps colors brighter than white, with no alpha, also at one byte a texel:

```powershell
PS> $cutout = Import-WgpuImage ./sprite.png -FixAlphaBorder -MipLevels 0 -Format Bc7RgbaUnormSrgb
PS> $sky = Import-WgpuImage ./scene.hdr -PowerOfTwo Nearest -MipLevels 0 -Format Bc6hRgbUfloat
PS> $cutout, $sky | Select-Object Width, Height, Format, MipLevelCount
Width Height Format           MipLevelCount
----- ------ ------           -------------
  128    128 Bc7RgbaUnormSrgb             8
  256    128 Bc6hRgbUfloat                9
```

## Check what compression changed

BC7 keeps each block of 4 by 4 texels close to the picture, not exact. `Compare-WgpuTexture` measures how close, against the same picture loaded without `-Format`: a row for each mip level and channel with the mean squared error; the PSNR in decibels, which grows as the copy comes closer; and SSIM, which is 1 where the copy keeps the picture's local structure. Level 0's rows:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $original = Import-WgpuImage ./scene.png -PowerOfTwo Nearest -MipLevels 0 -MipFilter Kaiser -Srgb
PS> $color | Compare-WgpuTexture $original | Where-Object Level -EQ 0
Level Layer Channel                  MSE             PSNR              SSIM
----- ----- -------                  ---             ----              ----
    0     0 Red     3.05668567978661E-06 55.1474921765355 0.999128520924049
    0     0 Green   2.34988871467705E-06 56.2895270442872 0.999052178355408
    0     0 Blue    3.10549503195886E-06 55.0786916118403  0.99862654642476
    0     0 Alpha                      0                ∞                 1
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $original = (Import-WgpuImage ./scene.png -PowerOfTwo Nearest -Srgb).GenerateMips(0, 'Kaiser', $null)
PS> $color.Compare($original) | Where-Object Level -EQ 0
Level Layer Channel                  MSE             PSNR              SSIM
----- ----- -------                  ---             ----              ----
    0     0 Red     3.05668567978661E-06 55.1474921765355 0.999128520924049
    0     0 Green   2.34988871467705E-06 56.2895270442872 0.999052178355408
    0     0 Blue    3.10549503195886E-06 55.0786916118403  0.99862654642476
    0     0 Alpha                      0                ∞                 1
```
{{< /tab >}}
{{< /tabs >}}

`-Difference` draws where the error is, in a picture of the same size: each channel's error on a fixed log scale, black at 2^-12 or less and full at 1 or more, each twelfth of brightness doubling it, so the same brightness means the same error in any picture:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $color | Compare-WgpuTexture $original -Difference | Export-WgpuImage -Path ./scene-bc7-error.png
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $color.Difference($original, $null).Save((Join-Path $PWD scene-bc7-error.png))
```
{{< /tab >}}
{{< /tabs >}}

![The error of the BC7 copy of the landscape](/images/scene-bc7-error.png)

About half the sky comes back a step or two of the 8-bit values off, which shows dim, in rows that follow its gradient, and the flat lower hills come back exact and stay black. The brightest texels, about six steps off, are along the far hill's ridge and the sun's rim, where a block of 4 by 4 texels holds two colors.

## Make a normal map

A normal map gives a flat surface the light and shade of small bumps: each texel holds the direction the surface faces there, its normal, which a shader lights in place of the shape's own. `ConvertTo-WgpuNormalMap` makes one from a height map, a gray picture that is lighter where the surface is higher, such as this brick wall:

![A height map of a brick wall](/images/height.png)

The bricks repeat across the wall, so `-HeightEdge Wrap` takes the slopes at each edge from the other side, and `-Strength 4` makes the step from the mortar up to a brick's face steep enough to catch the light:

```powershell
PS> $height = Import-WgpuImage ./height.png
PS> $height | ConvertTo-WgpuNormalMap -Strength 4 -HeightEdge Wrap | Export-WgpuImage -Path ./wall-normal.png
```

![The normal map of the brick wall](/images/wall-normal.png)

Red is how far each normal leans right, and green how far it leans up the image, as glTF reads a normal map, each stored with 128 for none. The flat faces are the blue of a normal pointing straight out, 128, 128, 255; a bevel that faces right has more red, and one that faces up more green. A renderer that reads green the other way, the convention named after DirectX, takes `-FlipGreen`. Without Wrap, the slope at each edge carries on past it, and the `Notes` say the wall looks like it tiles:

```powershell
PS> ($height | ConvertTo-WgpuNormalMap -Strength 4).Notes
the heights, read from red, were made into normals on the GPU by the Prewitt kernel at strength 4, the slope at each edge carried on past it; green points up, as glTF reads it, and alpha holds 1
the height map looks like it tiles, as its left and right edges, and its top and bottom, join as neighboring texels do: the Wrap edge reads across them
the texels were packed into Rgba8Unorm on the CPU
```

A normal map keeps X and Y in BC5, two channels in one byte a texel, and the shader rebuilds Z from them. Averaging normals for the mip levels shortens them, which would dim the wall in the distance, so each level's normals are scaled back to length 1. `Import-WgpuImage -HeightToNormal` makes the normals as the height map loads, before `-MipLevels` and `-Format`:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $wall = Import-WgpuImage ./height.png -HeightToNormal 4 -HeightEdge Wrap -MipLevels 0 -Format Bc5RgUnorm
PS> $wall | Select-Object Width, Height, Format, MipLevelCount
Width Height Format     MipLevelCount
----- ------ ------     -------------
  128    128 Bc5RgUnorm             8
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $wall = $height.ToNormalMap(4, $null, $null, 'Wrap', $null).GenerateMips(0, $null, $null, $null, 'Renormalize').Compress('Bc5RgUnorm', $null, $null)
PS> $wall | Select-Object Width, Height, Format, MipLevelCount
Width Height Format     MipLevelCount
----- ------ ------     -------------
  128    128 Bc5RgUnorm             8
```
{{< /tab >}}
{{< /tabs >}}

[Make normal maps](Images.md#make-normal-maps) has the kernels, the edges and what alpha can hold.

## Save it whole

A `.ktx2` or `.dds` file holds a texture as it is on the GPU, every level in its own format, so a renderer copies it straight in. `-Zstd` compresses a KTX2 file's levels further, for the download, and the renderer undoes it as it loads:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> Export-WgpuImage $color ./scene.ktx2 -Zstd
PS> Export-WgpuImage $cutout ./sprite.ktx2 -Zstd
PS> Export-WgpuImage $wall ./wall.ktx2 -Zstd
PS> Export-WgpuImage $sky ./sky.dds
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $color.Save((Join-Path $PWD scene.ktx2))
PS> $cutout.Save((Join-Path $PWD sprite.ktx2))
PS> $wall.Save((Join-Path $PWD wall.ktx2))
PS> $sky.Save((Join-Path $PWD sky.dds))
```

The `Save` method has no Zstandard setting; `Export-WgpuImage -Zstd` writes the smaller file.
{{< /tab >}}
{{< /tabs >}}

Loading one back gives the texture as it was saved:

```powershell
PS> Import-WgpuImage ./scene.ktx2 | Select-Object Width, Height, Format, MipLevelCount
Width Height Format           MipLevelCount
----- ------ ------           -------------
  256    128 Bc7RgbaUnormSrgb             9
```

## Reach devices without BC

Desktop GPUs decode the BC formats; most phone GPUs decode ASTC or ETC2 instead. `Get-WgpuTextureFormat` says what a format needs, here the TextureCompressionBc feature. A KTX2 file in a Basis Universal format loads on any device, transcoded into the best format that device holds, which is BC7 on the GPU these pages ran on:

```powershell
PS> Get-WgpuTextureFormat -Name Bc7RgbaUnorm | Select-Object Name, IsCompressed, RequiredFeatures
Name             IsCompressed RequiredFeatures
----             ------------ ----------------
Bc7RgbaUnorm             True {TextureCompressionBc}
Bc7RgbaUnormSrgb         True {TextureCompressionBc}

PS> Export-WgpuImage $mipped ./scene-any.ktx2 -Format BasisUastc4x4 -Quality 80
PS> Import-WgpuImage ./scene-any.ktx2 | Select-Object Format, MipLevelCount
Format       MipLevelCount
------       -------------
Bc7RgbaUnorm             9
```

A normal map in a Basis Universal format takes `-TwoChannel`, which keeps X in red, green and blue and Y in alpha, for the shader to rebuild Z from red and alpha:

```powershell
PS> Export-WgpuImage $height ./wall-any.ktx2 -HeightToNormal 4 -HeightEdge Wrap -MipLevels 0 -Format BasisUastc4x4 -TwoChannel
PS> Import-WgpuImage ./wall-any.ktx2 | Select-Object Format, MipLevelCount
Format       MipLevelCount
------       -------------
Bc7RgbaUnorm             8
```

[Compress a texture](Images.md#compress-a-texture) lists every format the module encodes, and what happens on a device that cannot hold one.
