---
title: How To Edit And Adjust Images
weight: 9
---

Crop, scale, frame, turn and mirror a picture, adjust its colors, soften, sharpen or outline it, cut a sprite out of the color behind it, and make an HDR picture fit a screen, all on the GPU. Each step makes a new texture and leaves the one it read as it was. The pictures are [scene.png](/images/scene.png), [badge.png](/images/badge.png), [sprite.png](/images/sprite.png) and [scene.hdr](/images/scene.hdr):

![A landscape: a blue sky fading to white at the horizon, a pale yellow sun at the top right, and two green hills](/images/scene.png)

```powershell
PS> $scene = Import-WgpuImage ./scene.png
```

## Crop, scale and frame it

One call of `Resize-WgpuTexture` crops, then scales, then pads, whatever order the parameters are in. This makes a square thumbnail of the sun: the right half of the sky, 128 by 96 texels, scaled to 80 wide, on a white 96 by 96 canvas. The methods take one step each:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $thumb = $scene | Resize-WgpuTexture -Crop 128, 0, 128, 96 -Width 80 -Pad 96, 96 -PadColor 1, 1, 1, 1
PS> $thumb | Select-Object Width, Height
Width Height
----- ------
   96     96

PS> Export-WgpuImage $thumb ./thumb.png
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $thumb = $scene.Crop(128, 0, 128, 96).Resize(80, 0, 'Box').Pad(96, 96, 'Center', @(1, 1, 1, 1))
PS> $thumb | Select-Object Width, Height
Width Height
----- ------
   96     96

PS> $thumb.Save((Join-Path $PWD thumb.png))
```
{{< /tab >}}
{{< /tabs >}}

![A square thumbnail: the sun in blue sky above the top of a hill, on a white margin](/images/thumb.png)

Shrinking averages the texels each new texel covers. [Change a texture's size and orientation](Images.md#change-a-textures-size-and-orientation) has the other filters, `-Scale`, `-MaxSize` and `-PowerOfTwo`.

## Turn and mirror it

`-Rotate` turns a texture clockwise by 90, 180 or 270 degrees, and `-Flip` mirrors it. A crop of one texel reads one texel back, here where the sun's center lands:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $turned = $scene | Convert-WgpuTexture -Rotate 90
PS> $turned | Select-Object Width, Height
Width Height
----- ------
  160    256

PS> $mirrored = $scene | Convert-WgpuTexture -Flip Horizontal
PS> (Read-WgpuTexture ($mirrored | Resize-WgpuTexture -Crop 67, 44, 1, 1)) -join ','
255,240,158,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $turned = $scene.Rotate(90)
PS> $turned | Select-Object Width, Height
Width Height
----- ------
  160    256

PS> $mirrored = $scene.Flip('Horizontal')
PS> $mirrored.Crop(67, 44, 1, 1).Read() -join ','
255,240,158,255
```
{{< /tab >}}
{{< /tabs >}}

The sun's center was 188 texels from the left edge; mirrored, it is 188 texels from the right one, at 67.

## Adjust its colors

`-Brightness` and `-Contrast` take -100 to 100 and `-Gamma` a power, as ImageMagick's `-brightness-contrast` and `-gamma` do. `-Grayscale` weighs red, green and blue by how bright each looks, and `-Invert` makes a negative. Each runs here on a copy of the scene 128 texels wide:

```powershell
PS> $small = $scene | Resize-WgpuTexture -Width 128
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $small | Convert-WgpuTexture -Brightness 10 -Contrast -20 -Gamma 1.2 | Export-WgpuImage -Path ./adjusted.png
PS> $small | Convert-WgpuTexture -Grayscale | Export-WgpuImage -Path ./gray.png
PS> $small | Convert-WgpuTexture -Invert | Export-WgpuImage -Path ./negative.png
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $small.Adjust(10, -20, 1.2, 0).Save((Join-Path $PWD adjusted.png))
PS> $small.Grayscale('Rec709').Save((Join-Path $PWD gray.png))
PS> $small.Invert().Save((Join-Path $PWD negative.png))
```
{{< /tab >}}
{{< /tabs >}}

![The landscape brighter and flatter](/images/adjusted.png) ![The landscape in gray](/images/gray.png) ![The landscape as a negative: an orange sky and purple hills](/images/negative.png)

[Change a texture's channels and colors](Images.md#change-a-textures-channels-and-colors) lists the order one call applies these in, and the other grayscale weights.

## Soften, sharpen or outline it

`-Blur` softens a picture by a Gaussian whose standard deviation it gives in texels, `-Sharpen` brings out its detail by adding back that many times the difference from a blur of it, and `-Edges Canny` draws its outlines in white on black. Each runs here on the same copy:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $small | Convert-WgpuTexture -Blur 2 | Export-WgpuImage -Path ./soft.png
PS> $small | Convert-WgpuTexture -Sharpen 1 | Export-WgpuImage -Path ./sharp.png
PS> $small | Convert-WgpuTexture -Edges Canny | Export-WgpuImage -Path ./outline.png
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $small.Blur(2, $null).Save((Join-Path $PWD soft.png))
PS> $small.Sharpen(1, $null, $null, $null).Save((Join-Path $PWD sharp.png))
PS> $small.FindEdges('Canny', $null, $null, $null).Save((Join-Path $PWD outline.png))
```
{{< /tab >}}
{{< /tabs >}}

![The landscape softened](/images/soft.png) ![The landscape sharpened](/images/sharp.png) ![The landscape's outlines in white on black: a circle where the sun is and a line along the hilltops](/images/outline.png)

[Blur, sharpen and find edges](Images.md#blur-sharpen-and-find-edges) has the other edge finders, the settings of each filter, and what they read past the edge of a picture.

## Cut a sprite out of its background

The badge is drawn on magenta, a color it does not use, so the magenta can be made transparent. `-ColorKey` gives alpha 0 to every texel of one color:

![A badge: a blue ring around a white disc, on magenta](/images/badge.png)

```powershell
PS> $badge = Import-WgpuImage ./badge.png
PS> $cut = $badge | Convert-WgpuTexture -ColorKey '#FF00FF'
PS> Export-WgpuImage $cut ./badge-cut.png
```

![The badge with no background: the page shows through around the ring](/images/badge-cut.png)

This function counts the texels a texture holds with alpha 0, the fourth number of each:

```powershell
PS> function Get-ClearTexelCount($Texture) {
>>     $texels = Read-WgpuTexture $Texture
>>     @(for ($i = 3; $i -lt $texels.Length; $i += 4) { if ($texels[$i] -eq 0) { $i } }).Count
>> }
PS> Get-ClearTexelCount $cut
1968
```

A JPEG of the badge shifts the magenta a little in many texels, so the exact key misses most of them, and `-ColorKeyTolerance` lets each channel differ from the key by up to that much:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> Export-WgpuImage $badge ./badge.jpg
PS> $blurred = Import-WgpuImage ./badge.jpg
PS> Get-ClearTexelCount ($blurred | Convert-WgpuTexture -ColorKey '#FF00FF')
124

PS> Get-ClearTexelCount ($blurred | Convert-WgpuTexture -ColorKey '#FF00FF' -ColorKeyTolerance 0.1)
1884
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $badge.Save((Join-Path $PWD badge.jpg))
PS> $blurred = Import-WgpuImage ./badge.jpg
PS> Get-ClearTexelCount $blurred.ColorKey('#FF00FF', 0)
124

PS> Get-ClearTexelCount $blurred.ColorKey('#FF00FF', 0.1)
1884
```
{{< /tab >}}
{{< /tabs >}}

The 84 texels the tolerance still leaves are scattered around the ring, where the JPEG mixed the magenta with the ring's blue.

A sprite whose edge fades into alpha, drawn where only an alpha test is available, needs that edge made hard. `-AlphaThreshold` gives alpha 0 below the threshold and full alpha from it up:

![A sprite: an orange disc with a darker ring, whose edge fades into transparency](/images/sprite.png)

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $sprite = Import-WgpuImage ./sprite.png
PS> Get-ClearTexelCount $sprite
7892

PS> Get-ClearTexelCount ($sprite | Convert-WgpuTexture -AlphaThreshold 0.5)
9152
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $sprite = Import-WgpuImage ./sprite.png
PS> Get-ClearTexelCount $sprite
7892

PS> Get-ClearTexelCount $sprite.AlphaThreshold(0.5)
9152
```
{{< /tab >}}
{{< /tabs >}}

The outer half of the fading edge became clear, and the inner half fully opaque. [Make cutouts with alpha](Images.md#make-cutouts-with-alpha) has `-FixAlphaBorder`, which keeps a cutout's edge from darkening when it is filtered.

## Make an HDR picture fit a screen

[scene.hdr](/images/scene.hdr) is the landscape in Radiance HDR, whose texels hold light far brighter than white: here its sun is 24 times brighter. It loads as 32-bit floats:

```powershell
PS> $hdr = Import-WgpuImage ./scene.hdr
PS> $hdr | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
  256    160 Rgba32Float

PS> (Measure-WgpuArray (Read-WgpuTexture $hdr)).Maximum
24
```

A screen shows 0 to 1. `-ToneMap` maps the whole range into it with a curve, AgX here, keeping the sun brighter than the sky without cutting it off, and `-Exposure` scales the light by stops first, -2 for a quarter of it. The curve's result is linear light, which `-ColorSpace Srgb` encodes as an 8-bit image file expects:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $hdr | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb | Export-WgpuImage -Path ./scene-agx.png
PS> $hdr | Convert-WgpuTexture -Exposure -2 -ToneMap AgX -ColorSpace Srgb | Export-WgpuImage -Path ./scene-darker.png
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $hdr.ToneMap('AgX').ConvertColorSpace('Srgb').Save((Join-Path $PWD scene-agx.png))
PS> $hdr.Adjust(0, 0, 1, -2).ToneMap('AgX').ConvertColorSpace('Srgb').Save((Join-Path $PWD scene-darker.png))
```
{{< /tab >}}
{{< /tabs >}}

![The HDR landscape tone mapped with AgX](/images/scene-agx.png) ![The same, two stops darker](/images/scene-darker.png)

Reinhard, AcesFilmic and Neutral are the other curves.

## Keep its colors right in shaders

An 8-bit picture's colors are sRGB-encoded unless its file names another color space; browsers take an untagged image to be sRGB. The encoding makes equal steps in the numbers look about equally far apart in brightness. A shader that blends or lights colors needs them in linear light, and reads an sRGB-format texture's texels decoded into linear light. `-Srgb` loads a picture in the sRGB form of its format, and `-AssignColorSpace Srgb` relabels a texture that way, both keeping the stored numbers. `-ColorSpace Srgb` converts instead: it changes the numbers so the colors look the same as they did. The sun's center shows the difference:

```powershell
PS> $sun = $scene | Resize-WgpuTexture -Crop 188, 44, 1, 1
PS> (Read-WgpuTexture $sun) -join ','
255,240,158,255

PS> $relabeled = $sun | Convert-WgpuTexture -AssignColorSpace Srgb
PS> "$($relabeled.Format): $((Read-WgpuTexture $relabeled) -join ',')"
Rgba8UnormSrgb: 255,240,158,255

PS> $converted = $sun | Convert-WgpuTexture -ColorSpace Srgb
PS> "$($converted.Format): $((Read-WgpuTexture $converted) -join ',')"
Rgba8UnormSrgb: 255,248,206,255
```

An 8-bit picture whose file names no other color space is sRGB-encoded already: relabel it, or load it with `-Srgb`. Convert only colors worked out in linear light, as a shader's output.
