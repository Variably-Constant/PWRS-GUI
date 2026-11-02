---
title: Textures and images
weight: 50
---

A texture is a grid of texels in one format, on the GPU: one, two or three dimensions, with layers and mip levels, each mip level half the size of the one before. Its texels go in and come out as one array, rows packed, of the number type the format holds: bytes for the 8-bit formats, floats for the float ones. Image files load into textures and save from them, DDS and KTX2 files hold a texture whole, a texture can be given mip levels and compressed, panoramas and other images become cube maps and the maps image-based lighting reads, a compute shader reads and writes textures, and a sampler reads a texture between its texels. The parameters are on [Textures and images](Texture-Cmdlets.md) under Cmdlets and types, and the `Wgpu.Texture` class on [Types](Types.md#wgputexture).

## Make a texture

A 2D texture of the default format, Rgba8Unorm, gets every usage the format allows on the adapter when `-Usage` names none. `-Data` gives its texels, converted to the format's number type:

```powershell
PS> $texture = New-WgpuTexture 4 4
PS> $texture
Label          Width Height Format
-----          ----- ------ ------
4x4 Rgba8Unorm     4      4 Rgba8Unorm

PS> $texture | Select-Object Dimension, ElementType, Usage
Dimension ElementType Usage
--------- ----------- -----
       D2       UInt8 {CopySrc, CopyDst, TextureBinding, StorageBinding…}

PS> $pixels = New-WgpuTexture 2 2 -Data ([byte[]](255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255))
PS> $heights = New-WgpuTexture 3 2 -Format R32Float -Data ([float[]](1.5, 2, 3, 4, 5, 6.25))
```

A 3D texture, an array of layers with mip levels, and a texture limited to the usages a compute shader that writes it needs:

```powershell
PS> New-WgpuTexture 16 16 -Depth 8 -Dimension D3 -Format Rgba16Float | Select-Object Dimension, DepthOrArrayLayers, Format
Dimension DepthOrArrayLayers Format
--------- ------------------ ------
       D3                  8 Rgba16Float

PS> New-WgpuTexture 32 32 -Layers 6 -MipLevels 6 | Select-Object Dimension, DepthOrArrayLayers, MipLevelCount
Dimension DepthOrArrayLayers MipLevelCount
--------- ------------------ -------------
       D2                  6             6

PS> (New-WgpuTexture 8 8 -Usage StorageBinding, CopySrc).Usage -join ', '
CopySrc, StorageBinding
```

A format wgpu does not have is refused, as are texels that do not fill the level:

```powershell
PS> New-WgpuTexture 4 4 -Format Rgba9Unorm
New-WgpuTexture: wgpu has no texture format named Rgba9Unorm; Get-WgpuTextureFormat lists them

PS> New-WgpuTexture 4 4 -Data ([byte[]](1, 2, 3))
New-WgpuTexture: mip 0 of a Rgba8Unorm texture holds 64 bytes; 3 were given
```

## Choose a format

`Get-WgpuTextureFormat` lists every format wgpu has, and what the adapter of the default device, or of `-Device`, can do with each. Given some text, it keeps the formats whose name contains it, so `Where-Object` picks the one named exactly:

```powershell
PS> (Get-WgpuTextureFormat).Count
118

PS> Get-WgpuTextureFormat rgba8
Name               BlockSize Components AllowedUsages
----               --------- ---------- -------------
Rgba8Unorm                 4          4 {CopySrc, CopyDst, TextureBinding, StorageBinding…}
Rgba8UnormSrgb             4          4 {CopySrc, CopyDst, TextureBinding, RenderAttachment…}
Rgba8Snorm                 4          4 {CopySrc, CopyDst, TextureBinding, StorageBinding…}
Rgba8Uint                  4          4 {CopySrc, CopyDst, TextureBinding, StorageBinding…}
Rgba8Sint                  4          4 {CopySrc, CopyDst, TextureBinding, StorageBinding…}
Etc2Rgba8Unorm            16          4 {}
Etc2Rgba8UnormSrgb        16          4 {}

PS> Get-WgpuTextureFormat Rgba16Float | Where-Object Name -EQ Rgba16Float | Format-List
Name             : Rgba16Float
BlockSize        : 8
BlockWidth       : 1
BlockHeight      : 1
Components       : 4
IsCompressed     : False
IsDepthStencil   : False
IsSrgb           : False
ElementType      : Float16
RequiredFeatures : {}
AllowedUsages    : {CopySrc, CopyDst, TextureBinding, StorageBinding…}
Capabilities     : {Filterable, MultisampleX2, MultisampleX4, MultisampleX8…}
```

The usages a format allows decide what a texture of it can be: one a compute shader writes needs StorageBinding, which no sRGB format and no depth or stencil format allows:

```powershell
PS> (Get-WgpuTextureFormat | Where-Object AllowedUsages -Contains StorageBinding).Count
41

PS> (Get-WgpuTextureFormat Rgba8UnormSrgb | Where-Object Name -EQ Rgba8UnormSrgb).AllowedUsages -join ', '
CopySrc, CopyDst, TextureBinding, RenderAttachment, TransientAttachment
```

A compressed format is read in blocks and needs a feature; the software adapter has no ASTC:

```powershell
PS> Get-WgpuTextureFormat Astc4x4Unorm | Where-Object Name -EQ Astc4x4Unorm | Select-Object Name, BlockWidth, BlockHeight, BlockSize, RequiredFeatures
Name             : Astc4x4Unorm
BlockWidth       : 4
BlockHeight      : 4
BlockSize        : 16
RequiredFeatures : {TextureCompressionAstc}

PS> (Get-WgpuTextureFormat Astc4x4Unorm -Device (Open-WgpuDevice -Fallback) | Where-Object Name -EQ Astc4x4Unorm).AllowedUsages.Count
0
```

## Read and write texels

A read gives a mip level of a layer, both 0 when not named, as one array of the format's number type, and a write takes texels that fill the level exactly:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture $pixels) -join ','
255,0,0,255,0,255,0,255,0,0,255,255,255,255,255,255

PS> Read-WgpuTexture $heights
1.5
2
3
4
5
6.25

PS> Write-WgpuTexture $heights ([int[]](1, 2, 3, 4, 5, 6))
PS> (Read-WgpuTexture $heights) -join ','
1,2,3,4,5,6

PS> Write-WgpuTexture $heights ([float[]](1, 2))
Write-WgpuTexture: mip 0 of a R32Float texture holds 24 bytes; 8 were given
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $pixels.Read() -join ','
255,0,0,255,0,255,0,255,0,0,255,255,255,255,255,255

PS> $heights.Read()
1.5
2
3
4
5
6.25

PS> $heights.Write([int[]](1, 2, 3, 4, 5, 6))
PS> $heights.Read() -join ','
1,2,3,4,5,6

PS> $heights.Write([float[]](1, 2))
MethodInvocationException: Exception calling "Write" with "1" argument(s): "[WgpuTexelCount] mip 0 of a R32Float texture holds 24 bytes; 8 were given"
```
{{< /tab >}}
{{< /tabs >}}

`-AsBytes` reads the raw bytes of any format:

```powershell
PS> $pair = New-WgpuTexture 2 1 -Format R32Float -Data ([float[]](1, 2))
PS> (Read-WgpuTexture $pair -AsBytes) -join ','
0,0,128,63,0,0,0,64
```

One mip level, and one layer of an array:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $mipped = New-WgpuTexture 4 4 -MipLevels 3 -Format R32Float
PS> Write-WgpuTexture $mipped ([float[]](9)) -MipLevel 2
PS> Read-WgpuTexture $mipped -MipLevel 2
9

PS> $layers = New-WgpuTexture 2 1 -Layers 3
PS> Write-WgpuTexture $layers ([byte[]](1..8)) -Layer 2
PS> (Read-WgpuTexture $layers -Layer 2) -join ','
1,2,3,4,5,6,7,8
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $mipped = New-WgpuTexture 4 4 -MipLevels 3 -Format R32Float
PS> $mipped.Write([float[]](9), 2)
PS> $mipped.Read(2)
9

PS> $layers = New-WgpuTexture 2 1 -Layers 3
PS> $layers.Write([byte[]](1..8), 0, 2)
PS> $layers.Read(0, 2) -join ','
1,2,3,4,5,6,7,8
```
{{< /tab >}}
{{< /tabs >}}

## Load and save image files

`Import-WgpuImage` loads PNG, JPEG, GIF, WebP, AVIF, JPEG XL, TIFF, BMP, TGA, OpenEXR, Radiance HDR, QOI, ICO, PNM and Farbfeld files, and DDS and KTX2 files whole, as [the next section](#save-and-load-a-texture-whole) shows, and draws SVG files and PDF pages, as [Draw an SVG file or a PDF page](#draw-an-svg-file-or-a-pdf-page) shows. An 8-bit image loads as Rgba8Unorm, or as Rgba8UnormSrgb with `-Srgb`, a 16-bit one, or an AVIF file of 10 or 12 bits, as Rgba16Unorm and a float one as Rgba32Float, unless `-Format` names a format to convert it into; gray is Rec. 709 luma, and a two-channel format takes red and green, or a gray image's gray and alpha. Saving writes the format the file's extension names, converted to what that format stores, with a two-channel texture as red and green, blue 0 and alpha 1. A .NET method resolves a relative path against the process's working directory, which PowerShell's location does not change, so the `Save` method takes the file's full path:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $pixels | Export-WgpuImage -Path ./tiny.png
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

PS> (Import-WgpuImage ./tiny.png -Srgb).Format
Rgba8UnormSrgb

PS> (Read-WgpuTexture (Import-WgpuImage ./tiny.png -Format R8Unorm)) -join ','
54,182,18,255

PS> Get-ChildItem ./*.png | Import-WgpuImage | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
    2      2 Rgba8Unorm
```

`-Path` reads wildcards and loads every file they match. `-LiteralPath` takes a name as it is written, brackets included, and the files `Get-ChildItem` pipes bind to it:

```powershell
PS> $pixels | Export-WgpuImage -Path './tiny[1].png'
PS> (Import-WgpuImage -LiteralPath './tiny[1].png').Label
C:\Temp\wgpu-demo\tiny[1].png

PS> Import-WgpuImage -Path './tiny[1].png'
Import-WgpuImage: ./tiny[1].png names no file

PS> Import-WgpuImage -Path ./tiny*.png | Select-Object Label, Width
Label                         Width
-----                         -----
C:\Temp\wgpu-demo\tiny.png        2
C:\Temp\wgpu-demo\tiny[1].png     2
```

`-ImageFormat` names the format whatever the extension. OpenEXR keeps floats, compressed with ZIP, as half floats for a texture of 16-bit float texels and as full floats for any other; PNG keeps 16 bits a channel for a texture of 16-bit or float texels and 8 for any other; JPEG has no alpha; and AVIF keeps alpha, at 8 bits a channel, and compresses color with some loss:

```powershell
PS> Export-WgpuImage $pixels ./tiny.bmp
PS> Export-WgpuImage $pixels ./tiny.dat -ImageFormat Tga
PS> Get-ChildItem ./tiny.* | Select-Object Name, Length
Name     Length
----     ------
tiny.bmp    138
tiny.dat     36
tiny.png     86

PS> $light = New-WgpuTexture 1 1 -Format Rgba32Float -Data ([float[]](2.5, 0.5, 0.25, 1))
PS> Export-WgpuImage $light ./light.exr
PS> (Read-WgpuTexture (Import-WgpuImage ./light.exr)) -join ','
2.5,0.5,0.25,1

PS> Export-WgpuImage (New-WgpuTexture 1 1 -Data ([byte[]](255, 0, 0, 128))) ./faded.jpg
PS> (Read-WgpuTexture (Import-WgpuImage ./faded.jpg))[3]
255

PS> Export-WgpuImage (New-WgpuTexture 1 1 -Data ([byte[]](255, 0, 0, 128))) ./faded.avif
PS> (Read-WgpuTexture (Import-WgpuImage ./faded.avif)) -join ','
253,1,1,128
```

A file that is not an image, an extension no image format has, and a texture format no image holds are refused:

```powershell
PS> Set-Content ./notes.png 'not an image'
PS> Import-WgpuImage ./notes.png
Import-WgpuImage: C:\Temp\wgpu-demo\notes.png cannot be read as an image: Format error decoding Png: Invalid PNG signature.

PS> Export-WgpuImage $pixels ./tiny.xyz
Export-WgpuImage: C:\Temp\wgpu-demo\tiny.xyz names no image format the module writes: The file extension `."xyz"` was not recognized as an image format

PS> Export-WgpuImage (New-WgpuTexture 2 2 -Format R32Uint) ./counts.png
Export-WgpuImage: a R32Uint texture has no image form; images come from the unsigned norm and float formats, and a .dds or .ktx2 file holds the texture whole
```

## Follow a file's orientation and color profile

`Import-WgpuImage` turns an image as its file's orientation says and converts its colors to sRGB from the color profile the file carries, and says what it did on the verbose stream and in the texture's `Notes`. The orientation is the EXIF one of a JPEG, PNG, WebP or TIFF file, a PNG's in the text chunk ImageMagick writes it in included, an AVIF file's rotation and mirror, or a JPEG XL file's own orientation. [turned.jpg](/images/turned.jpg) is the landscape at 128 by 80 pixels, recorded as turned a quarter clockwise:

```powershell
PS> $turned = Import-WgpuImage ./turned.jpg -Verbose
VERBOSE: the pixels were turned 90 degrees clockwise, as the file's orientation says

PS> '{0}x{1}' -f $turned.Width, $turned.Height
80x128
```

The color profile is an ICC profile of RGB, gray or CMYK color, a CMYK JPEG or TIFF file's converting from its inks as stored, or an AVIF file's CICP values. The colors convert as littleCMS converts them with its optimizations off, perceptually unless `-RenderingIntent` names another intent: by the profile's own table for the intent, read for each pixel, a floating point D2B table among them, or by its matrix and curves. Colors sRGB lacks are clipped. [landscape-p3.jpg](/images/landscape-p3.jpg) holds the values of the landscape in [scene.png](/images/scene.png) under a Display P3 profile, whose wider gamut gives the same values more vivid colors:

```powershell
PS> $vivid = Import-WgpuImage ./landscape-p3.jpg -Verbose
VERBOSE: the colors were converted from the file's color profile, Display P3, to sRGB, perceptually

PS> (Read-WgpuTexture (Import-WgpuImage ./scene.png -Width 128))[0..2] -join ','
48,108,209

PS> (Read-WgpuTexture $vivid)[0..2] -join ','
15,111,218
```

An AVIF or JPEG XL file of PQ or HLG light, HDR, loads as linear light in sRGB's primaries, with BT.2408's HDR reference white of 203 nits at 1 and brighter light above it, in Rgba16Float unless `-Format` names another format. A file whose orientation or color profile the module cannot read or use loads as stored, and a warning says why.

`-IgnoreOrientation` loads the pixels as the file stores them, and `-IgnoreProfile` the colors. `-RenderingIntent` converts with another of the profile's intents than `Perceptual`: `RelativeColorimetric`, `Saturation` or `AbsoluteColorimetric`. An option the file leaves nothing to change, such as `-IgnoreProfile` for a file with no profile, loads the file and a warning says so, and `-RenderingIntent` beside `-IgnoreProfile` is refused.

## Draw an SVG file or a PDF page

`Import-WgpuImage` draws an SVG file through resvg, and the first page of a PDF through hayro, in sRGB. An SVG is drawn at the size it gives, in pixels of a 96th of an inch. `-Width`, `-Height`, `-Scale`, `-MaxSize` and `-PowerOfTwo` set the size it is drawn at, so a small drawing loads large and sharp, never resampled, and `-Crop`, `-CropMargin`, `-Rotate` and `-Flip` are drawn too. Its text is drawn in the machine's fonts; text in a font-family the machine has no font of is drawn in its default family, and a warning names the family:

```powershell
PS> Get-Content ./badge.svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32" viewBox="0 0 64 32">
  <rect width="64" height="32" rx="8" fill="#3366cc"/>
  <circle cx="16" cy="16" r="10" fill="#ffffff"/>
</svg>

PS> $badge = Import-WgpuImage ./badge.svg -Width 256 -Verbose
VERBOSE: the SVG file, 64x32 at its own size, was drawn at 256x128

PS> '{0}x{1}' -f $badge.Width, $badge.Height
256x128
```

A PDF's page is drawn at 72 dots per inch, a pixel a point, or at the `-Dpi` given, on white unless `-NoBackground` leaves it transparent where nothing is drawn. The size options draw a page as they draw an SVG. [page.pdf](/images/page.pdf) is a page 40 by 30 points:

```powershell
PS> $page = Import-WgpuImage ./page.pdf -Dpi 144 -Verbose
VERBOSE: page 1 of the PDF's 1 was drawn at 144 dots per inch, 80x60 at that resolution, at 80x60

PS> '{0}x{1}' -f $page.Width, $page.Height
80x60
```

`-Password` opens a PDF protected by a password, taken as a SecureString, as `Read-Host -AsSecureString` gives one; a protected PDF without it is refused, and so is a wrong password. A page draws the standard fonts PDF readers supply, from fonts hayro carries, and leaves out what hayro does not draw, such as text in a CID font whose encoding is not the identity, with a warning that says what.

## Save and load a texture whole

A `.dds` or `.ktx2` file holds a texture whole: every mip level, layer and cube face, in the texture's own format, compressed formats included. `Import-WgpuImage` loads one, and `Export-WgpuImage` and the `Save` method write one, chosen by the extension. A texture made with `-Cube` is a cube map, square layers six to a cube, which its `IsCube` says and its file records:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $sky = New-WgpuTexture 4 4 -Layers 6 -MipLevels 3 -Cube
PS> $sky | Export-WgpuImage -Path ./sky.dds
PS> Import-WgpuImage ./sky.dds | Select-Object Width, DepthOrArrayLayers, MipLevelCount, IsCube
Width DepthOrArrayLayers MipLevelCount IsCube
----- ------------------ ------------- ------
    4                  6             3   True
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $sky = New-WgpuTexture 4 4 -Layers 6 -MipLevels 3 -Cube
PS> $sky.Save((Join-Path $PWD sky.dds))
PS> Import-WgpuImage ./sky.dds | Select-Object Width, DepthOrArrayLayers, MipLevelCount, IsCube
Width DepthOrArrayLayers MipLevelCount IsCube
----- ------------------ ------------- ------
    4                  6             3   True
```
{{< /tab >}}
{{< /tabs >}}

`-Zstd` compresses each level of a KTX2 file with Zstandard, and `-Cube` writes any texture whose layers are square and six to a cube as a cube map. A texture whose layers are not is refused:

```powershell
PS> $sky | Export-WgpuImage -Path ./sky.ktx2 -Zstd
PS> Get-ChildItem ./sky.* | Select-Object Name, Length
Name     Length
----     ------
sky.dds     652
sky.ktx2    322

PS> New-WgpuTexture 4 4 -Layers 6 | Export-WgpuImage -Path ./faces.ktx2 -Cube
PS> (Import-WgpuImage ./faces.ktx2).IsCube
True

PS> New-WgpuTexture 4 4 -Layers 4 | Export-WgpuImage -Path ./four.ktx2 -Cube
Export-WgpuImage: cube faces are square 2D layers, six to a cube; a 4x4 texture with 4 layers is not that
```

## Give a texture mip levels

`-MipLevels` gives a texture that many mip levels, or with 0 every level down to 1x1, as `Import-WgpuImage` loads it or `Export-WgpuImage` saves it. Each texel of a level is the average of the texels under it in the level above, worked out on the GPU in floating point, in linear light for an sRGB format, and rounded to the format once. The `GenerateMips` method does the same for a texture already made: with no argument it fills the levels the texture has, and with a number it returns a new texture with that many. This image has black columns on the left and white ones on the right, so every level below it is gray:

```powershell
PS> $columns = New-WgpuTexture 4 4 -Data ([byte[]](0..15 | ForEach-Object { if ($_ % 4 -lt 2) { 0, 0, 0, 255 } else { 255, 255, 255, 255 } }))
PS> $columns | Export-WgpuImage -Path ./columns.png
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $mipped = Import-WgpuImage ./columns.png -MipLevels 0
PS> $mipped.MipLevelCount
3

PS> (Read-WgpuTexture $mipped -MipLevel 2) -join ','
128,128,128,255

PS> (Read-WgpuTexture (Import-WgpuImage ./columns.png -Srgb -MipLevels 0) -MipLevel 2) -join ','
188,188,188,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $mipped = (Import-WgpuImage ./columns.png).GenerateMips(0)
PS> $mipped.MipLevelCount
3

PS> $mipped.Read(2) -join ','
128,128,128,255

PS> $levels = New-WgpuTexture 4 4 -MipLevels 3 -Data (Read-WgpuTexture $columns)
PS> $levels.GenerateMips()
PS> $levels.Read(2) -join ','
128,128,128,255
```
{{< /tab >}}
{{< /tabs >}}

A texture's `Notes` say what making it involved, here how its levels were made:

```powershell
PS> $mipped.Notes
mip levels 1 to 2 were rendered on the GPU in Rgba16Float, each from the level above it
mip levels 1 to 2 were packed into Rgba8Unorm on the CPU
```

`-MipFilter` chooses how each level is made. Box, the default, averages the texels under it in the level above. Triangle, Kaiser, Mitchell and Lanczos make each level straight from level 0, weighing the texels near each new texel by their distance from it. Min and Max keep the smallest or the largest value of each channel among the texels under it, as a depth pyramid needs. `-MipEdge` says what Triangle, Kaiser, Mitchell and Lanczos read past an edge of the image: nothing with Clamp, the default; the other side with Wrap, for a texture that tiles; or the image mirrored with Mirror. A sampler's ClampToEdge, Repeat and MirrorRepeat name the same three. `GenerateMips` takes the filter and the edge after the number of levels. The first texel of level 1 here takes in part of the white column to its right, and with Wrap also the white column at the far edge:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture (Import-WgpuImage ./columns.png -MipLevels 0 -MipFilter Triangle) -MipLevel 1)[0]
36

PS> $tiled = Import-WgpuImage ./columns.png -MipLevels 0 -MipFilter Triangle -MipEdge Wrap
PS> (Read-WgpuTexture $tiled -MipLevel 1)[0]
64

PS> $tiled.Notes
mip levels 1 to 2 were rendered on the GPU in Rgba16Float, each from level 0 by the Triangle filter, its edges wrapped
mip levels 1 to 2 were packed into Rgba8Unorm on the CPU
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $columns.GenerateMips(0, 'Triangle', $null).Read(1)[0]
36

PS> $columns.GenerateMips(0, 'Triangle', 'Repeat').Read(1)[0]
64

PS> $columns.GenerateMips(0, 'Triangle', [Wgpu.AddressMode]::MirrorRepeat).Read(1)[0]
32
```
{{< /tab >}}
{{< /tabs >}}

`-KeepCoverage` keeps a cutout as solid at every level as at level 0. Averaging thins strands of leaves or hair, so fewer texels of each smaller level pass an alpha test, and the cutout fades away in the distance. Given the alpha an alpha test passes at, each new level's alpha is scaled so the share of its texels at or above that alpha is level 0's, or as near it as a scale can bring it, as texconv's `--keep-coverage` does. The GPU counts each level's alpha in steps of 1/4096. Each layer of an array or cube keeps its own level 0's share, and a 3D texture's level counts all its depth slices together. `GenerateMips` takes it after the edge, and `Convert-WgpuTexture` and `Resize-WgpuTexture` take `-MipFilter`, `-MipEdge` and `-KeepCoverage` for the levels they render again. Here scattered texels cover about three tenths of a texture, and the share of each level's texels with alpha at or above a half falls away without it:

```powershell
PS> $leaves = New-WgpuTexture 32 32 -Data ([byte[]](0..1023 | ForEach-Object { 40, 120, 30, $(if ((($_ * 2654435761) % 4294967296) % 10 -lt 3) { 255 } else { 0 }) }))
PS> $leaves | Export-WgpuImage -Path ./leaves.png
PS> function Get-Share($texture, $level) { $a = Read-WgpuTexture $texture -MipLevel $level; $n = 0; for ($i = 3; $i -lt $a.Count; $i += 4) { if ($a[$i] -ge 128) { $n++ } }; $n / ($a.Count / 4) }
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $faded = Import-WgpuImage ./leaves.png -MipLevels 0
PS> (0..4 | ForEach-Object { Get-Share $faded $_ }) -join ', '
0.2919921875, 0.30859375, 0.03125, 0, 0

PS> $kept = Import-WgpuImage ./leaves.png -MipLevels 0 -KeepCoverage 0.5
PS> (0..4 | ForEach-Object { Get-Share $kept $_ }) -join ', '
0.2919921875, 0.30859375, 0.328125, 0.25, 0.25

PS> $kept.Notes
mip levels 1 to 5 were rendered on the GPU in Rgba16Float, each from the level above it, its alpha scaled to keep level 0's share of texels with alpha at or above 0.5
mip levels 1 to 5 were packed into Rgba8Unorm on the CPU
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $faded = $leaves.GenerateMips(0)
PS> (0..4 | ForEach-Object { Get-Share $faded $_ }) -join ', '
0.2919921875, 0.30859375, 0.03125, 0, 0

PS> $kept = $leaves.GenerateMips(0, $null, $null, 0.5)
PS> (0..4 | ForEach-Object { Get-Share $kept $_ }) -join ', '
0.2919921875, 0.30859375, 0.328125, 0.25, 0.25
```
{{< /tab >}}
{{< /tabs >}}

A normal map's levels keep their normals unit length with `-NormalMips`, which `GenerateMips` takes after the coverage; [Make normal maps](#make-normal-maps) shows it.

A filter, an edge, a coverage or `-NormalMips` that would change nothing for any file is refused: one given without `-MipLevels` or a change to the texture that renders its levels again, or with `-MipLevels 1`, except `-NormalMips` alone beside a `-Format`, which marks the texels as normals; Wrap or Mirror with Box, Min or Max, which never read past an edge; and `-KeepCoverage` on a format with no alpha channel. When the file decides, because it already has every level asked for, Basis Universal makes its levels, or it stores normals as the `-Format` does, a warning says so:

```powershell
PS> Import-WgpuImage ./columns.png -MipFilter Kaiser
Import-WgpuImage: -MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the mip levels -MipLevels adds and a transform renders again are made, and the call gives neither: give -MipLevels

PS> Import-WgpuImage ./columns.png -MipLevels 0 -MipEdge Wrap
Import-WgpuImage: Box, the filter when none is given, reads only the texels each new texel covers and never past an edge, so the Wrap edge changes nothing; Triangle, Kaiser, Mitchell and Lanczos read past one

PS> New-WgpuTexture 4 4 -MipLevels 3 | Export-WgpuImage -Path ./levels.dds
PS> $null = Import-WgpuImage ./levels.dds -MipLevels 0 -MipFilter Kaiser
WARNING: no mip level was rendered, as the file has every level the call asks for, so the mip filter changed nothing
```

`GenerateMips()` refuses a texture with a single level, which `GenerateMips(0)` copies instead, and a box filter cannot average integer texels:

```powershell
PS> $columns.GenerateMips()
MethodInvocationException: Exception calling "GenerateMips" with "0" argument(s): "[WgpuMipLevels] GenerateMips() fills a texture's own mip levels, and this one has a single level: GenerateMips(0) returns a copy with the full chain"

PS> New-WgpuTexture 4 4 -Format R8Uint | Export-WgpuImage -Path ./ids.dds -MipLevels 0
Export-WgpuImage: mip levels and format conversions read texels as floating-point colors, and R8Uint holds integers, depth or stencil, which a box filter cannot average
```

## Compress a texture

`-Format` names the format a texture takes as it loads or saves. A BC format is encoded on the GPU: BC1, BC2, BC3 and BC7 and their sRGB forms, BC4 and BC5, signed or not, and BC6H for colors of zero and above. An image file's 8-bit color is taken as already in the format's encoding, so an sRGB format keeps a photo's colors and averages its levels in linear light, and any other format keeps the values as they are, as a normal map needs. `-Effort`, from 0 to 10, trades time for quality in the BC7 and BC6H encoders. The `Compress` method returns a compressed copy of a texture. The first encode of a BC format in a session builds that format's encoder on the GPU, and the encodes after it reuse it. On the GPU these pages ran on, the first BC7 encode took 1 to 3.4 seconds through Direct3D 12 and at most 0.13 through Vulkan, and each one after it a hundredth of a second:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $bc7 = Import-WgpuImage ./columns.png -Format Bc7RgbaUnormSrgb -MipLevels 0
PS> $bc7 | Select-Object Format, MipLevelCount
Format           MipLevelCount
------           -------------
Bc7RgbaUnormSrgb             3

PS> $bc7.Notes
mip levels 1 to 2 were rendered on the GPU in Rgba16Float, each from the level above it
mip levels 0 to 2 were encoded into Bc7RgbaUnormSrgb on the GPU by block_compression's BC7 opaque basic settings
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $bc7 = (Import-WgpuImage ./columns.png -Srgb -MipLevels 0).Compress('Bc7RgbaUnormSrgb', $null, $null)
PS> $bc7 | Select-Object Format, MipLevelCount
Format           MipLevelCount
------           -------------
Bc7RgbaUnormSrgb             3

PS> $bc7.Notes
mip levels 0 to 2 were encoded into Bc7RgbaUnormSrgb on the GPU by block_compression's BC7 opaque basic settings
the new texture leaves out the source's RenderAttachment usage, which Bc7RgbaUnormSrgb does not allow
```
{{< /tab >}}
{{< /tabs >}}

A KTX2 file also takes the Basis Universal formats, which Basis Universal encodes into a file any device can load: BasisEtc1s, the smallest, BasisUastc4x4, the ASTC formats from BasisAstc4x4 to BasisAstc12x12, the HDR ones and the rest `-Format` completes. Such a file loads transcoded into the best format the device holds. `-Quality`, from 1 to 100, sets a Basis Universal format's quality, and `-Effort` the time it takes:

```powershell
PS> $columns | Export-WgpuImage -Path ./columns.ktx2 -Format BasisUastc4x4 -Quality 80
PS> (Import-WgpuImage ./columns.ktx2).Format
Bc7RgbaUnorm
```

A file in a compressed format the device cannot hold loads decoded on the CPU into an uncompressed format, which a warning says. This device was opened with a single feature, so it has no BC:

```powershell
PS> $columns | Export-WgpuImage -Path ./columns.dds -Format Bc1RgbaUnorm
PS> $plain = Open-WgpuDevice -Feature Float32Filterable
PS> (Import-WgpuImage ./columns.dds -Device $plain).Format
WARNING: the device has no TextureCompressionBc, so the Bc1RgbaUnorm texels were decoded on the CPU into Rgba8Unorm
Rgba8Unorm
```

A compressed texture saves as an image file decoded: on the CPU by the module's decoders, which give the same pixels on every device, or with `-Decoder Gpu` on the GPU, as a shader reads it. The `Save` and `SaveAsync` methods take the decoder after the path. The verbose stream says where the texels were decoded:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $bc7 | Export-WgpuImage -Path ./preview.png -Verbose
VERBOSE: the Bc7RgbaUnormSrgb texels were decoded on the CPU into Rgba8UnormSrgb

PS> $bc7 | Export-WgpuImage -Path ./preview-gpu.png -Decoder Gpu -Verbose
VERBOSE: the Bc7RgbaUnormSrgb texels were decoded on the GPU into Rgba16Float and packed into Rgba8UnormSrgb on the CPU
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $bc7.Save((Join-Path $PWD preview.png))
PS> $bc7.Save((Join-Path $PWD preview-gpu.png), 'Gpu')
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> $onCpu = (Read-WgpuTexture (Import-WgpuImage ./preview.png)) -join ','
PS> $onGpu = (Read-WgpuTexture (Import-WgpuImage ./preview-gpu.png)) -join ','
PS> $onCpu -eq $onGpu
True
```

A setting that does nothing is refused: `-Quality` for a BC format, `-Decoder` for a DDS or KTX2 file, which keeps the blocks, and `-Srgb` beside a GPU format, which says itself whether its color is sRGB. So is a format the module does not encode:

```powershell
PS> Import-WgpuImage ./columns.png -Format Bc7RgbaUnorm -Quality 80
Import-WgpuImage: -Quality applies to a Basis Universal -Format with a quality setting

PS> $bc7 | Export-WgpuImage -Path ./preview.dds -Decoder Gpu
Export-WgpuImage: -Decoder applies to an image file; a DDS or KTX2 file holds a block-compressed texture's blocks as they are

PS> Import-WgpuImage ./columns.png -Format Bc7RgbaUnorm -Srgb
Import-WgpuImage: -Srgb applies to an image file loaded without -Format or with a Basis Universal format; a GPU format says whether its color is sRGB, as Bc7RgbaUnormSrgb does

PS> $columns | Export-WgpuImage -Path ./columns.dds -Format Astc4x4Unorm
Export-WgpuImage: Astc4x4Unorm is not a format the module encodes on the GPU; it encodes Bc1RgbaUnorm, Bc2RgbaUnorm, Bc3RgbaUnorm and Bc7RgbaUnorm and their Srgb forms, Bc4RUnorm, Bc4RSnorm, Bc5RgUnorm, Bc5RgSnorm and Bc6hRgbUfloat, and ASTC and ETC2 textures come from the Basis Universal formats, such as BasisAstc4x4 and BasisEtc1s
```

## Change a texture's size and orientation

`Resize-WgpuTexture` makes a new texture of another size: `-Width` or `-Height`, the other side keeping the aspect ratio, or both; `-Scale`, a factor for both sides; `-MaxSize`, which shrinks a texture with a side past it and leaves a smaller one as it is; and `-PowerOfTwo`, which rounds each side to the Nearest power of two, Up or Down. Shrinking averages the texels each new texel covers, the Box filter, and enlarging uses the Catmull-Rom cubic, the Bicubic filter, each side on its own; `-Filter` names Nearest, Box, Bilinear, Bicubic or Lanczos instead. The new texture keeps the format and as many of the mip levels as its size has, rendered again from level 0, and its `Notes` say what was done:

```powershell
PS> $photo = New-WgpuTexture 8 4 -MipLevels 3 -Data ([byte[]](0..31 | ForEach-Object { [byte]($_ * 8), 64, 128, 255 }))
PS> $photo | Resize-WgpuTexture -Width 4 | Select-Object Width, Height, MipLevelCount
Width Height MipLevelCount
----- ------ -------------
    4      2             3

PS> $photo | Resize-WgpuTexture -Scale 2 -Filter Lanczos | Select-Object Width, Height
Width Height
----- ------
   16      8

PS> $photo | Resize-WgpuTexture -MaxSize 6 -PowerOfTwo Down | Select-Object Width, Height
Width Height
----- ------
    4      2

PS> ($photo | Resize-WgpuTexture -Height 2).Notes
the texture was resized to 4x2 with the Box filter on the GPU in Rgba16Float
mip levels 1 to 2 were rendered again on the GPU, each from the level above it
the texels were packed into Rgba8Unorm on the CPU
```

`-Crop` keeps a region, four numbers for its left, top, width and height, or two, a width and height `-Anchor` places, in the middle when omitted; `-CropMargin` cuts texels from the left, top, right and bottom. `-Pad` puts the texture on a larger canvas, which `-Anchor` places it on and `-PadColor` fills, transparent black when omitted. One call crops, then resizes, then pads, whatever order the parameters come in. These two by two texels are numbered so each one shows where it went:

```powershell
PS> $tiles = New-WgpuTexture 2 2 -Data ([byte[]](1..16))
PS> (Read-WgpuTexture ($tiles | Resize-WgpuTexture -Crop 1, 0, 1, 2)) -join ','
5,6,7,8,13,14,15,16

PS> (Read-WgpuTexture ($tiles | Resize-WgpuTexture -CropMargin 0, 1, 0, 0)) -join ','
9,10,11,12,13,14,15,16

PS> (Read-WgpuTexture ($tiles | Resize-WgpuTexture -Pad 3, 2 -Anchor Left -PadColor 1, 1, 1, 1)) -join ','
1,2,3,4,5,6,7,8,255,255,255,255,9,10,11,12,13,14,15,16,255,255,255,255

PS> $photo | Resize-WgpuTexture -Pad 8, 8 -Width 2 -Crop 0, 0, 4, 4 | Select-Object Width, Height
Width Height
----- ------
    8      8
```

`Convert-WgpuTexture` flips a texture with `-Flip`, Horizontal or Vertical, and turns it clockwise with `-Rotate` 90, 180 or 270, turning first when given both. The methods `Resize`, `Crop`, `Pad`, `Flip` and `Rotate` do what the cmdlets do, each with an Async form:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture ($tiles | Convert-WgpuTexture -Flip Horizontal)) -join ','
5,6,7,8,1,2,3,4,13,14,15,16,9,10,11,12

PS> (Read-WgpuTexture ($tiles | Convert-WgpuTexture -Rotate 90)) -join ','
9,10,11,12,1,2,3,4,13,14,15,16,5,6,7,8

PS> (Read-WgpuTexture ($tiles | Resize-WgpuTexture -Width 4 -Filter Nearest)) -join ','
1,2,3,4,1,2,3,4,5,6,7,8,5,6,7,8,1,2,3,4,1,2,3,4,5,6,7,8,5,6,7,8,9,10,11,12,9,10,11,12,13,14,15,16,13,14,15,16,9,10,11,12,9,10,11,12,13,14,15,16,13,14,15,16
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $tiles.Flip('Horizontal').Read() -join ','
5,6,7,8,1,2,3,4,13,14,15,16,9,10,11,12

PS> $tiles.Rotate(90).Read() -join ','
9,10,11,12,1,2,3,4,13,14,15,16,5,6,7,8

PS> $tiles.Resize(4, 0, 'Nearest').Read() -join ','
1,2,3,4,1,2,3,4,5,6,7,8,5,6,7,8,1,2,3,4,1,2,3,4,5,6,7,8,5,6,7,8,9,10,11,12,9,10,11,12,13,14,15,16,13,14,15,16,9,10,11,12,9,10,11,12,13,14,15,16,13,14,15,16

PS> $tiles.Crop(1, 0, 1, 2).Read() -join ','
5,6,7,8,13,14,15,16

PS> $tiles.Pad(3, 2, 'Left', @(1, 1, 1, 1)).Read() -join ','
1,2,3,4,5,6,7,8,255,255,255,255,9,10,11,12,13,14,15,16,255,255,255,255

PS> $tiles.RotateAsync(180).GetAwaiter().GetResult().Read() -join ','
13,14,15,16,9,10,11,12,5,6,7,8,1,2,3,4
```
{{< /tab >}}
{{< /tabs >}}

`Import-WgpuImage` and `Export-WgpuImage` take the same parameters, applied as the file loads or saves and before `-Format` and `-MipLevels` convert:

```powershell
PS> $photo | Export-WgpuImage -Path ./photo.png
PS> Import-WgpuImage ./photo.png -Rotate 90 -MaxSize 2 | Select-Object Width, Height
Width Height
----- ------
    1      2

PS> $photo | Export-WgpuImage -Path ./square.dds -Pad 8, 8 -Format Bc7RgbaUnorm
PS> Import-WgpuImage ./square.dds | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
    8      8 Bc7RgbaUnorm
```

`-Format` writes the new texture in another format, as `Import-WgpuImage -Format` loads one:

```powershell
PS> $photo | Resize-WgpuTexture -Width 2 -Format Rgba16Float | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
    2      1 Rgba16Float
```

A cube map is not flipped or rotated, and a canvas smaller than the texture is refused:

```powershell
PS> New-WgpuTexture 4 4 -Layers 6 -Cube | Convert-WgpuTexture -Rotate 90
Convert-WgpuTexture: a cube map is not flipped or rotated, as its faces would no longer meet at their edges

PS> $tiles | Resize-WgpuTexture -Pad 1, 1
Resize-WgpuTexture: -Pad grows the canvas, and 1x1 is smaller than the 2x2 texture it pads
```

## Change a texture's channels and colors

`Convert-WgpuTexture` also changes each texel. `-Swizzle` rearranges the channels by four letters of r, g, b, a, 0 and 1, one for each channel of the new texture. `-Premultiply` and `-Unpremultiply` multiply or divide red, green and blue by alpha. `-Invert`, `-Brightness`, `-Contrast` and `-Gamma` change the values the texture stores, as ImageMagick's `-negate`, `-brightness-contrast` and `-gamma` do. `-Grayscale` weighs red, green and blue with Rec. 709's weights in linear light, or as `-GrayscaleWeights` names: Rec601, on the stored values as Pillow's `convert('L')` weighs them, or three numbers. `-Exposure` multiplies linear values by 2 to its power, and `-ToneMap` maps HDR colors into the range from 0 to 1 with Reinhard, AcesFilmic, AgX or Neutral. `-ColorSpace` converts the texels to Linear or Srgb so their colors look the same, switching a format with an sRGB form to that form, and `-AssignColorSpace` relabels them as they are.

One call makes these after any change of size, in this order whatever order they are given in: `-AssignColorSpace`, `-ColorKey`, `-Swizzle`, `-ReconstructZ`, `-Normalize`, `-FlipGreen`, `-Unpremultiply`, `-AlphaThreshold`, `-Exposure`, `-ToneMap`, `-Brightness` and `-Contrast`, `-Gamma`, `-Grayscale`, `-Invert`, `-Premultiply` and `-ColorSpace`, then the filters of [Blur, sharpen and find edges](#blur-sharpen-and-find-edges), and then `-FixAlphaBorder`; [Make cutouts with alpha](#make-cutouts-with-alpha) has the three alpha steps, and [Make normal maps](#make-normal-maps) the three normal steps. On an sRGB texture, a swizzle moves the values it stores, and premultiplying works in linear light. The methods `Swizzle`, `Premultiply`, `Unpremultiply`, `ConvertColorSpace`, `AssignColorSpace`, `Grayscale`, `Invert`, `Adjust` and `ToneMap` do what the switches do, each with an Async form:

```powershell
PS> $texel = New-WgpuTexture 1 1 -Data ([byte[]](200, 100, 50, 128))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture ($texel | Convert-WgpuTexture -Swizzle bgra)) -join ','
50,100,200,128

PS> (Read-WgpuTexture ($texel | Convert-WgpuTexture -Grayscale)) -join ','
118,118,118,128

PS> (Read-WgpuTexture ($texel | Convert-WgpuTexture -Grayscale -GrayscaleWeights Rec601)) -join ','
124,124,124,128

PS> (Read-WgpuTexture ($texel | Convert-WgpuTexture -Brightness 10 -Contrast 20 -Gamma 1.2)) -join ','
251,141,79,128

PS> $premultiplied = $texel | Convert-WgpuTexture -Premultiply
PS> (Read-WgpuTexture $premultiplied) -join ','
100,50,25,128

PS> (Read-WgpuTexture ($premultiplied | Convert-WgpuTexture -Unpremultiply -Invert -Premultiply)) -join ','
28,78,103,128

PS> ($texel | Convert-WgpuTexture -ColorSpace Srgb).Format
Rgba8UnormSrgb

PS> ($texel | Convert-WgpuTexture -AssignColorSpace Srgb).Format
Rgba8UnormSrgb
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $texel.Swizzle('bgra').Read() -join ','
50,100,200,128

PS> $texel.Grayscale($null).Read() -join ','
118,118,118,128

PS> $texel.Grayscale('Rec601').Read() -join ','
124,124,124,128

PS> $texel.Adjust(10, 20, 1.2, $null).Read() -join ','
251,141,79,128

PS> $texel.InvertAsync().GetAwaiter().GetResult().Read() -join ','
55,155,205,128

PS> $premultiplied = $texel.Premultiply()
PS> $premultiplied.Read() -join ','
100,50,25,128

PS> $premultiplied.Unpremultiply().Invert().Premultiply().Read() -join ','
28,78,103,128

PS> $texel.ConvertColorSpace('Srgb').Format
Rgba8UnormSrgb

PS> $texel.AssignColorSpace('Srgb').Format
Rgba8UnormSrgb
```
{{< /tab >}}
{{< /tabs >}}

Reinhard's curve takes the brightest texel's luminance as its white point, as texconv's does, and the `Notes` give it; the other curves work on each texel alone:

```powershell
PS> $hdr = New-WgpuTexture 2 1 -Format Rgba16Float -Data ([float[]](6, 3, 1.5, 1, 0.5, 0.25, 0.125, 1))
PS> ($hdr | Convert-WgpuTexture -ToneMap Reinhard).Notes
the texels were tone mapped with Reinhard's curve on the GPU in Rgba16Float
Reinhard's white point was the brightest texel's luminance, 3.735

PS> (Read-WgpuTexture ($hdr | Convert-WgpuTexture -Exposure -1 -ToneMap AgX)) -join ','
0.840332,0.684082,0.5634766,1,0.2788086,0.1561279,0.09014893,1
```

`-Format` writes the new texture in another format: a color format, a BC format, encoded on the GPU, or a Basis Universal format, as `Import-WgpuImage` and the other cmdlets that make a texture take it. With `-ColorSpace`, a format that has an sRGB form switches to the form it names, so the values are encoded once, and an HDR texture tone mapped for a screen becomes 8-bit sRGB in one call. `-Format` alone converts each mip level as it is:

```powershell
PS> $screen = $hdr | Convert-WgpuTexture -Exposure -1 -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm
PS> $screen.Format
Rgba8UnormSrgb

PS> (Read-WgpuTexture $screen) -join ','
236,216,198,255,144,110,85,255

PS> ($hdr | Convert-WgpuTexture -Format Rgba32Float).Format
Rgba32Float
```

`Join-WgpuTexture` packs channels of several textures into one, as an occlusion, roughness and metal map packs three masks. `-Red`, `-Green`, `-Blue` and `-Alpha` each take a texture, whose red channel they read; a texture and a channel letter as a pair; or a number. An omitted color channel is 0 and an omitted alpha 1; a smaller texture is resized to the largest one's size first; and `-Format` names the new texture's format, Rgba8Unorm when omitted. `[Wgpu.Texture]::Join` does the same:

```powershell
PS> $occlusion = New-WgpuTexture 2 2 -Data ([byte[]](@(200, 200, 200, 255) * 4))
PS> $roughness = New-WgpuTexture 2 2 -Data ([byte[]](@(128, 128, 128, 255) * 4))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture (Join-WgpuTexture -Red $occlusion -Green $roughness -Blue 0)) -join ','
200,128,0,255,200,128,0,255,200,128,0,255,200,128,0,255

PS> (Read-WgpuTexture (Join-WgpuTexture $occlusion $occlusion $occlusion ($texel, 'a')))[0..3] -join ','
200,200,200,128
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> [Wgpu.Texture]::Join($occlusion, $roughness, 0, $null, $null).Read() -join ','
200,128,0,255,200,128,0,255,200,128,0,255,200,128,0,255

PS> [Wgpu.Texture]::Join($occlusion, $occlusion, $occlusion, @($texel, 'a'), $null).Read()[0..3] -join ','
200,200,200,128
```
{{< /tab >}}
{{< /tabs >}}

`Import-WgpuImage` and `Export-WgpuImage` take every switch above, applied as the file loads or saves:

```powershell
PS> $texel | Export-WgpuImage -Path ./gray.png -Grayscale
PS> (Read-WgpuTexture (Import-WgpuImage ./gray.png)) -join ','
118,118,118,128

PS> (Read-WgpuTexture (Import-WgpuImage ./gray.png -Invert)) -join ','
137,137,137,128
```

A swizzle that is not four letters is refused, and so is relabeling a format that has no sRGB form:

```powershell
PS> $texel | Convert-WgpuTexture -Swizzle rgb
Convert-WgpuTexture: -Swizzle takes four letters of r, g, b, a, 0 and 1, one for each channel of the new texture, as 'bgra' or 'rrr1'; 'rgb' is not that

PS> New-WgpuTexture 2 2 -Format Rgba16Float | Convert-WgpuTexture -AssignColorSpace Srgb
Convert-WgpuTexture: -AssignColorSpace relabels a format that has an sRGB form, as Rgba8Unorm has Rgba8UnormSrgb, and Rgba16Float has none
```

## Blur, sharpen and find edges

`-Blur` blurs a texture by a Gaussian of the standard deviation it gives, in texels: one number, or two, across and down. The Gaussian reaches 4 standard deviations to each side, as scipy's `gaussian_filter` does, and works on linear values weighed by alpha, so the color of a transparent texel does not spread into the texels around it. `-Sharpen` sharpens by an unsharp mask: it adds back that many times the difference between each texel and a Gaussian blur of it. `-SharpenRadius` is that blur's standard deviation, 1 when omitted, and a channel whose difference is less than `-SharpenThreshold` keeps its value, so smooth areas stay smooth. `-Edges` replaces the texels with their edges: Sobel, Scharr and Prewitt give the size of each channel's gradient by those kernels, scaled so a step from 0 to 1 reads 1, and Laplacian the size of each channel's second derivative, all of the values the texture stores; alpha is kept. `-FilterEdge` says what the filters read past the edge of the texture: Clamp, the edge texel repeated, the default; Wrap, the other side, for a texture that tiles; or Mirror.

One call runs them after the changes to each texel and before `-FixAlphaBorder`: the blur, then the unsharp mask, then the edges, on level 0 of each layer, and of each depth slice of a 3D texture, on its own, and the mip levels are rendered again from it. The methods `Blur`, `Sharpen` and `FindEdges` do what the switches do, each with an Async form. Here they run on a row of five grays:

```powershell
PS> $grays = New-WgpuTexture 5 1 -Data ([byte[]](51, 51, 51, 255, 51, 51, 51, 255, 204, 204, 204, 255, 51, 51, 51, 255, 51, 51, 51, 255))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture ($grays | Convert-WgpuTexture -Blur 1)) -join ','
59,59,59,255,88,88,88,255,112,112,112,255,88,88,88,255,59,59,59,255

PS> (Read-WgpuTexture ($grays | Convert-WgpuTexture -Sharpen 0.25)) -join ','
49,49,49,255,42,42,42,255,227,227,227,255,42,42,42,255,49,49,49,255

PS> (Read-WgpuTexture ($grays | Convert-WgpuTexture -Sharpen 0.25 -SharpenThreshold 0.2)) -join ','
51,51,51,255,51,51,51,255,227,227,227,255,51,51,51,255,51,51,51,255

PS> (Read-WgpuTexture ($grays | Convert-WgpuTexture -Edges Sobel)) -join ','
0,0,0,255,153,153,153,255,0,0,0,255,153,153,153,255,0,0,0,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $grays.Blur(1, $null).Read() -join ','
59,59,59,255,88,88,88,255,112,112,112,255,88,88,88,255,59,59,59,255

PS> $grays.Sharpen(0.25, $null, $null, $null).Read() -join ','
49,49,49,255,42,42,42,255,227,227,227,255,42,42,42,255,49,49,49,255

PS> $grays.SharpenAsync(0.25, $null, 0.2, $null).GetAwaiter().GetResult().Read() -join ','
51,51,51,255,51,51,51,255,227,227,227,255,51,51,51,255,51,51,51,255

PS> $grays.FindEdges('Sobel', $null, $null, $null).Read() -join ','
0,0,0,255,153,153,153,255,0,0,0,255,153,153,153,255,0,0,0,255
```
{{< /tab >}}
{{< /tabs >}}

Canny finds thin, linked edges in the luminance, as scikit-image's `canny` does. It smooths the luminance by a Gaussian of standard deviation `-EdgeSigma`, 1 when omitted, keeps each texel whose gradient is the largest across the edge, and keeps an edge whose gradient is at least the low of `-EdgeThreshold`'s two numbers where it links to a texel at least the high one, 0.1 and 0.2 when omitted. The result is 1 on an edge and 0 elsewhere, and 0 in the outermost ring of texels. Here it outlines a disc:

```powershell
PS> $disc = New-WgpuTexture 12 12 -Data ([byte[]]@(foreach ($y in 0..11) { foreach ($x in 0..11) { $v = if (($x - 5.6) * ($x - 5.6) + ($y - 6.3) * ($y - 6.3) -lt 16) { 255 } else { 0 }; $v, $v, $v, 255 } }))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $edges = Read-WgpuTexture ($disc | Convert-WgpuTexture -Edges Canny)
PS> foreach ($y in 0..11) { -join (0..11 | ForEach-Object { if ($edges[($y * 12 + $_) * 4]) { '#' } else { '.' } }) }
............
............
............
...######...
..##....##..
..#......#..
..#......#..
..#......#..
..#......#..
..##....##..
....#####...
............
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $edges = $disc.FindEdges('Canny', $null, $null, $null).Read()
PS> foreach ($y in 0..11) { -join (0..11 | ForEach-Object { if ($edges[($y * 12 + $_) * 4]) { '#' } else { '.' } }) }
............
............
............
...######...
..##....##..
..#......#..
..#......#..
..#......#..
..#......#..
..##....##..
....#####...
............
```
{{< /tab >}}
{{< /tabs >}}

`Import-WgpuImage` and `Export-WgpuImage` take `-Blur`, `-Sharpen`, `-SharpenRadius`, `-SharpenThreshold` and `-FilterEdge`, and a texture's `Notes` say what the filters did. A setting out of range is refused, and so is a setting given without its filter:

```powershell
PS> $grays | Export-WgpuImage -Path ./soft.png -Blur 1
PS> (Read-WgpuTexture (Import-WgpuImage ./soft.png)) -join ','
59,59,59,255,88,88,88,255,112,112,112,255,88,88,88,255,59,59,59,255

PS> ($disc | Convert-WgpuTexture -Edges Canny).Notes
Canny's edges were found on the GPU in the luminance smoothed by a Gaussian of a standard deviation of 1 texel, kept above 0.1 where they link to one above 0.2, the edge texels repeated past the edges
the texels were packed into Rgba8Unorm on the CPU

PS> $grays | Convert-WgpuTexture -Blur 0
Convert-WgpuTexture: -Blur takes standard deviations above 0, and 0 is not

PS> $grays | Convert-WgpuTexture -EdgeSigma 2
Convert-WgpuTexture: -EdgeThreshold and -EdgeSigma say how Canny finds edges: give -Edges Canny
```

## Make cutouts with alpha

`-ColorKey` gives alpha 0 to the texels of one color, as a sprite sheet's magenta background asks, and leaves every other texel's alpha as it is. The color is red, green and blue from 0 to 1 as the texture stores them, rounded to the texture's own precision, so 1, 0, 1 is an 8-bit 255, 0, 255; or `'#RRGGBB'`. `-ColorKeyTolerance` lets each channel differ from the key by up to that much, for a key a JPEG has blurred. `-AlphaThreshold` bakes an alpha test into the texture: alpha below it becomes 0 and the rest 1. The key reads the colors as the texture holds them, before a swizzle, and the threshold cuts after a swizzle and unpremultiplying and before premultiplying, so a cut texel premultiplies to nothing. The methods `ColorKey` and `AlphaThreshold` do the same, each with an Async form:

```powershell
PS> $sprite = New-WgpuTexture 2 2 -Data ([byte[]](255, 0, 255, 255, 250, 5, 250, 255, 40, 80, 120, 100, 40, 80, 120, 200))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture ($sprite | Convert-WgpuTexture -ColorKey '#FF00FF')) -join ','
255,0,255,0,250,5,250,255,40,80,120,100,40,80,120,200

PS> (Read-WgpuTexture ($sprite | Convert-WgpuTexture -ColorKey 1, 0, 1 -ColorKeyTolerance 0.05)) -join ','
255,0,255,0,250,5,250,0,40,80,120,100,40,80,120,200

PS> (Read-WgpuTexture ($sprite | Convert-WgpuTexture -AlphaThreshold 0.5)) -join ','
255,0,255,255,250,5,250,255,40,80,120,0,40,80,120,255

PS> (Read-WgpuTexture ($sprite | Convert-WgpuTexture -AlphaThreshold 0.5 -Premultiply))[8..15] -join ','
0,0,0,0,40,80,120,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $sprite.ColorKey('#FF00FF', $null).Read() -join ','
255,0,255,0,250,5,250,255,40,80,120,100,40,80,120,200

PS> $sprite.ColorKeyAsync(@(1, 0, 1), 0.05).GetAwaiter().GetResult().Read() -join ','
255,0,255,0,250,5,250,0,40,80,120,100,40,80,120,200

PS> $sprite.AlphaThreshold(0.5).Read() -join ','
255,0,255,255,250,5,250,255,40,80,120,0,40,80,120,255
```
{{< /tab >}}
{{< /tabs >}}

`Import-WgpuImage` and `Export-WgpuImage` take both, and a texture's `Notes` say what they did. A threshold that is not above 0 and at most 1 is refused, and so is either on a format with no alpha channel:

```powershell
PS> ($sprite | Convert-WgpuTexture -ColorKey '#FF00FF').Notes
the texels were made transparent where red, green and blue were 1, 0 and 1 on the GPU in Rgba16Float
the texels were packed into Rgba8Unorm on the CPU

PS> $sprite | Convert-WgpuTexture -AlphaThreshold 0
Convert-WgpuTexture: -AlphaThreshold is above 0 and at most 1, not 0

PS> New-WgpuTexture 2 2 -Format R8Unorm | Convert-WgpuTexture -AlphaThreshold 0.5
Convert-WgpuTexture: R8Unorm has no alpha channel, so -AlphaThreshold would change nothing
```

`-FixAlphaBorder` gives each texel with alpha 0 the color of the nearest texel with alpha above 0 and leaves its alpha at 0, so filtering and mip levels blend no dark or keyed color in at a cutout's edge; NVIDIA's Texture Tools Exporter fills transparent areas for the same reason with its Edge Pad effect. It runs after every change to each texel and after the filters, and `-BorderReach` limits it to the texels within that many texels of one with alpha above 0. The method `FixAlphaBorder` takes the reach, or `$null` for none, and has an Async form:

```powershell
PS> $row = New-WgpuTexture 6 1 -Data ([byte[]](200, 10, 20, 255) + [byte[]]::new(16) + [byte[]](30, 40, 220, 255))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture ($row | Convert-WgpuTexture -FixAlphaBorder)) -join ','
200,10,20,255,200,10,20,0,200,10,20,0,30,40,220,0,30,40,220,0,30,40,220,255

PS> (Read-WgpuTexture ($row | Convert-WgpuTexture -FixAlphaBorder -BorderReach 1)) -join ','
200,10,20,255,200,10,20,0,0,0,0,0,0,0,0,0,30,40,220,0,30,40,220,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $row.FixAlphaBorder($null).Read() -join ','
200,10,20,255,200,10,20,0,200,10,20,0,30,40,220,0,30,40,220,0,30,40,220,255

PS> $row.FixAlphaBorderAsync(1).GetAwaiter().GetResult().Read() -join ','
200,10,20,255,200,10,20,0,0,0,0,0,0,0,0,0,30,40,220,0,30,40,220,255
```
{{< /tab >}}
{{< /tabs >}}

`Import-WgpuImage` and `Export-WgpuImage` take both switches, and the `Notes` say what the fix did. Premultiplied alpha keeps a texel with alpha 0 black, and filters without blending in the color of transparent texels already, so a call that gives both `-FixAlphaBorder` and `-Premultiply` is refused:

```powershell
PS> ($row | Convert-WgpuTexture -FixAlphaBorder).Notes
every transparent texel took the color of the nearest texel with alpha above 0, found on the GPU
the texels were packed into Rgba8Unorm on the CPU

PS> $row | Convert-WgpuTexture -FixAlphaBorder -Premultiply
Convert-WgpuTexture: -FixAlphaBorder colors texels with alpha 0, and a premultiplied texel with alpha 0 stays black: give -FixAlphaBorder or -Premultiply, not both
```

`-KeepCoverage`, in [Give a texture mip levels](#give-a-texture-mip-levels), keeps a cutout from fading as its mip levels shrink.

## Make normal maps

`ConvertTo-WgpuNormalMap` makes a normal map from a height map: for each texel the normal, the direction the surface faces there, from the slopes of the heights around it. `-Strength` sets how steep the slopes are: a height step of 1 between neighboring texels, with heights running from 0 to 1, gives a slope of the strength, as texconv's `-nmapamp` does; 1 when omitted. X grows to the right and Y up the image, as glTF reads a normal map, and `-FlipGreen` points Y down, the convention named after DirectX. The new texture is Rgba8Unorm, or the `-Format` named. A format that holds no negative values stores each normal n as n / 2 + 1 / 2, so a flat texel is 128, 128, 255, and any other stores n. The method `ToNormalMap` takes the strength, the height channel, the kernel, the edge and the alpha, each `$null` for its default, makes Rgba8Unorm with green up, and has an Async form. This height map rises 64 a texel to the right, so each normal leans left:

```powershell
PS> $ramp = New-WgpuTexture 4 4 -Format R8Unorm -Data ([byte[]](@(0, 64, 128, 192) * 4))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture ($ramp | ConvertTo-WgpuNormalMap -Strength 4))[0..3] -join ','
37,128,217,255

PS> (Read-WgpuTexture ($ramp | ConvertTo-WgpuNormalMap -Strength 4 -Format Rgba32Float))[0..3] -join ','
-0.7084892,0,0.7057216,1
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $ramp.ToNormalMap(4, $null, $null, $null, $null).Read()[0..3] -join ','
37,128,217,255

PS> $ramp.ToNormalMapAsync(4, $null, $null, $null, $null).GetAwaiter().GetResult().Read()[0..3] -join ','
37,128,217,255
```
{{< /tab >}}
{{< /tabs >}}

`-HeightChannel` reads the heights from Red, the default, Green, Blue or Alpha, or the Luminance of red, green and blue by texconv's weights. `-NormalAlpha`, or `-Alpha`, puts 1 in alpha, the default; the Height, which parallax mapping reads; or texconv's Occlusion term, darker where the eight neighbors rise above the texel. This height map is in alpha, and rises 64 a texel down the image:

```powershell
PS> $slope = New-WgpuTexture 4 4 -Data ([byte[]](0..15 | ForEach-Object { 0, 0, 0, (($_ -shr 2) * 64) }))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture ($slope | ConvertTo-WgpuNormalMap -HeightChannel Alpha -Strength 4))[0..3] -join ','
128,218,217,255

PS> (Read-WgpuTexture ($slope | ConvertTo-WgpuNormalMap -HeightChannel Alpha -Strength 4 -FlipGreen))[0..3] -join ','
128,37,217,255

PS> (Read-WgpuTexture ($slope | ConvertTo-WgpuNormalMap -HeightChannel Alpha -Alpha Height))[16..19] -join ','
128,159,251,64

PS> (Read-WgpuTexture ($slope | ConvertTo-WgpuNormalMap -HeightChannel Alpha -Strength 4 -Alpha Occlusion))[3]
165
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $slope.ToNormalMap(4, 'Alpha', $null, $null, $null).Read()[0..3] -join ','
128,218,217,255

PS> $slope.ToNormalMap(4, 'Alpha', $null, $null, $null).FlipGreen().Read()[0..3] -join ','
128,37,217,255

PS> $slope.ToNormalMap($null, 'Alpha', $null, $null, 'Height').Read()[16..19] -join ','
128,159,251,64

PS> $slope.ToNormalMap(4, 'Alpha', $null, $null, 'Occlusion').Read()[3]
165
```
{{< /tab >}}
{{< /tabs >}}

`-NormalKernel`, or `-Kernel`, takes the slopes. Prewitt, texconv's and the default, averages the differences of three rows or columns; Central takes the two neighbors alone; Sobel weighs the middle difference twice; and Sobel5x5, NVIDIA Texture Tools' kernel, reaches two texels each way. Each is scaled so a steady ramp gives the same slope, and they differ where the slope changes, as beside a single raised texel:

```powershell
PS> $peak = New-WgpuTexture 5 5 -Format R8Unorm -Data ([byte[]](0..24 | ForEach-Object { if ($_ -eq 12) { 255 } else { 0 } }))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> 'Prewitt', 'Central', 'Sobel', 'Sobel5x5' | ForEach-Object { '{0,-8} {1}' -f $_, ((Read-WgpuTexture ($peak | ConvertTo-WgpuNormalMap -Kernel $_))[44..46] -join ',') }
Prewitt  107,128,253
Central  70,128,242
Sobel    97,128,251
Sobel5x5 120,128,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> 'Prewitt', 'Central', 'Sobel', 'Sobel5x5' | ForEach-Object { '{0,-8} {1}' -f $_, ($peak.ToNormalMap($null, $null, $_, $null, $null).Read()[44..46] -join ',') }
Prewitt  107,128,253
Central  70,128,242
Sobel    97,128,251
Sobel5x5 120,128,255
```
{{< /tab >}}
{{< /tabs >}}

`-HeightEdge`, or `-Edge`, says what is read past the edges. Continue, the default, carries the slope at each edge on past it, so an edge texel leans as its neighbors do; Wrap reads the other side, for a height map that tiles; Clamp repeats the edge texel; and Mirror mirrors the map at its edge. Sobel5x5 reads two texels past an edge, so the last texel of the ramp's first row differs under each:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> 'Continue', 'Wrap', 'Clamp', 'Mirror' | ForEach-Object { '{0,-8} {1}' -f $_, ((Read-WgpuTexture ($ramp | ConvertTo-WgpuNormalMap -Strength 4 -Kernel Sobel5x5 -Edge $_))[12..14] -join ',') }
Continue 37,128,217
Wrap     179,128,244
Clamp    70,128,241
Mirror   84,128,247
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> 'Continue', 'Wrap', 'Clamp', 'Mirror' | ForEach-Object { '{0,-8} {1}' -f $_, ($ramp.ToNormalMap(4, $null, 'Sobel5x5', $_, $null).Read()[12..14] -join ',') }
Continue 37,128,217
Wrap     179,128,244
Clamp    70,128,241
Mirror   84,128,247
```
{{< /tab >}}
{{< /tabs >}}

The verbose stream and the `Notes` say how the normals were made, and without Wrap, when the height map looks like it tiles: when the mean step across a pair of its opposite edges is no larger than the largest between neighboring columns, or rows, inside it, and no more than three times their median. These waves repeat across the map both ways:

```powershell
PS> $waves = New-WgpuTexture 8 8 -Format R8Unorm -Data ([byte[]](0..63 | ForEach-Object { 128 + [int](100 * [math]::Sin([math]::PI * ($_ % 8) / 4) * [math]::Cos([math]::PI * ($_ -shr 3) / 4)) }))
PS> ($waves | ConvertTo-WgpuNormalMap).Notes
the heights, read from red, were made into normals on the GPU by the Prewitt kernel at strength 1, the slope at each edge carried on past it; green points up, as glTF reads it, and alpha holds 1
the height map looks like it tiles, as its left and right edges, and its top and bottom, join as neighboring texels do: the Wrap edge reads across them
the texels were packed into Rgba8Unorm on the CPU

PS> ($waves | ConvertTo-WgpuNormalMap -Edge Wrap).Notes
the heights, read from red, were made into normals on the GPU by the Prewitt kernel at strength 1, the edges wrapped; green points up, as glTF reads it, and alpha holds 1
the texels were packed into Rgba8Unorm on the CPU
```

`-Format` names the new texture's format: a color format, a BC format, encoded on the GPU, or a Basis Universal format. `-TwoChannel` keeps X and Y alone, for a shader to rebuild Z: in red and green for a format with two channels, such as Rg8Unorm or Bc5RgUnorm, and X in red, green and blue with Y in alpha for a Basis Universal LDR format. A format that would store the normals as sRGB colors is refused, and so is `-TwoChannel` for a format with neither layout:

```powershell
PS> (Read-WgpuTexture ($ramp | ConvertTo-WgpuNormalMap -Strength 4 -Format Rg8Unorm -TwoChannel))[0..1] -join ','
37,128

PS> ($ramp | ConvertTo-WgpuNormalMap -Strength 4 -Format Bc5RgUnorm -TwoChannel).Format
Bc5RgUnorm

PS> $ramp | ConvertTo-WgpuNormalMap -Format Rgba8UnormSrgb
ConvertTo-WgpuNormalMap: a normal map stores directions, which Rgba8UnormSrgb would encode as sRGB colors: give its linear form, Rgba8Unorm

PS> $ramp | ConvertTo-WgpuNormalMap -TwoChannel
ConvertTo-WgpuNormalMap: -TwoChannel keeps X and Y apart for a format with two channels, such as Rg8Unorm or Bc5RgUnorm, or a Basis Universal LDR format, which takes X in red, green and blue and Y in alpha; Rgba8Unorm is neither
```

The new texture has the height map's mip levels. Averaging normals shortens them, which dims the light a shader works out from them, so each texel of each level is scaled back to length 1; with `-NormalMips SlopeSpace` each normal's slopes are filtered instead, and made a unit normal again, as NVIDIA's exporter does. `-MipFilter` and `-MipEdge` choose the filter as for any texture, except Min and Max, which take each channel apart. `Import-WgpuImage`, `Export-WgpuImage`, `Convert-WgpuTexture` and `Resize-WgpuTexture` take `-NormalMips` for a texture of normals, whose levels are averaged as colors without it. Here a normal map made from rough heights gets its levels both ways:

```powershell
PS> $rough = New-WgpuTexture 8 8 -Format R8Unorm -Data ([byte[]](0..63 | ForEach-Object { ($_ * 37 + ($_ -shr 3) * 91) % 256 }))
PS> $rough | ConvertTo-WgpuNormalMap -Strength 2 | Export-WgpuImage -Path ./rough.png
PS> function Get-Length($texture, $level) { $t = Read-WgpuTexture $texture -MipLevel $level; $n = 0..2 | ForEach-Object { $t[$_] / 255 * 2 - 1 }; [math]::Round([math]::Sqrt($n[0] * $n[0] + $n[1] * $n[1] + $n[2] * $n[2]), 3) }
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $averaged = Import-WgpuImage ./rough.png -MipLevels 0
PS> (1..3 | ForEach-Object { Get-Length $averaged $_ }) -join ', '
0.925, 0.94, 0.891

PS> $kept = Import-WgpuImage ./rough.png -MipLevels 0 -NormalMips Renormalize
PS> (1..3 | ForEach-Object { Get-Length $kept $_ }) -join ', '
0.997, 0.996, 1.001

PS> $kept.Notes
mip levels 1 to 3 were rendered on the GPU in Rgba16Float, each from the level above it, each texel's normal scaled back to length 1
mip levels 1 to 3 were packed into Rgba8Unorm on the CPU
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $normals = Import-WgpuImage ./rough.png
PS> $averaged = $normals.GenerateMips(0)
PS> (1..3 | ForEach-Object { Get-Length $averaged $_ }) -join ', '
0.925, 0.94, 0.891

PS> $kept = $normals.GenerateMips(0, $null, $null, $null, 'Renormalize')
PS> (1..3 | ForEach-Object { Get-Length $kept $_ }) -join ', '
0.997, 0.996, 1.001

PS> $kept.Notes
mip levels 1 to 3 were rendered on the GPU in Rgba16Float, each from the level above it, each texel's normal scaled back to length 1
mip levels 1 to 3 were packed into Rgba8Unorm on the CPU
```
{{< /tab >}}
{{< /tabs >}}

Min and Max with `-NormalMips` are refused:

```powershell
PS> Import-WgpuImage ./rough.png -MipLevels 0 -MipFilter Max -NormalMips Renormalize
Import-WgpuImage: Max takes each channel's largest value on its own, which makes no normal of a normal's channels: give -NormalMips with Box, Triangle, Kaiser, Mitchell or Lanczos
```

`Convert-WgpuTexture` changes the normals a texture holds, right after `-Swizzle`. `-ReconstructZ` rebuilds each normal's Z, in blue, from its X and Y, as for a normal map stored in two channels; `-Normalize` scales each normal to length 1; and `-FlipGreen` turns green between glTF's convention and DirectX's. Each reads a normal n stored as n / 2 + 1 / 2 in a format that holds no negative values, and as n in any other, and writes it back the same way. The methods `ReconstructZ`, `Normalize` and `FlipGreen` do the same, each with an Async form. These two normals have a Z of -1, as the blue of a map stored in two channels might hold:

```powershell
PS> $pair = New-WgpuTexture 2 1 -Data ([byte[]](230, 200, 0, 255, 128, 64, 0, 255))
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuTexture ($pair | Convert-WgpuTexture -ReconstructZ)) -join ','
230,200,150,255,128,64,238,255

PS> (Read-WgpuTexture ($pair | Convert-WgpuTexture -Normalize)) -join ','
201,179,37,255,128,71,13,255

PS> (Read-WgpuTexture ($pair | Convert-WgpuTexture -FlipGreen)) -join ','
230,55,0,255,128,191,0,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $pair.ReconstructZ().Read() -join ','
230,200,150,255,128,64,238,255

PS> $pair.NormalizeAsync().GetAwaiter().GetResult().Read() -join ','
201,179,37,255,128,71,13,255

PS> $pair.FlipGreen().Read() -join ','
230,55,0,255,128,191,0,255
```
{{< /tab >}}
{{< /tabs >}}

`Import-WgpuImage` and `Export-WgpuImage` take the three steps, and make heights into normals with `-HeightToNormal`, which takes the strength, beside `-HeightChannel`, `-NormalKernel`, `-HeightEdge`, `-NormalAlpha`, `-FlipGreen` and `-TwoChannel` as `ConvertTo-WgpuNormalMap` takes them. The normals are made after the changes of size, orientation and texels, and before `-Format` and the mip levels, which keep them unit length:

```powershell
PS> $ramp | Export-WgpuImage -Path ./ramp.png
PS> (Read-WgpuTexture (Import-WgpuImage ./ramp.png -HeightToNormal 4))[0..3] -join ','
37,128,217,255

PS> Export-WgpuImage $ramp ./ramp-normal.dds -HeightToNormal 4 -Format Bc5RgUnorm -MipLevels 0
PS> Import-WgpuImage ./ramp-normal.dds | Select-Object Format, MipLevelCount
Format     MipLevelCount
------     -------------
Bc5RgUnorm             3
```

`-NormalMips` marks the texels as normals, so a `-Format` that stores them the other way takes each one written again: n / 2 + 1 / 2 in a format that holds no negative values and n in any other, its Z rebuilt where the texels hold X and Y alone. A normal map saved as an image file comes back as signed normals this way:

```powershell
PS> $ramp | ConvertTo-WgpuNormalMap -Strength 4 | Export-WgpuImage -Path ./ramp-normal.png
PS> $signed = Import-WgpuImage ./ramp-normal.png -Format Rgba16Float -NormalMips Renormalize
PS> (Read-WgpuTexture $signed)[0..3] -join ','
-0.7099609,0.00390625,0.7011719,1

PS> $signed.Notes
the normals of mip level 0 were written again on the CPU as Rgba16Float stores them, n itself rather than n / 2 + 1 / 2
mip level 0 was packed into Rgba16Float on the CPU
```

A normal step on a format without the channel it writes is refused, and so are settings for heights without `-HeightToNormal`:

```powershell
PS> New-WgpuTexture 2 2 -Format Rg8Unorm | Convert-WgpuTexture -ReconstructZ
Convert-WgpuTexture: Rg8Unorm has no blue channel to hold the Z it rebuilds, so -ReconstructZ would change nothing; Import-WgpuImage and Export-WgpuImage write the texels into a -Format that has it

PS> Import-WgpuImage ./ramp.png -HeightChannel Alpha
Import-WgpuImage: -HeightChannel, -NormalKernel, -HeightEdge and -NormalAlpha say how -HeightToNormal makes normals from heights: give -HeightToNormal
```

## Make cube maps and lighting maps

A cube map holds the light from every direction as six square faces, the layers of one texture in the order +X, -X, +Y, -Y, +Z and -Z, and its `IsCube` says it is one. `ConvertTo-WgpuCubeMap` makes one from six textures, its faces as they are, and the method `CubeFromFaces` does the same. These six faces are each one shade of red:

```powershell
PS> $faces = foreach ($face in 0..5) { New-WgpuTexture 2 2 -Data ([byte[]](@(($face * 40), 0, 0, 255) * 4)) }
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $cube = ConvertTo-WgpuCubeMap $faces
PS> $cube | Select-Object Width, DepthOrArrayLayers, IsCube
Width DepthOrArrayLayers IsCube
----- ------------------ ------
    2                  6   True
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $cube = [Wgpu.Texture]::CubeFromFaces($faces, $null)
PS> $cube | Select-Object Width, DepthOrArrayLayers, IsCube
Width DepthOrArrayLayers IsCube
----- ------------------ ------
    2                  6   True
```
{{< /tab >}}
{{< /tabs >}}

`-PositiveX`, `-NegativeX`, `-PositiveY`, `-NegativeY`, `-PositiveZ` and `-NegativeZ` name each face instead. Faces must share one size and format:

```powershell
PS> ConvertTo-WgpuCubeMap ($faces[0..4] + (New-WgpuTexture 4 4))
ConvertTo-WgpuCubeMap: the -Z face is 4x4 Rgba8Unorm where the +X face is 2x2 Rgba8Unorm: a cube's faces share one size and format, and Resize-WgpuTexture or Convert-WgpuTexture -Format brings a face in line
```

`ConvertFrom-WgpuCubeMap` writes the faces into one image, in a layout of square cells: a HorizontalCross, the default, or a VerticalCross; a VerticalCrossTurned, whose -Z face is upside down; a HorizontalTee; a HorizontalStrip or a VerticalStrip, the faces in order; or Godot's Grid2x3 and Grid3x2. The cells no face takes are transparent black. `ConvertTo-WgpuCubeMap` reads the image back by its shape, 4:3 a horizontal cross, 3:4 a vertical one, 6:1 and 1:6 the strips, and 2:3 and 3:2 the grids; `-Layout` names the tee and the turned cross, which share the crosses' shapes. The faces come back as they were:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $cross = ConvertFrom-WgpuCubeMap $cube
PS> '{0}x{1}' -f $cross.Width, $cross.Height
8x6

PS> ($cross | ConvertTo-WgpuCubeMap | Compare-WgpuTexture $cube | Measure-Object MSE -Maximum).Maximum
0

PS> $tee = ConvertFrom-WgpuCubeMap $cube -Layout HorizontalTee
PS> ($tee | ConvertTo-WgpuCubeMap -Layout HorizontalTee | Compare-WgpuTexture $cube | Measure-Object MSE -Maximum).Maximum
0
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $cross = $cube.ToLayout('HorizontalCross', $null, $null, $null, $null)
PS> '{0}x{1}' -f $cross.Width, $cross.Height
8x6

PS> ($cross.ToCubeMap($null, $null, $null, $null).Compare($cube) | Measure-Object MSE -Maximum).Maximum
0

PS> $tee = $cube.ToLayout('HorizontalTee', $null, $null, $null, $null)
PS> ($tee.ToCubeMap('HorizontalTee', $null, $null, $null).Compare($cube) | Measure-Object MSE -Maximum).Maximum
0
```
{{< /tab >}}
{{< /tabs >}}

A panorama holds every direction in an image twice as wide as it is high, its middle facing +X and its left quarter -Z, as three.js and Khronos's glTF IBL sampler read one. This one, [sky.hdr](/images/sky.hdr), is a clear sky with the sun 30 degrees up, over green ground; [sky.exr](/images/sky.exr) holds the same light. Tone mapped for a screen, it looks like this:

![A panorama of a clear sky: blue overhead, paler toward the horizon and around the sun on the right, over flat green ground](/images/sky.png)

`ConvertTo-WgpuCubeMap` reads a 2:1 image as a panorama and resamples it on the GPU: each texel of a face is the mean of a grid of samples over it, each weighed by the solid angle it covers. The faces are a quarter of the panorama's width, 128 texels here, or `-FaceSize`. This cube of 64-texel faces, tone mapped and laid out as a cross, shows the sun on the edge between the -X face and the +Z face:

```powershell
PS> $sky = Import-WgpuImage ./sky.hdr
PS> $shown = $sky | ConvertTo-WgpuCubeMap -FaceSize 64 | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm
PS> $shown | ConvertFrom-WgpuCubeMap | Export-WgpuImage -Path ./sky-cross.png
```

![The sky as a horizontal cross: the top face blue, the four side faces in a row with the horizon across them and the sun where the -X and +Z faces meet, and the green bottom face](/images/sky-cross.png)

`Export-WgpuImage -Layer` saves one face of the cube as an image of its own, +X, -X, +Y, -Y, +Z and -Z in turn:

```powershell
PS> $names = 'px', 'nx', 'py', 'ny', 'pz', 'nz'
PS> foreach ($face in 0..5) { Export-WgpuImage $shown "./sky-$($names[$face]).png" -Layer $face }
```

![The +X face: sky over ground](/images/sky-px.png) ![The -X face, with the sun at its right edge](/images/sky-nx.png) ![The +Y face: the sky overhead](/images/sky-py.png) ![The -Y face: the ground](/images/sky-ny.png) ![The +Z face, with the sun at its left edge](/images/sky-pz.png) ![The -Z face, away from the sun](/images/sky-nz.png)

`-Yaw` turns a panorama about the vertical axis before it is read, +X toward -Z, so 45 degrees brings the sun to the middle of the +Z face:

```powershell
PS> $sky | ConvertTo-WgpuCubeMap -FaceSize 64 -Yaw 45 | Measure-WgpuTexture | Where-Object Channel -EQ Red | Select-Object Layer, Maximum
Layer Maximum
----- -------
    0    1.04
    1    1.04
    2    0.57
    3    0.10
    4 1496.28
    5    0.63
```

`ConvertFrom-WgpuCubeMap -Panorama` makes a panorama 4 faces wide and 2 high, resampled the same way, with `-Yaw` turning it back. `-Layout AngularMap` makes an angular map, a disc whose middle faces -Z and whose rim faces +Z, and `-Layout Octahedral` an octahedral map, a square whose middle faces +Y; `ConvertTo-WgpuCubeMap` reads either square image when `-Layout` names it. Each step averages over solid angles, so a round trip blurs a little. The sky through a cube of 128-texel faces and back, measured against the sun's brightness as the peak:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $back = $sky | ConvertTo-WgpuCubeMap | ConvertFrom-WgpuCubeMap -Panorama
PS> $back | Compare-WgpuTexture $sky | Where-Object Channel -NE Alpha | Select-Object Channel, PSNR, SSIM
Channel  PSNR SSIM
-------  ---- ----
    Red 50.92 1.00
  Green 50.93 1.00
   Blue 50.93 1.00
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $back = $sky.ToCubeMap($null, $null, $null, $null).ToPanorama($null, $null, $null, $null)
PS> $back.Compare($sky) | Where-Object Channel -NE Alpha | Select-Object Channel, PSNR, SSIM
Channel  PSNR SSIM
-------  ---- ----
    Red 50.92 1.00
  Green 50.93 1.00
   Blue 50.93 1.00
```
{{< /tab >}}
{{< /tabs >}}

`Import-WgpuImage -Cube` makes the cube while loading, with `-Layout`, `-Yaw` and `-FaceSize` as `ConvertTo-WgpuCubeMap` takes them, and `Export-WgpuImage -Layout` or `-Panorama` writes a cube into an image file. A DDS or KTX2 file holds the cube whole, its faces as they are:

```powershell
PS> Import-WgpuImage ./sky.hdr -Cube -FaceSize 64 | Select-Object Width, DepthOrArrayLayers, IsCube
Width DepthOrArrayLayers IsCube
----- ------------------ ------
   64                  6   True

PS> $skyCube = $sky | ConvertTo-WgpuCubeMap -Format Rgba16Float
PS> Export-WgpuImage $skyCube ./sky-cross.exr -Layout HorizontalCross
PS> Import-WgpuImage ./sky-cross.exr | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
  512    384 Rgba32Float

PS> Export-WgpuImage $skyCube ./sky-cube.ktx2
PS> Import-WgpuImage ./sky-cube.ktx2 | Select-Object Width, DepthOrArrayLayers, IsCube, Format
Width DepthOrArrayLayers IsCube Format
----- ------------------ ------ ------
  128                  6   True Rgba16Float
```

Image-based lighting lights a surface with an environment, which a renderer's shader reads from three maps: the split sum Karis described for Unreal Engine 4, which glTF viewers and Filament use. `ConvertTo-WgpuIrradianceMap` makes the first, for diffuse light: each texel the light a surface facing its direction receives from every direction, weighed by the cosine of the angle between them and divided by pi, so a shader multiplies it by the surface's albedo. Each texel sums the whole environment, brought to 64 texels a face, each of those texels weighed by its exact solid angle; the map is 32 texels a face, or `-FaceSize`, in Rgba16Float, or the `-Format` named. Facing up, the sky's irradiance is the blue sky's and the sun's, and facing down the green ground's:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $irradiance = $sky | ConvertTo-WgpuIrradianceMap -Harmonics
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $irradiance = $sky.ToIrradianceMap($null, $true, $null)
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> $irradiance | Select-Object Width, DepthOrArrayLayers, Format
Width DepthOrArrayLayers Format
----- ------------------ ------
   32                  6 Rgba16Float

PS> $middle = (16 * 32 + 16) * 4
PS> ((Read-WgpuTexture $irradiance -Layer 2)[$middle..($middle + 2)] | ForEach-Object { [math]::Round($_, 3) }) -join ', '
0.816, 0.878, 1.075

PS> ((Read-WgpuTexture $irradiance -Layer 3)[$middle..($middle + 2)] | ForEach-Object { [math]::Round($_, 3) }) -join ', '
0.1, 0.099, 0.061
```

`-Harmonics` also gives the map a `Harmonics` property: the irradiance as nine spherical harmonics for each color channel, in the basis and scale Filament's cmgen writes with `--sh-irradiance`, which a shader can sum in place of reading the cube. Facing up, at (0, 1, 0), four of the nine basis functions are not 0, and their sum, divided by pi, comes near the cube's red facing up; nine harmonics hold no detail as small as the sun, so the two differ by what the sun adds near it:

```powershell
PS> $irradiance.Harmonics.Red | ForEach-Object { [math]::Round($_, 4) }
6.0069
-2.3034
2.8193
2.8193
-1.0748
-1.0748
0.2738
1.3353
0.4742

PS> $red = $irradiance.Harmonics.Red
PS> [math]::Round((0.282095 * $red[0] - 0.488603 * $red[1] - 0.315392 * $red[6] - 0.546274 * $red[8]) / [math]::PI, 3)
0.788
```

`ConvertTo-WgpuSpecularMap` makes the second map, for reflections: each mip level the environment as the GGX lobe of one roughness reflects it, which a shader reads at the level of the surface's roughness in the direction a ray reflects. Level 0, roughness 0, is the environment itself. `-RoughnessMapping Linear`, the default, gives level n of N levels perceptual roughness n / (N - 1), down to 2x2, as Khronos's glTF IBL sampler writes the levels and glTF viewers read them; Quadratic gives it 1 - sqrt(1 - n / (N - 1)), down to 16x16, as Filament's cmgen writes them. Each texel takes 1024 samples of the lobe, or `-Samples`; `-MipLevels` sets N:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $specular = $sky | ConvertTo-WgpuSpecularMap
PS> $specular | Select-Object Width, MipLevelCount, Format
Width MipLevelCount Format
----- ------------- ------
  128             7 Rgba16Float

PS> ($sky | ConvertTo-WgpuSpecularMap -RoughnessMapping Quadratic).MipLevelCount
4
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $specular = $sky.ToSpecularMap($null, $null, $null, $null)
PS> $specular | Select-Object Width, MipLevelCount, Format
Width MipLevelCount Format
----- ------------- ------
  128             7 Rgba16Float

PS> $sky.ToSpecularMap('Quadratic', $null, $null, $null).MipLevelCount
4
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> $specular.Notes
the panorama was resampled on the GPU into faces 128 texels across in Rgba32Float, each texel the mean of 3 by 3 samples over it, each weighed by the solid angle it covers
the specular map was prefiltered on the GPU into 7 mip levels from 128 texels a face in Rgba32Float: level 0 the environment itself, and each other level the GGX lobe of perceptual roughness level / (levels - 1), each sample read at the mip level Khronos's sampler computes, by 1024 samples a texel
the texels were packed into Rgba16Float on the CPU

PS> ($specular | Compare-WgpuTexture $skyCube | Measure-Object MSE -Maximum).Maximum
0
```

`New-WgpuBrdfTable` makes the third: for the cosine of the view's angle across and perceptual roughness down, the part of the reflected light F0 scales, in red, and the part it adds, in green, with Smith's height-correlated GGX visibility, as Khronos's sampler and Filament compute them. Texel (x, y) holds the cosine (x + 0.5) / 128 and the roughness (y + 0.5) / 128, so a shader reads it at (NdotV, roughness) and lights a surface with the specular map's color times F0 times red plus green. It is 128 texels a side in Rg16Float, or `-Size` and `-Format`. Seen head on, a smooth surface reflects F0 itself, and seen at a grazing angle nearly all the light, whatever F0 is:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $brdf = New-WgpuBrdfTable -Format Rg32Float
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $brdf = [Wgpu.Texture]::NewBrdfTable($null, $null, 'Rg32Float', $null)
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> $table = Read-WgpuTexture $brdf
PS> '{0:N4} {1:N4}' -f $table[127 * 2], $table[127 * 2 + 1]
1.0000 0.0000

PS> '{0:N4} {1:N4}' -f $table[0], $table[1]
0.0194 0.9804
```

The three maps take an environment as a cube map, or as a panorama or a layout, converted first as `ConvertTo-WgpuCubeMap` converts it, with `-Layout` and `-Yaw`. [How To Light A Scene With An Environment](How-To-Light-A-Scene-With-An-Environment.md) lights spheres with them in a shader. An image of a shape no cube comes in is refused, and so is a sample count of 0:

```powershell
PS> ConvertTo-WgpuCubeMap (New-WgpuTexture 6 5)
ConvertTo-WgpuCubeMap: a 6x5 image has no shape a cube comes in: a panorama is 2:1, a horizontal cross 4:3, a vertical cross 3:4, a strip 6:1 or 1:6, and Godot's grids 2:3 and 3:2; -Layout names a tee, a turned cross, an angular map or an octahedral map

PS> $sky | ConvertTo-WgpuSpecularMap -Samples 0
ConvertTo-WgpuSpecularMap: -Samples is how many samples each texel takes, 1 or more
```

## Compare and measure textures

`Compare-WgpuTexture` compares a texture with a reference, such as a block-compressed copy with the picture it was made from. For each mip level both have, each layer and each channel both store, it writes a row with the mean squared error of the texture's values against the reference's, `MSE`; the peak signal-to-noise ratio in decibels, `PSNR`, which grows as the copy comes closer and is infinite where it matches; and the structural similarity, `SSIM`, which is 1 where the copy keeps the reference's local structure. The texture to check comes from the pipeline, and the reference is the first argument, as it is the `Compare` method's:

```powershell
PS> $gradient = New-WgpuTexture 16 16 -Data ([byte[]](0..255 | ForEach-Object { $_, (255 - $_), ($_ * 37 % 256), 255 }))
PS> $gradient | Export-WgpuImage -Path ./gradient.png
PS> $original = Import-WgpuImage ./gradient.png
PS> $copy = Import-WgpuImage ./gradient.png -Format Bc7RgbaUnorm
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $copy | Compare-WgpuTexture $original
Level Layer Channel                  MSE             PSNR              SSIM
----- ----- -------                  ---             ----              ----
    0     0 Red     0.000198180988081507 37.0293801063442 0.994615872701009
    0     0 Green   0.000157211168781238 38.0351660353988 0.994779374864366
    0     0 Blue    0.000868836505190311 30.6106193997709 0.994040065341526
    0     0 Alpha                      0                ∞                 1
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $copy.Compare($original)
Level Layer Channel                  MSE             PSNR              SSIM
----- ----- -------                  ---             ----              ----
    0     0 Red     0.000198180988081507 37.0293801063442 0.994615872701009
    0     0 Green   0.000157211168781238 38.0351660353988 0.994779374864366
    0     0 Blue    0.000868836505190311 30.6106193997709 0.994040065341526
    0     0 Alpha                      0                ∞                 1
```
{{< /tab >}}
{{< /tabs >}}

Each value is read as the texture stores it, from 0 to 1: an sRGB texture's in its encoded form, a block-compressed texture's decoded on the CPU first. PSNR's peak is 1, and for a float texture the reference channel's largest value in that level and layer, so an HDR texture's PSNR scales with its brightness. SSIM is Wang et al.'s, as scikit-image computes it with a Gaussian window of sigma 1.5, averaged over the level less a 5-texel border, so a level under 11 texels on a side has none. The textures need the same dimension, width, height and layers; their formats and mip levels may differ, and a channel only one of them stores is left out:

```powershell
PS> Compare-WgpuTexture $original (New-WgpuTexture 8 8)
Compare-WgpuTexture: the reference is 16x16 and the texture 8x8: textures compare only at the same width and height
```

`-Difference` writes a picture of the error instead: an Rgba8Unorm texture of the textures' size, layers and shared mip levels, each texel the error on a fixed log scale, black at 2^-12 or less and full at 1 or more, each twelfth of brightness doubling the error, so the same brightness means the same error in any picture. Red, green and blue show in their own colors with alpha opaque, and `-Channel` shows one channel, alpha included, as gray. The `Difference` method takes the reference and the channel, `$null` for the three colors:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $picture = $copy | Compare-WgpuTexture $original -Difference
PS> $picture | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
   16     16 Rgba8Unorm

PS> (Read-WgpuTexture $picture)[0..7] -join ','
0,0,128,255,85,85,85,255

PS> (Read-WgpuTexture ($copy | Compare-WgpuTexture $original -Difference -Channel Blue))[0..7] -join ','
128,128,128,255,85,85,85,255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $picture = $copy.Difference($original, $null)
PS> $picture | Select-Object Width, Height, Format
Width Height Format
----- ------ ------
   16     16 Rgba8Unorm

PS> $picture.Read()[0..7] -join ','
0,0,128,255,85,85,85,255

PS> $copy.Difference($original, 'Blue').Read()[0..7] -join ','
128,128,128,255,85,85,85,255
```
{{< /tab >}}
{{< /tabs >}}

`Measure-WgpuTexture` measures one texture's values: for each mip level, layer and channel, the minimum, the maximum, the mean, the population standard deviation and a histogram, whose bins split `-Range`, 0 to 1 when omitted, into `-Bins`, 256 when omitted, so each 8-bit value has a bin of its own. Values below the range count in the first bin and values above it in the last. The `Measure` method takes the bins and the range, `$null` for either one's default:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $original | Measure-WgpuTexture | Format-Table Channel, Minimum, Maximum, Average, StandardDeviation
Channel Minimum Maximum Average StandardDeviation
------- ------- ------- ------- -----------------
    Red    0.00    1.00    0.50              0.29
  Green    0.00    1.00    0.50              0.29
   Blue    0.00    1.00    0.50              0.29
  Alpha    1.00    1.00    1.00              0.00

PS> ($original | Measure-WgpuTexture -Bins 4)[2].Histogram -join ','
64,64,64,64
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $original.Measure($null, $null) | Format-Table Channel, Minimum, Maximum, Average, StandardDeviation
Channel Minimum Maximum Average StandardDeviation
------- ------- ------- ------- -----------------
    Red    0.00    1.00    0.50              0.29
  Green    0.00    1.00    0.50              0.29
   Blue    0.00    1.00    0.50              0.29
  Alpha    1.00    1.00    1.00              0.00

PS> $original.Measure(4, $null)[2].Histogram -join ','
64,64,64,64
```
{{< /tab >}}
{{< /tabs >}}

Both read every format whose texels the module can read back. Snorm values span -1 to 1, so their PSNR peak is 2. An integer format reads through the norm format with its bits, as R8Uint reads as R8Unorm. Depth reads from 0 to 1 and stencil over 255, a depth-stencil texture's depth as Red and its stencil as Green. A multisampled texture reads as a resolve would store it, each texel's samples averaged in a shader and rounded to the format, which the verbose stream says:

```powershell
PS> $samples = New-WgpuTexture 16 16 -SampleCount 4 -Usage RenderAttachment, TextureBinding
PS> ($samples | Measure-WgpuTexture -Verbose)[0].Maximum
VERBOSE: the 4 samples of each texel of the texture were averaged in a shader and rounded to Rgba8Unorm
0
```

An NV12 or P010 texture reads its luma as Red at its size and its chroma as Green and Blue at half its width and height, as [its planes](#read-and-write-the-planes-of-nv12-and-p010) hold them, so the chroma's histograms count a quarter of the texels. It compares only with another NV12 or P010 texture, and its difference picture shows each chroma texel's error over the 2 by 2 texels that share it:

```powershell
PS> $frame = New-WgpuTexture 4 4 -Format NV12
PS> Measure-WgpuTexture $frame -Bins 1 | Format-Table Channel, Average, Histogram
Channel Average Histogram
------- ------- ---------
    Red    0.00 {16}
  Green    0.00 {4}
   Blue    0.00 {4}
```

## Process an image on the GPU

A compute shader reads one texture and writes another, a storage texture, one invocation per texel. This one inverts the colors of the image loaded above:

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
PS> $inverted = New-WgpuTexture $image.Width $image.Height -Usage StorageBinding, CopySrc
PS> Invoke-WgpuCompute (Get-Content ./invert.wgsl -Raw) @{ source = $image; inverted = $inverted } -Invocations $image.Width, $image.Height
PS> (Read-WgpuTexture $inverted) -join ','
0,255,255,255,255,0,255,255,255,255,0,255,0,0,0,255
```

## Sample a texture

A sampler reads a texture between its texels, filtered and wrapped as it says. The default sampler takes the nearest texel and clamps to the edge; `-Filter Linear` blends the texels around the point, `-AddressMode Repeat` wraps past the edges, `-Anisotropy` filters at a slant and needs every filter linear, and `-Compare` makes a comparison sampler for depth:

```powershell
PS> New-WgpuSampler
Label        MagFilter MinFilter AddressModeU Compare
-----        --------- --------- ------------ -------
Wgpu sampler Nearest   Nearest   ClampToEdge

PS> New-WgpuSampler -Filter Linear -AddressMode Repeat -Anisotropy 8 | Select-Object MagFilter, MinFilter, AddressModeU, Anisotropy
MagFilter MinFilter AddressModeU Anisotropy
--------- --------- ------------ ----------
   Linear    Linear       Repeat          8

PS> (New-WgpuSampler -Compare LessEqual).Compare
LessEqual

PS> New-WgpuSampler -Anisotropy 8
New-WgpuSampler: the sampler cannot be created: Validation Error

Caused by:
  In Device::create_sampler, label = 'Wgpu sampler'
    Invalid filter mode for minFilter: Nearest. When anistropic clamp is not 1 (it is 8), all filter modes must be linear.
```

A shader that samples the red of a texture two texels wide at five points across it: 0, a quarter, a half, three quarters and the right edge. The texel centers are at a quarter and three quarters:

```powershell
PS> Get-Content ./sample.wgsl
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<storage, read_write> red: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    red[id.x] = textureSampleLevel(source, smp, vec2<f32>(f32(id.x) / 4.0, 0.5), 0.0).r;
}
PS> $ramp = New-WgpuTexture 2 1 -Format Rgba32Float -Data ([float[]](0, 0, 0, 1, 1, 0, 0, 1))
PS> $sample = Get-Content ./sample.wgsl -Raw
PS> Invoke-WgpuCompute $sample @{ source = $ramp; smp = (New-WgpuSampler -Filter Nearest); red = [float[]]::new(5) } -Workgroups 5
0
0
1
1
1

PS> Invoke-WgpuCompute $sample @{ source = $ramp; smp = (New-WgpuSampler -Filter Linear); red = [float[]]::new(5) } -Workgroups 5
0
0
0.5
1
1
```

Nearest changes texel at the half, where linear blends the two evenly, and past the centers both clamp to the edge texel. Rgba32Float can be filtered only on a device with the Float32Filterable feature, which a device has whenever its adapter does.

## Bind a whole texture

A texture given whole to a shader binds through the view its binding reads: the binding's dimension, over every layer and mip level the texture has, and of a format that holds depth and stencil, the depth for a depth or float binding and the stencil for a `u32` one. So a cube map binds to a `texture_cube` binding, and to a `texture_2d_array` binding as six layers, and six square layers bind as a cube whether or not the texture was made as one. This shader reads the middle of each face of the red cube made above:

```powershell
PS> Get-Content ./faces.wgsl
@group(0) @binding(0) var faces: texture_cube<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<storage, read_write> red: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    var axes = array<vec3<f32>, 6>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, -1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0));
    red[id.x] = textureSampleLevel(faces, smp, axes[id.x], 0.0).r;
}
PS> $byFace = Get-Content ./faces.wgsl -Raw
PS> $red = Invoke-WgpuCompute $byFace @{ faces = $cube; smp = (New-WgpuSampler); red = [float[]]::new(6) } -Workgroups 6
PS> ($red | ForEach-Object { [math]::Round($_ * 255) }) -join ','
0,40,80,120,160,200
```

A binding that cannot read the texture whole is refused, and the message names `CreateView`, which binds the part to read: twelve layers given to a `texture_cube` binding, six to a `texture_2d` binding, or a texture with mip levels to a storage binding, which writes one level. A view binds as it was made:

```powershell
PS> $two = New-WgpuTexture 2 2 -Layers 12 -Cube
PS> foreach ($layer in 0..11) { Write-WgpuTexture $two ([byte[]](@(($layer * 20), 0, 0, 255) * 4)) -Layer $layer }
PS> Invoke-WgpuCompute $byFace @{ faces = $two; smp = (New-WgpuSampler); red = [float[]]::new(6) } -Workgroups 6
Invoke-WgpuCompute: faces is a texture_cube<f32> binding, which reads six layers, and the texture has 12; bind six through CreateView

PS> $second = $two.CreateView($null, 'Cube', $null, $null, 6, 6)
PS> $red = Invoke-WgpuCompute $byFace @{ faces = $second; smp = (New-WgpuSampler); red = [float[]]::new(6) } -Workgroups 6
PS> ($red | ForEach-Object { [math]::Round($_ * 255) }) -join ','
120,140,160,180,200,220
```

## Bind part of a texture

A view binds one mip level or a range of layers in place of the whole texture, as another dimension, or as another format the texture was made to allow with `-ViewFormat`. A cube map's view with no dimension given is a cube over its whole cubes, so `$cube.CreateView()` binds to a `texture_cube` binding as the texture does:

```powershell
PS> $mipped.CreateView($null, $null, 1, 1)
Label                         Format
-----                         ------
C:\Temp\wgpu-demo\columns.png Rgba8Unorm

PS> (New-WgpuTexture 2 2 -ViewFormat Rgba8UnormSrgb).CreateView('Rgba8UnormSrgb').Format
Rgba8UnormSrgb

PS> $red = Invoke-WgpuCompute $byFace @{ faces = $cube.CreateView(); smp = (New-WgpuSampler); red = [float[]]::new(6) } -Workgroups 6
PS> ($red | ForEach-Object { [math]::Round($_ * 255) }) -join ','
0,40,80,120,160,200
```

## Read and write depth and stencil

A depth or stencil texture is read and written one aspect at a time, the one `-Aspect` names, and only where WebGPU gives the aspect a layout outside the GPU. The depth of Depth16Unorm goes both ways, as 16-bit integers with 65535 for 1. The depth of Depth32Float and Depth32FloatStencil8 can be read but not written. The stencil of every format that has one goes both ways, as bytes, and the depth of Depth24Plus and Depth24PlusStencil8 never leaves the GPU:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $depth = New-WgpuTexture 2 2 -Format Depth16Unorm
PS> Write-WgpuTexture $depth ([uint16[]](0, 16384, 32768, 65535)) -Aspect DepthOnly
PS> (Read-WgpuTexture $depth -Aspect DepthOnly) -join ','
0,16384,32768,65535

PS> $both = New-WgpuTexture 2 2 -Format Depth24PlusStencil8
PS> Write-WgpuTexture $both ([byte[]](1, 2, 3, 4)) -Aspect StencilOnly
PS> (Read-WgpuTexture $both -Aspect StencilOnly) -join ','
1,2,3,4

PS> Read-WgpuTexture $both -Aspect DepthOnly
Read-WgpuTexture: only the stencil of Depth24PlusStencil8 can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $depth = New-WgpuTexture 2 2 -Format Depth16Unorm
PS> $depth.Write([uint16[]](0, 16384, 32768, 65535), 0, 0, 'DepthOnly')
PS> $depth.Read(0, 0, 'DepthOnly') -join ','
0,16384,32768,65535

PS> $both = New-WgpuTexture 2 2 -Format Depth24PlusStencil8
PS> $both.Write([byte[]](1, 2, 3, 4), 0, 0, 'StencilOnly')
PS> $both.Read(0, 0, 'StencilOnly') -join ','
1,2,3,4

PS> $both.Read(0, 0, 'DepthOnly')
MethodInvocationException: Exception calling "Read" with "3" argument(s): "[WgpuAspect] only the stencil of Depth24PlusStencil8 can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU"
```
{{< /tab >}}
{{< /tabs >}}

Given whole to a shader, a texture that holds both binds the aspect its binding reads: the stencil to a `texture_2d<u32>` binding, as here, and the depth to a `texture_depth_2d` binding or a float one:

```powershell
PS> Get-Content ./marks.wgsl
@group(0) @binding(0) var marks: texture_2d<u32>;
@group(0) @binding(1) var<storage, read_write> values: array<u32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    values[id.x] = textureLoad(marks, vec2<i32>(i32(id.x % 2u), i32(id.x / 2u)), 0).r;
}
PS> (Invoke-WgpuCompute (Get-Content ./marks.wgsl -Raw) @{ marks = $both; values = [uint32[]]::new(4) } -Workgroups 4) -join ','
1,2,3,4
```

A render pass writes depth, and stencil by a stencil test, as [Rendering](Rendering.md#mask-with-a-stencil-test) shows. A depth or stencil format is never a storage binding, so a depth texture asked to be one is refused:

```powershell
PS> New-WgpuTexture 2 2 -Format Depth32Float -Usage StorageBinding
New-WgpuTexture: Depth32Float is a depth or stencil format, which cannot be a storage binding
```

## Read and write the planes of NV12 and P010

An NV12 or P010 texture keeps its texels in two planes, read and written one at a time, the one `-Aspect` names. `Plane0` is the luma, one value a texel at the texture's size, and `Plane1` the chroma, two values a texel at half its width and height. NV12's values are bytes, and P010's are 16-bit values of which the high 10 bits are used:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $frame = New-WgpuTexture 4 4 -Format NV12
PS> Write-WgpuTexture $frame ([byte[]](0..15 | ForEach-Object { $_ * 16 })) -Aspect Plane0
PS> Write-WgpuTexture $frame ([byte[]]((128, 64) * 4)) -Aspect Plane1
PS> (Read-WgpuTexture $frame -Aspect Plane1) -join ','
128,64,128,64,128,64,128,64

PS> Read-WgpuTexture $frame
Read-WgpuTexture: NV12 keeps its texels in planes: give -Aspect Plane0 for its luma or Plane1 for its chroma
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $frame = New-WgpuTexture 4 4 -Format NV12
PS> $frame.Write([byte[]](0..15 | ForEach-Object { $_ * 16 }), 0, 0, 'Plane0')
PS> $frame.Write([byte[]]((128, 64) * 4), 0, 0, 'Plane1')
PS> $frame.Read(0, 0, 'Plane1') -join ','
128,64,128,64,128,64,128,64

PS> $frame.Read()
MethodInvocationException: Exception calling "Read" with "0" argument(s): "[WgpuAspect] NV12 keeps its texels in planes: give -Aspect Plane0 for its luma or Plane1 for its chroma"
```
{{< /tab >}}
{{< /tabs >}}

A view of a plane binds it in the format of its values, R8Unorm or Rg8Unorm, or R16Unorm or Rg16Unorm for P010, so a shader reads the luma and the chroma as two textures, each chroma texel shared by 2 by 2 texels of the luma:

```powershell
PS> Get-Content ./planes.wgsl
@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> values: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let at = vec2<i32>(i32(id.x % 4u), i32(id.x / 4u));
    let uv = textureLoad(chroma, at / 2, 0).rg;
    values[id.x * 3u] = textureLoad(luma, at, 0).r;
    values[id.x * 3u + 1u] = uv.x;
    values[id.x * 3u + 2u] = uv.y;
}
PS> $luma = $frame.CreateView($null, $null, $null, $null, $null, $null, 'Plane0')
PS> $chroma = $frame.CreateView($null, $null, $null, $null, $null, $null, 'Plane1')
PS> $chroma.Format
Rg8Unorm

PS> $values = Invoke-WgpuCompute (Get-Content ./planes.wgsl -Raw) @{ luma = $luma; chroma = $chroma; values = [float[]]::new(48) } -Workgroups 16
PS> ($values[0..5] | ForEach-Object { [math]::Round($_ * 255) }) -join ','
0,128,64,16,128,64
```

Direct3D 12 and Vulkan allow these formats one mip level and two dimensions, so a texture of more levels or of three dimensions is refused, though it can have layers:

```powershell
PS> New-WgpuTexture 16 16 -Format NV12 -MipLevels 2
New-WgpuTexture: the texture cannot be created: Validation Error

Caused by:
  In Device::create_texture, label = '16x16 NV12'
    Multi-planar texture (NV12) mip level count (2) must be 1
```

## Free a texture

A texture's GPU memory is freed when .NET collects the object, or at once with the `Destroy` method, after which every use of it fails:

```powershell
PS> $spent = New-WgpuTexture 2 2 -Label spent
PS> $spent.Destroy()
PS> $spent.Read()
MethodInvocationException: Exception calling "Read" with "0" argument(s): "[WgpuSubmit] the submission is invalid: Validation Error

Caused by:
  In Queue::submit
    Texture with 'spent' label has been destroyed
"
```
