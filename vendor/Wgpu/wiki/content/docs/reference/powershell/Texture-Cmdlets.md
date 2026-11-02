---
title: Textures and images
weight: 50
---

The syntax, the parameters and the errors of `New-WgpuTexture`, `Write-WgpuTexture`, `Read-WgpuTexture`, `Import-WgpuImage`, `Export-WgpuImage`, `Resize-WgpuTexture`, `Convert-WgpuTexture`, `ConvertTo-WgpuNormalMap`, `ConvertTo-WgpuCubeMap`, `ConvertFrom-WgpuCubeMap`, `ConvertTo-WgpuIrradianceMap`, `ConvertTo-WgpuSpecularMap`, `New-WgpuBrdfTable`, `Join-WgpuTexture`, `Compare-WgpuTexture`, `Measure-WgpuTexture`, `Get-WgpuTextureFormat` and `New-WgpuSampler`. The examples are on [Images](Images.md).

### New-WgpuTexture

Creates a texture: 1D, 2D (with array layers) or 3D, of any format.

Without -Usage the texture takes every usage its format allows on the adapter, of copy, sampling, storage and render attachment. A depth or stencil format is never a storage binding. -Data fills mip level 0 of layer 0, converted to the format's number type.

Alias: `New-GpuTexture`

```powershell
New-WgpuTexture [-Width] <uint> [[-Height] <uint>] [-Depth <uint>] [-Format <string>] [-MipLevels <uint>] [-SampleCount <uint>] [-Dimension <TextureDimension>] [-Usage <TextureUsage[]>] [-ViewFormat <string[]>] [-Data <Object>] [-Cube] [-Label <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Cube` | switch |  | Record it as a cube map: square 2D layers, six to a cube, so -Height equal to -Width and -Depth 6, or a multiple of 6 for an array of cubes. |
| `-Data` | object |  | Texels for mip level 0 of layer 0, rows packed. |
| `-Depth` | uint |  | Depth of a 3D texture, or array layers of a 2D one; 1 when omitted. Alias: `-Layers`. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to create it on; the default device when omitted. |
| `-Dimension` | [Wgpu.TextureDimension](Types.md#wgputexturedimension) |  | D1, D2 or D3; D2 when omitted. |
| `-Format` | string |  | The format's name; Rgba8Unorm when omitted. |
| `-Height` | uint |  | Height in texels; 1 when omitted. |
| `-Label` | string |  | The texture's label. |
| `-MipLevels` | uint |  | Mip levels; 1 when omitted. |
| `-SampleCount` | uint |  | Samples per texel; 1 when omitted. |
| `-Usage` | [Wgpu.TextureUsage](Types.md#wgputextureusage)[] |  | What it may be used for; every usage the format allows when omitted. |
| `-ViewFormat` | string[] |  | Other formats views of it may take. |
| `-Width` | uint |  | Width in texels. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuUnknownFormat` and `WgpuTexture`; with `-Data`, `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuAspect`, `WgpuTexelCount` and `WgpuTextureWrite`; and without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a value that does not convert to a number.

### Write-WgpuTexture

Writes texels into a texture, rows packed, converted to the format's number type.

Alias: `Write-GpuTexture`

```powershell
Write-WgpuTexture [-Texture] <Texture> [-Data] <Object> [-MipLevel <uint>] [-Layer <uint>] [-Aspect <TextureAspect>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Aspect` | [Wgpu.TextureAspect](Types.md#wgputextureaspect) |  | The aspect of a depth-stencil texture, or the plane of an NV12 or P010 texture. |
| `-Data` | object |  | The texels, rows packed. |
| `-Layer` | uint |  | The array layer; 0 when omitted. |
| `-MipLevel` | uint |  | The mip level; 0 when omitted. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) |  | The texture to write. |

Writes nothing to the pipeline.

#### Errors

`WgpuNoTexture`, `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuAspect`, `WgpuTexelCount` and `WgpuTextureWrite`, each described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a value that does not convert to a number.

### Read-WgpuTexture

Copies a texture's texels back from the GPU and writes them as one array, rows packed, as the format's number type.

Alias: `Read-GpuTexture`

```powershell
Read-WgpuTexture [-Texture] <Texture> [-MipLevel <uint>] [-Layer <uint>] [-Aspect <TextureAspect>] [-AsBytes] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AsBytes` | switch |  | Write the texels as byte[] instead of the format's number type. |
| `-Aspect` | [Wgpu.TextureAspect](Types.md#wgputextureaspect) |  | The aspect of a depth-stencil texture, or the plane of an NV12 or P010 texture. |
| `-Layer` | uint |  | The array layer; 0 when omitted. |
| `-MipLevel` | uint |  | The mip level; 0 when omitted. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The texture to read. |

Writes `array`.

#### Errors

`WgpuNoTexture`, `WgpuAspect`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`, each described in the [Error Reference](Error-Reference.md).

### Import-WgpuImage

Loads an image file into a texture: PNG, JPEG, GIF, WebP, AVIF, JPEG XL, TIFF, Photoshop's PSD and PSB, BMP, TGA, EXR, HDR, QOI, ICO, PNM or Farbfeld, and a DDS or KTX2 file whole, with every mip level, array layer and cube face; and draws an SVG file or a PDF's first page.

An SVG file is drawn at the size it gives, in pixels of a 96th of an inch, and a PDF's page at 72 dots per inch, a pixel a point, or at the -Dpi given, on white unless -NoBackground leaves it transparent; -Password opens a protected PDF. -Width, -Height, -Scale, -MaxSize and -PowerOfTwo set the size either is drawn at, and -Crop, -CropMargin, -Rotate and -Flip are drawn too, so the drawing is never resampled. An SVG's text is drawn in the machine's fonts; text in a font-family the machine has no font of is drawn in its default family, which a warning says.

An image file's pixels are turned as its orientation says: an EXIF orientation, a PNG's in the text chunk ImageMagick writes it in included, an AVIF's rotation and mirror, or a JPEG XL's own. Its colors are converted to sRGB from the color profile it carries, an ICC profile or the CICP values of an AVIF or an HDR JPEG XL, as littleCMS converts with its optimizations off: perceptually, or with the -RenderingIntent named, with colors sRGB lacks clipped. A float image's colors are converted to linear light in sRGB's primaries instead, light below 0 clipped and light above 1 kept, and a PQ or HLG AVIF or JPEG XL loads as linear light in sRGB's primaries too, with BT.2408's HDR reference white of 203 nits at 1. An orientation or profile the module cannot use leaves the file as stored, which a warning says. -IgnoreOrientation loads the pixels as stored, and -IgnoreProfile the colors.

Without -Format a float image loads as Rgba32Float, a PQ or HLG AVIF or JPEG XL as Rgba16Float, a 16-bit image as Rgba16Unorm, and the rest as Rgba8Unorm, or Rgba8UnormSrgb with -Srgb. A DDS or KTX2 file keeps its format; a block-compressed one the device cannot hold is decoded on the CPU, which a warning says. A KTX2 file in a Basis Universal format is transcoded to the best format the device holds.

-Format converts the texture: into a color format, a BC format encoded on the GPU, or a Basis Universal format, which Basis Universal encodes and the module transcodes. An image file's 8-bit color is taken as already in the -Format's encoding, and a two-channel -Format takes red and green, or a gray image's gray and alpha. -MipLevels gives it mip levels, rendered on the GPU: each from the level above by the Box filter, or as -MipFilter and -MipEdge say. -Blur and -Sharpen filter the texels after the size, orientation and texel changes, as Convert-WgpuTexture does. -HeightToNormal makes the heights into normals after those, as ConvertTo-WgpuNormalMap does, and -NormalMips keeps normals unit length in the mip levels. -Cube makes a cube map after those changes and before -Format and -MipLevels: from a panorama or a layout of the faces, read by its shape, -Layout, -Yaw and -FaceSize as ConvertTo-WgpuCubeMap reads them, or from a DDS or KTX2 file's square layers, six to a cube, recorded as its faces. What the import involved goes to the verbose stream and the texture's Notes.

Alias: `Import-GpuImage`

```powershell
Import-WgpuImage [-Path] <string> [-Format <string>] [-MipLevels <uint>] [-MipFilter <MipFilter>] [-MipEdge <MipEdge>] [-KeepCoverage <double>] [-NormalMips <NormalMips>] [-Quality <uint>] [-Effort <uint>] [-Srgb] [-IgnoreOrientation] [-IgnoreProfile] [-RenderingIntent <RenderingIntent>] [-Dpi <double>] [-NoBackground] [-Password <securestring>] [-Usage <TextureUsage[]>] [-Label <string>] [-Device <Device>] [-Width <uint>] [-Height <uint>] [-Scale <double>] [-MaxSize <uint>] [-PowerOfTwo <PowerOfTwo>] [-Filter <ResizeFilter>] [-Flip <FlipDirection>] [-Rotate <uint>] [-Crop <uint[]>] [-CropMargin <uint[]>] [-Pad <uint[]>] [-Anchor <Anchor>] [-PadColor <double[]>] [-Swizzle <string>] [-Premultiply] [-Unpremultiply] [-ColorSpace <ColorSpace>] [-AssignColorSpace <ColorSpace>] [-Grayscale] [-GrayscaleWeights <Object>] [-Invert] [-Brightness <double>] [-Contrast <double>] [-Gamma <double>] [-Exposure <double>] [-ToneMap <ToneMapCurve>] [-ColorKey <Object>] [-ColorKeyTolerance <double>] [-AlphaThreshold <double>] [-Blur <double[]>] [-Sharpen <double>] [-SharpenRadius <double>] [-SharpenThreshold <double>] [-FilterEdge <FilterEdge>] [-FixAlphaBorder] [-BorderReach <uint>] [-ReconstructZ] [-Normalize] [-FlipGreen] [-HeightToNormal <double>] [-HeightChannel <HeightChannel>] [-NormalKernel <NormalKernel>] [-HeightEdge <HeightEdge>] [-NormalAlpha <NormalAlpha>] [-TwoChannel] [-Cube] [-Layout <CubeLayout>] [-Yaw <double>] [-FaceSize <uint>] [<CommonParameters>]
Import-WgpuImage -LiteralPath <string> [-Format <string>] [-MipLevels <uint>] [-MipFilter <MipFilter>] [-MipEdge <MipEdge>] [-KeepCoverage <double>] [-NormalMips <NormalMips>] [-Quality <uint>] [-Effort <uint>] [-Srgb] [-IgnoreOrientation] [-IgnoreProfile] [-RenderingIntent <RenderingIntent>] [-Dpi <double>] [-NoBackground] [-Password <securestring>] [-Usage <TextureUsage[]>] [-Label <string>] [-Device <Device>] [-Width <uint>] [-Height <uint>] [-Scale <double>] [-MaxSize <uint>] [-PowerOfTwo <PowerOfTwo>] [-Filter <ResizeFilter>] [-Flip <FlipDirection>] [-Rotate <uint>] [-Crop <uint[]>] [-CropMargin <uint[]>] [-Pad <uint[]>] [-Anchor <Anchor>] [-PadColor <double[]>] [-Swizzle <string>] [-Premultiply] [-Unpremultiply] [-ColorSpace <ColorSpace>] [-AssignColorSpace <ColorSpace>] [-Grayscale] [-GrayscaleWeights <Object>] [-Invert] [-Brightness <double>] [-Contrast <double>] [-Gamma <double>] [-Exposure <double>] [-ToneMap <ToneMapCurve>] [-ColorKey <Object>] [-ColorKeyTolerance <double>] [-AlphaThreshold <double>] [-Blur <double[]>] [-Sharpen <double>] [-SharpenRadius <double>] [-SharpenThreshold <double>] [-FilterEdge <FilterEdge>] [-FixAlphaBorder] [-BorderReach <uint>] [-ReconstructZ] [-Normalize] [-FlipGreen] [-HeightToNormal <double>] [-HeightChannel <HeightChannel>] [-NormalKernel <NormalKernel>] [-HeightEdge <HeightEdge>] [-NormalAlpha <NormalAlpha>] [-TwoChannel] [-Cube] [-Layout <CubeLayout>] [-Yaw <double>] [-FaceSize <uint>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AlphaThreshold` | double |  | Gives alpha 0 to texels whose alpha is below this, and 1 to the rest, as an alpha test would: above 0 and at most 1. |
| `-Anchor` | [Wgpu.Anchor](Types.md#wgpuanchor) |  | Where a crop to a size or the padding places the texture; Center when omitted. |
| `-AssignColorSpace` | [Wgpu.ColorSpace](Types.md#wgpucolorspace) |  | Relabels the texels as Linear or Srgb without changing them. |
| `-Blur` | double[] |  | Blurs the texels by a Gaussian of this standard deviation in texels, or of two, across and down, in linear values weighed by alpha. |
| `-BorderReach` | uint |  | How far -FixAlphaBorder carries color, in texels; every texel with alpha 0 is colored when omitted. |
| `-Brightness` | double |  | Brightens or darkens the stored values, from -100 to 100, as ImageMagick's -brightness-contrast. |
| `-ColorKey` | object |  | Gives alpha 0 to the texels of this color, as the texture stores it: red, green and blue from 0 to 1, rounded to the texture's own precision, or '#RRGGBB'. Other texels keep their alpha. |
| `-ColorKeyTolerance` | double |  | How far each of red, green and blue may be from -ColorKey's, from 0 to 1; 0, the same stored value, when omitted. |
| `-ColorSpace` | [Wgpu.ColorSpace](Types.md#wgpucolorspace) |  | Converts the texels to Linear or Srgb so their colors look the same. |
| `-Contrast` | double |  | Raises or lowers the stored values' contrast, from -100 to 100, as ImageMagick's -brightness-contrast. |
| `-Crop` | uint[] |  | Keeps a region, four numbers for its left, top, width and height, or a size -Anchor places, two numbers. |
| `-CropMargin` | uint[] |  | Cuts this many texels from the left, top, right and bottom. |
| `-Cube` | switch |  | Makes a cube map: from a panorama or a layout of the faces, read by its shape as ConvertTo-WgpuCubeMap reads it, or from a DDS or KTX2 file's square layers, six to a cube. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to load it onto; the default device when omitted. |
| `-Dpi` | double |  | The resolution a PDF's page is drawn at, in dots per inch: 72, a pixel a point, when omitted. |
| `-Effort` | uint |  | Encoding effort, 0 to 10: Basis Universal's for a Basis format, and the preset of Bc7RgbaUnorm, Bc7RgbaUnormSrgb and Bc6hRgbUfloat; the encoder's default when omitted. |
| `-Exposure` | double |  | Multiplies linear values by 2 to this power: stops of exposure, applied before -ToneMap. |
| `-FaceSize` | uint |  | The faces' size in texels for a cube -Cube resamples from a panorama, an angular map or an octahedral map; a quarter of a panorama's or an angular map's width, and half an octahedral map's, when omitted. |
| `-Filter` | [Wgpu.ResizeFilter](Types.md#wgpuresizefilter) |  | The resize filter: Box where a side shrinks and Bicubic where it grows when omitted. |
| `-FilterEdge` | [Wgpu.FilterEdge](Types.md#wgpufilteredge) |  | What -Blur and -Sharpen read past the edge: Clamp, the edge texel repeated, the default; Wrap, the other side, for an image that tiles; or Mirror. |
| `-FixAlphaBorder` | switch |  | Gives each texel with alpha 0 the color of the nearest texel with alpha above 0, its alpha left at 0, so filtering and mip levels blend no dark or keyed color in at a cutout's edge. |
| `-Flip` | [Wgpu.FlipDirection](Types.md#wgpuflipdirection) |  | Mirrors the texture: Horizontal or Vertical. |
| `-FlipGreen` | switch |  | Flips each normal's green between the convention glTF reads, up, and DirectX's, down; with -HeightToNormal, points green down in the normals it makes. |
| `-Format` | string |  | The format to load the texture in: a color format, a BC format, encoded on the GPU, or a Basis Universal format such as BasisEtc1s or BasisUastc4x4, which Basis Universal encodes. |
| `-Gamma` | double |  | Gamma corrects the stored values to value^(1/gamma), as ImageMagick's -gamma. |
| `-Grayscale` | switch |  | Makes the texture grayscale, with Rec. 709's weights in linear light or those -GrayscaleWeights names. |
| `-GrayscaleWeights` | object |  | The weights -Grayscale uses: Rec709, Rec601 or three numbers for red, green and blue. |
| `-Height` | uint |  | Resizes the texture to this height; without -Width, the width keeps the aspect ratio. |
| `-HeightChannel` | [Wgpu.HeightChannel](Types.md#wgpuheightchannel) |  | Where -HeightToNormal reads the heights: Red when omitted, Green, Blue, Alpha or Luminance. |
| `-HeightEdge` | [Wgpu.HeightEdge](Types.md#wgpuheightedge) |  | What -HeightToNormal reads past the edges: Continue, the slope at the edge carried on, when omitted; Wrap, Clamp or Mirror. |
| `-HeightToNormal` | double |  | Makes the heights into normals, after the size, orientation and texel changes: a height step of 1 between neighboring texels gives a slope of this. |
| `-IgnoreOrientation` | switch |  | Loads an image file's pixels as the file stores them, not turned as its orientation says. |
| `-IgnoreProfile` | switch |  | Loads an image file's colors as the file stores them, not converted from its color profile. |
| `-Invert` | switch |  | Turns red, green and blue into 1 minus their stored values; alpha is kept. |
| `-KeepCoverage` | double |  | Scales the alpha of each mip level -MipLevels adds, or a transform renders again, so the share of its texels with alpha at or above this, the alpha an alpha test passes at, stays level 0's: above 0 and at most 1. |
| `-Label` | string |  | The texture's label; the file's path when omitted. |
| `-Layout` | [Wgpu.CubeLayout](Types.md#wgpucubelayout) |  | How the image -Cube reads holds the faces: needed for a HorizontalTee, a VerticalCrossTurned, a Grid2x3, a Grid3x2, an AngularMap or an Octahedral map, which the image's shape does not tell apart. |
| `-LiteralPath` | string | by name | The image, DDS or KTX2 file, named exactly as written, wildcard characters included; the files Get-ChildItem pipes bind here. Alias: `-PSPath`, `-LP`, `-FullName`. |
| `-MaxSize` | uint |  | Shrinks a texture with a side past this many texels to it, keeping the aspect ratio. |
| `-MipEdge` | [Wgpu.MipEdge](Types.md#wgpumipedge) |  | What Triangle, Kaiser, Mitchell and Lanczos read past an edge of the image: Clamp, the default, nothing; Wrap, the other side, for a texture that tiles; or Mirror, the image mirrored. A sampler's ClampToEdge, Repeat and MirrorRepeat name the same. |
| `-MipFilter` | [Wgpu.MipFilter](Types.md#wgpumipfilter) |  | How each mip level -MipLevels adds, or a transform renders again, is made: Box, the default, from the level above; Triangle, Kaiser, Mitchell or Lanczos from level 0; Min or Max, the smallest or largest value of each channel among the texels it covers. |
| `-MipLevels` | uint |  | Mip levels: those the file has when omitted, and 0 for the full chain down to 1x1. |
| `-NoBackground` | switch |  | Draws a PDF's page with no background, transparent where nothing is drawn, not on white. |
| `-NormalAlpha` | [Wgpu.NormalAlpha](Types.md#wgpunormalalpha) |  | What alpha holds in the normals -HeightToNormal makes: One when omitted, Height or Occlusion. |
| `-Normalize` | switch |  | Scales each normal to length 1. |
| `-NormalKernel` | [Wgpu.NormalKernel](Types.md#wgpunormalkernel) |  | The kernel -HeightToNormal takes slopes by: Prewitt, texconv's, when omitted, Central, Sobel or Sobel5x5. |
| `-NormalMips` | [Wgpu.NormalMips](Types.md#wgpunormalmips) |  | Keeps a normal map's normals unit length in each mip level -MipLevels adds, or a transform renders again: Renormalize scales each texel's normal back to length 1, and SlopeSpace filters the normals' slopes; Renormalize with -HeightToNormal when omitted. It marks the texels as normals, which a -Format that stores them the other way takes written again: n / 2 + 1 / 2 in a format that holds no negative values, and n in any other. |
| `-Pad` | uint[] |  | Places the texture on a larger canvas of this width and height. |
| `-PadColor` | double[] |  | The color around the padded texture: red, green, blue and alpha from 0 to 1; transparent black when omitted. |
| `-Password` | SecureString |  | The password a protected PDF opens with. |
| `-Path` | string | by name | The image, DDS or KTX2 file; a wildcard loads every file it matches. |
| `-PowerOfTwo` | [Wgpu.PowerOfTwo](Types.md#wgpupoweroftwo) |  | Rounds each side to a power of two: Nearest, Up or Down. |
| `-Premultiply` | switch |  | Multiplies red, green and blue by alpha, in linear light for an sRGB texture. |
| `-Quality` | uint |  | Basis Universal's quality, 1 to 100, for a Basis format that has one; Basis Universal's default when omitted. |
| `-ReconstructZ` | switch |  | Rebuilds each normal's Z, in blue, from its X and Y, in red and green; the -Format must have blue. |
| `-RenderingIntent` | [Wgpu.RenderingIntent](Types.md#wgpurenderingintent) |  | How a conversion from an image file's color profile maps colors, above all those sRGB lacks: Perceptual when omitted, RelativeColorimetric, Saturation or AbsoluteColorimetric. |
| `-Rotate` | uint |  | Turns the texture clockwise by 90, 180 or 270 degrees. |
| `-Scale` | double |  | Resizes both sides by this factor, such as 0.5 for half the size. |
| `-Sharpen` | double |  | Sharpens the texels by an unsharp mask: this many times the difference between each texel and a Gaussian blur of it is added back, in linear values weighed by alpha. |
| `-SharpenRadius` | double |  | The standard deviation, in texels, of the blur -Sharpen compares each texel with; 1 when omitted. |
| `-SharpenThreshold` | double |  | The smallest difference from the blur, from 0 to 1, that -Sharpen adds back in a channel; 0 when omitted. |
| `-Srgb` | switch |  | Read 8-bit color as sRGB: an image file's without a GPU -Format, and a DDS or KTX2 file's where its format has an sRGB form. |
| `-Swizzle` | string |  | Rearranges the channels: four letters of r, g, b, a, 0 and 1, one for each channel, as bgra or rrr1. |
| `-ToneMap` | [Wgpu.ToneMapCurve](Types.md#wgputonemapcurve) |  | Maps HDR colors into the range from 0 to 1: Reinhard, AcesFilmic, AgX or Neutral. |
| `-TwoChannel` | switch |  | Keeps normals' X and Y alone where -Format keeps them apart: in red and green for a format with two channels, such as Bc5RgUnorm, and X in red, green and blue with Y in alpha for a Basis Universal LDR format. |
| `-Unpremultiply` | switch |  | Divides red, green and blue by alpha where alpha is above 0, in linear light for an sRGB texture. |
| `-Usage` | [Wgpu.TextureUsage](Types.md#wgputextureusage)[] |  | What the texture may be used for; every usage the format allows when omitted. |
| `-Width` | uint |  | Resizes the texture to this width before -Format converts it; without -Height, the height keeps the aspect ratio. |
| `-Yaw` | double |  | Turns a panorama, an angular map or an octahedral map -Cube reads about the vertical axis by this many degrees, +X toward -Z. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuImage`, `WgpuSrgb`, `WgpuUnknownFormat`, `WgpuQuality`, `WgpuEffort`, `WgpuImageFormat`, `WgpuTextureFile` and `WgpuKtx2`; with a new size or orientation, `WgpuResize`, `WgpuCrop`, `WgpuPad`, `WgpuRotate` and `WgpuTransform`; for `-Blur`, `-Sharpen` and their settings, `WgpuBlur`, `WgpuSharpen` and `WgpuFilterEdge`, and `WgpuFilter`, from the passes that filter the texels; for `-HeightToNormal`, the normal steps, `-NormalMips` and `-TwoChannel`, `WgpuNormalMap`; for `-Cube`, `-Layout`, `-Yaw` and `-FaceSize`, `WgpuCubeMap`, which the passes that resample a projection raise too, and `WgpuCube`, for layers that are not square or not six to a cube; from converting, decoding and encoding its texels, `WgpuTextureFormat`, `WgpuTextureDecode`, `WgpuConvert`, `WgpuMipLevels`, `WgpuCompress`, `WgpuCompressFormat` and `WgpuBasisEncode`, and from the GPU work these take, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; from making the texture, `WgpuTextureUsage`, `WgpuTexture`, `WgpuTexelCount` and `WgpuTextureWrite`; and without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md).

### Export-WgpuImage

Saves a texture: as a DDS or KTX2 file whole, with every mip level, array layer and cube face, or as any other image file, from mip level 0 of one layer, in the format its extension names (or -ImageFormat), converted to what that format stores, a two-channel texture as red and green with blue 0 and alpha 1. An .exr file is compressed with ZIP, in half floats for a texture of 16-bit float texels and in full floats for any other.

For a DDS or KTX2 file, -Format converts the texture first: into a color format, a BC format encoded on the GPU, or, for KTX2, a Basis Universal format, which Basis Universal encodes. -MipLevels gives it mip levels, rendered on the GPU: each from the level above by the Box filter, or as -MipFilter and -MipEdge say. -Blur and -Sharpen filter the texels after the size, orientation and texel changes, as Convert-WgpuTexture does. -HeightToNormal makes the heights into normals after those, as ConvertTo-WgpuNormalMap does, and -NormalMips keeps normals unit length in the mip levels. -Zstd compresses a KTX2 file's levels; -Cube writes its layers as cube faces. What the export involved goes to the verbose stream.

A block-compressed texture saved as an image file is decoded first: on the CPU by the module's decoders, or with -Decoder Gpu by the GPU, as a shader reads it.

-Layout writes a cube map into an image file as ConvertFrom-WgpuCubeMap writes it, its faces in a layout of cells or an angular or octahedral map, and -Panorama as a panorama, with -Yaw and -FaceSize, before the size, orientation and texel changes; the image is in the format the cube's texels decode into.

Alias: `Export-GpuImage`

```powershell
Export-WgpuImage [-Texture] <Texture> [-Path] <string> [-ImageFormat <string>] [-Layer <uint>] [-Format <string>] [-MipLevels <uint>] [-MipFilter <MipFilter>] [-MipEdge <MipEdge>] [-KeepCoverage <double>] [-NormalMips <NormalMips>] [-Quality <uint>] [-Effort <uint>] [-Zstd] [-Cube] [-Decoder <Decoder>] [-Width <uint>] [-Height <uint>] [-Scale <double>] [-MaxSize <uint>] [-PowerOfTwo <PowerOfTwo>] [-Filter <ResizeFilter>] [-Flip <FlipDirection>] [-Rotate <uint>] [-Crop <uint[]>] [-CropMargin <uint[]>] [-Pad <uint[]>] [-Anchor <Anchor>] [-PadColor <double[]>] [-Swizzle <string>] [-Premultiply] [-Unpremultiply] [-ColorSpace <ColorSpace>] [-AssignColorSpace <ColorSpace>] [-Grayscale] [-GrayscaleWeights <Object>] [-Invert] [-Brightness <double>] [-Contrast <double>] [-Gamma <double>] [-Exposure <double>] [-ToneMap <ToneMapCurve>] [-ColorKey <Object>] [-ColorKeyTolerance <double>] [-AlphaThreshold <double>] [-Blur <double[]>] [-Sharpen <double>] [-SharpenRadius <double>] [-SharpenThreshold <double>] [-FilterEdge <FilterEdge>] [-FixAlphaBorder] [-BorderReach <uint>] [-ReconstructZ] [-Normalize] [-FlipGreen] [-HeightToNormal <double>] [-HeightChannel <HeightChannel>] [-NormalKernel <NormalKernel>] [-HeightEdge <HeightEdge>] [-NormalAlpha <NormalAlpha>] [-TwoChannel] [-Layout <CubeLayout>] [-Panorama] [-Yaw <double>] [-FaceSize <uint>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AlphaThreshold` | double |  | Gives alpha 0 to texels whose alpha is below this, and 1 to the rest, as an alpha test would: above 0 and at most 1. |
| `-Anchor` | [Wgpu.Anchor](Types.md#wgpuanchor) |  | Where a crop to a size or the padding places the texture; Center when omitted. |
| `-AssignColorSpace` | [Wgpu.ColorSpace](Types.md#wgpucolorspace) |  | Relabels the texels as Linear or Srgb without changing them. |
| `-Blur` | double[] |  | Blurs the texels by a Gaussian of this standard deviation in texels, or of two, across and down, in linear values weighed by alpha. |
| `-BorderReach` | uint |  | How far -FixAlphaBorder carries color, in texels; every texel with alpha 0 is colored when omitted. |
| `-Brightness` | double |  | Brightens or darkens the stored values, from -100 to 100, as ImageMagick's -brightness-contrast. |
| `-ColorKey` | object |  | Gives alpha 0 to the texels of this color, as the texture stores it: red, green and blue from 0 to 1, rounded to the texture's own precision, or '#RRGGBB'. Other texels keep their alpha. |
| `-ColorKeyTolerance` | double |  | How far each of red, green and blue may be from -ColorKey's, from 0 to 1; 0, the same stored value, when omitted. |
| `-ColorSpace` | [Wgpu.ColorSpace](Types.md#wgpucolorspace) |  | Converts the texels to Linear or Srgb so their colors look the same. |
| `-Contrast` | double |  | Raises or lowers the stored values' contrast, from -100 to 100, as ImageMagick's -brightness-contrast. |
| `-Crop` | uint[] |  | Keeps a region, four numbers for its left, top, width and height, or a size -Anchor places, two numbers. |
| `-CropMargin` | uint[] |  | Cuts this many texels from the left, top, right and bottom. |
| `-Cube` | switch |  | Write the texture's layers as cube faces: square 2D layers, six to a cube. A texture made as a cube map is written as one without it. |
| `-Decoder` | [Wgpu.Decoder](Types.md#wgpudecoder) |  | Where a block-compressed texture is decoded for an image file: Cpu, by the module's decoders, or Gpu, as a shader reads it; Cpu when omitted. |
| `-Effort` | uint |  | Encoding effort, 0 to 10: Basis Universal's for a Basis format, and the preset of Bc7RgbaUnorm, Bc7RgbaUnormSrgb and Bc6hRgbUfloat; the encoder's default when omitted. |
| `-Exposure` | double |  | Multiplies linear values by 2 to this power: stops of exposure, applied before -ToneMap. |
| `-FaceSize` | uint |  | The size in texels of the faces the panorama, angular map or octahedral map -Layout or -Panorama writes is made for; the cube's when omitted. |
| `-Filter` | [Wgpu.ResizeFilter](Types.md#wgpuresizefilter) |  | The resize filter: Box where a side shrinks and Bicubic where it grows when omitted. |
| `-FilterEdge` | [Wgpu.FilterEdge](Types.md#wgpufilteredge) |  | What -Blur and -Sharpen read past the edge: Clamp, the edge texel repeated, the default; Wrap, the other side, for an image that tiles; or Mirror. |
| `-FixAlphaBorder` | switch |  | Gives each texel with alpha 0 the color of the nearest texel with alpha above 0, its alpha left at 0, so filtering and mip levels blend no dark or keyed color in at a cutout's edge. |
| `-Flip` | [Wgpu.FlipDirection](Types.md#wgpuflipdirection) |  | Mirrors the texture: Horizontal or Vertical. |
| `-FlipGreen` | switch |  | Flips each normal's green between the convention glTF reads, up, and DirectX's, down; with -HeightToNormal, points green down in the normals it makes. |
| `-Format` | string |  | The format to write a DDS or KTX2 file in: a color format, a BC format, encoded on the GPU, or for KTX2 a Basis Universal format such as BasisEtc1s or BasisUastc4x4, which Basis Universal encodes. |
| `-Gamma` | double |  | Gamma corrects the stored values to value^(1/gamma), as ImageMagick's -gamma. |
| `-Grayscale` | switch |  | Makes the texture grayscale, with Rec. 709's weights in linear light or those -GrayscaleWeights names. |
| `-GrayscaleWeights` | object |  | The weights -Grayscale uses: Rec709, Rec601 or three numbers for red, green and blue. |
| `-Height` | uint |  | Resizes the texture to this height; without -Width, the width keeps the aspect ratio. |
| `-HeightChannel` | [Wgpu.HeightChannel](Types.md#wgpuheightchannel) |  | Where -HeightToNormal reads the heights: Red when omitted, Green, Blue, Alpha or Luminance. |
| `-HeightEdge` | [Wgpu.HeightEdge](Types.md#wgpuheightedge) |  | What -HeightToNormal reads past the edges: Continue, the slope at the edge carried on, when omitted; Wrap, Clamp or Mirror. |
| `-HeightToNormal` | double |  | Makes the heights into normals, after the size, orientation and texel changes: a height step of 1 between neighboring texels gives a slope of this. |
| `-ImageFormat` | string |  | The image format, such as Png or Jpeg; taken from the extension when omitted. |
| `-Invert` | switch |  | Turns red, green and blue into 1 minus their stored values; alpha is kept. |
| `-KeepCoverage` | double |  | Scales the alpha of each mip level -MipLevels adds, or a transform renders again, so the share of its texels with alpha at or above this, the alpha an alpha test passes at, stays level 0's: above 0 and at most 1. |
| `-Layer` | uint |  | The array layer to save as an image; 0 when omitted. |
| `-Layout` | [Wgpu.CubeLayout](Types.md#wgpucubelayout) |  | Writes a cube map into the image file in this layout of its faces, or as an AngularMap or an Octahedral map, as ConvertFrom-WgpuCubeMap writes it. |
| `-MaxSize` | uint |  | Shrinks a texture with a side past this many texels to it, keeping the aspect ratio. |
| `-MipEdge` | [Wgpu.MipEdge](Types.md#wgpumipedge) |  | What Triangle, Kaiser, Mitchell and Lanczos read past an edge of the image: Clamp, the default, nothing; Wrap, the other side, for a texture that tiles; or Mirror, the image mirrored. A sampler's ClampToEdge, Repeat and MirrorRepeat name the same. |
| `-MipFilter` | [Wgpu.MipFilter](Types.md#wgpumipfilter) |  | How each mip level -MipLevels adds, or a transform renders again, is made: Box, the default, from the level above; Triangle, Kaiser, Mitchell or Lanczos from level 0; Min or Max, the smallest or largest value of each channel among the texels it covers. |
| `-MipLevels` | uint |  | Mip levels to write: the texture's own when omitted, and 0 for the full chain down to 1x1. |
| `-NormalAlpha` | [Wgpu.NormalAlpha](Types.md#wgpunormalalpha) |  | What alpha holds in the normals -HeightToNormal makes: One when omitted, Height or Occlusion. |
| `-Normalize` | switch |  | Scales each normal to length 1. |
| `-NormalKernel` | [Wgpu.NormalKernel](Types.md#wgpunormalkernel) |  | The kernel -HeightToNormal takes slopes by: Prewitt, texconv's, when omitted, Central, Sobel or Sobel5x5. |
| `-NormalMips` | [Wgpu.NormalMips](Types.md#wgpunormalmips) |  | Keeps a normal map's normals unit length in each mip level -MipLevels adds, or a transform renders again: Renormalize scales each texel's normal back to length 1, and SlopeSpace filters the normals' slopes; Renormalize with -HeightToNormal when omitted. It marks the texels as normals, which a -Format that stores them the other way takes written again: n / 2 + 1 / 2 in a format that holds no negative values, and n in any other. |
| `-Pad` | uint[] |  | Places the texture on a larger canvas of this width and height. |
| `-PadColor` | double[] |  | The color around the padded texture: red, green, blue and alpha from 0 to 1; transparent black when omitted. |
| `-Panorama` | switch |  | Writes a cube map into the image file as a panorama, center +X and left quarter -Z. |
| `-Path` | string |  | The file to write. |
| `-PowerOfTwo` | [Wgpu.PowerOfTwo](Types.md#wgpupoweroftwo) |  | Rounds each side to a power of two: Nearest, Up or Down. |
| `-Premultiply` | switch |  | Multiplies red, green and blue by alpha, in linear light for an sRGB texture. |
| `-Quality` | uint |  | Basis Universal's quality, 1 to 100, for a Basis format that has one; Basis Universal's default when omitted. |
| `-ReconstructZ` | switch |  | Rebuilds each normal's Z, in blue, from its X and Y, in red and green; the -Format must have blue. |
| `-Rotate` | uint |  | Turns the texture clockwise by 90, 180 or 270 degrees. |
| `-Scale` | double |  | Resizes both sides by this factor, such as 0.5 for half the size. |
| `-Sharpen` | double |  | Sharpens the texels by an unsharp mask: this many times the difference between each texel and a Gaussian blur of it is added back, in linear values weighed by alpha. |
| `-SharpenRadius` | double |  | The standard deviation, in texels, of the blur -Sharpen compares each texel with; 1 when omitted. |
| `-SharpenThreshold` | double |  | The smallest difference from the blur, from 0 to 1, that -Sharpen adds back in a channel; 0 when omitted. |
| `-Swizzle` | string |  | Rearranges the channels: four letters of r, g, b, a, 0 and 1, one for each channel, as bgra or rrr1. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The texture to save. |
| `-ToneMap` | [Wgpu.ToneMapCurve](Types.md#wgputonemapcurve) |  | Maps HDR colors into the range from 0 to 1: Reinhard, AcesFilmic, AgX or Neutral. |
| `-TwoChannel` | switch |  | Keeps normals' X and Y alone where -Format keeps them apart: in red and green for a format with two channels, such as Bc5RgUnorm, and X in red, green and blue with Y in alpha for a Basis Universal LDR format. |
| `-Unpremultiply` | switch |  | Divides red, green and blue by alpha where alpha is above 0, in linear light for an sRGB texture. |
| `-Width` | uint |  | Resizes the texture to this width before -Format converts it; without -Height, the height keeps the aspect ratio. |
| `-Yaw` | double |  | Turns the panorama, angular map or octahedral map -Layout or -Panorama writes about the vertical axis by this many degrees, +X toward -Z, as ConvertTo-WgpuCubeMap reads it. |
| `-Zstd` | switch |  | Compress each mip level of a KTX2 file in a GPU format with Zstandard; a Basis format chooses its own supercompression. |

Writes nothing to the pipeline.

#### Errors

`WgpuNoTexture`, `WgpuImage`, `WgpuImageExtension`, `WgpuImageFormat`, `WgpuUnknownFormat`, `WgpuQuality`, `WgpuEffort`, `WgpuDecoder`, `WgpuLayer`, `WgpuZstd`, `WgpuCube`, `WgpuImageShape` and `WgpuImageWrite`; with a new size or orientation, `WgpuResize`, `WgpuCrop`, `WgpuPad`, `WgpuRotate` and `WgpuTransform`; for `-Blur`, `-Sharpen` and their settings, `WgpuBlur`, `WgpuSharpen` and `WgpuFilterEdge`, and `WgpuFilter`, from the passes that filter the texels; for `-HeightToNormal`, the normal steps, `-NormalMips` and `-TwoChannel`, `WgpuNormalMap`; for `-Layout`, `-Panorama`, `-Yaw` and `-FaceSize`, `WgpuCubeMap`, which the passes that resample a projection raise too; from reading the texture back, `WgpuAspect`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; and from converting, decoding and encoding its texels, `WgpuTextureFormat`, `WgpuTextureDecode`, `WgpuConvert`, `WgpuMipLevels`, `WgpuCompress`, `WgpuCompressFormat`, `WgpuBasisEncode`, `WgpuKtx2` and `WgpuTextureFile`. Each is described in the [Error Reference](Error-Reference.md).

### Resize-WgpuTexture

Changes a texture's size into a new texture: resizes it to a width and height, a scale, a size limit or powers of two, with a filter; crops it to a region, a size or margins; and pads it onto a larger canvas. It crops, then resizes, then pads, whatever order the parameters come in.

The new texture has the texture's format, or the -Format named, and as many of its mip levels as the new size's chain has, rendered again from level 0 as -MipFilter, -MipEdge, -KeepCoverage and -NormalMips say; the texture is left as it is. Shrinking defaults to the Box filter and enlarging to Bicubic, each side on its own. What was done goes to the verbose stream and the new texture's Notes.

Alias: `Resize-GpuTexture`

```powershell
Resize-WgpuTexture [-Texture] <Texture> [-Width <uint>] [-Height <uint>] [-Scale <double>] [-MaxSize <uint>] [-PowerOfTwo <PowerOfTwo>] [-Filter <ResizeFilter>] [-Crop <uint[]>] [-CropMargin <uint[]>] [-Pad <uint[]>] [-Anchor <Anchor>] [-PadColor <double[]>] [-MipFilter <MipFilter>] [-MipEdge <MipEdge>] [-KeepCoverage <double>] [-NormalMips <NormalMips>] [-Format <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Anchor` | [Wgpu.Anchor](Types.md#wgpuanchor) |  | Where a crop to a size, or the texture on the padded canvas, is placed: TopLeft, Top, TopRight, Left, Center, Right, BottomLeft, Bottom or BottomRight; Center when omitted. |
| `-Crop` | uint[] |  | The part to keep: four numbers, the left, top, width and height of a region, or two, a width and height -Anchor places. |
| `-CropMargin` | uint[] |  | The texels to cut from the left, top, right and bottom. |
| `-Filter` | [Wgpu.ResizeFilter](Types.md#wgpuresizefilter) |  | The filter: Nearest, Box, Bilinear, Bicubic or Lanczos; Box where a side shrinks and Bicubic where it grows when omitted. |
| `-Format` | string |  | The new texture's format: a color format, a BC format, encoded on the GPU, or a Basis Universal format; the texture's own when omitted. |
| `-Height` | uint |  | The new height; without -Width, the width keeps the aspect ratio. |
| `-KeepCoverage` | double |  | Scales the alpha of each mip level rendered again so the share of its texels with alpha at or above this, the alpha an alpha test passes at, stays level 0's: above 0 and at most 1. |
| `-MaxSize` | uint |  | The most texels either side may have: a texture past it shrinks to it, keeping its aspect ratio, and a smaller one stays as it is. |
| `-MipEdge` | [Wgpu.MipEdge](Types.md#wgpumipedge) |  | What Triangle, Kaiser, Mitchell and Lanczos read past an edge of the image: Clamp, the default; Wrap, the other side; or Mirror. |
| `-MipFilter` | [Wgpu.MipFilter](Types.md#wgpumipfilter) |  | How the mip levels rendered again are made: Box, the default, each from the level above; Triangle, Kaiser, Mitchell or Lanczos from level 0; Min or Max, the smallest or largest value of each channel among the texels each new texel covers. |
| `-NormalMips` | [Wgpu.NormalMips](Types.md#wgpunormalmips) |  | Keeps the normals of a normal map unit length in each mip level rendered again: Renormalize scales each texel's normal back to length 1, and SlopeSpace filters the normals' slopes. |
| `-Pad` | uint[] |  | The width and height of a larger canvas to place the texture on. |
| `-PadColor` | double[] |  | The color around the padded texture: red, green, blue and alpha from 0 to 1; transparent black when omitted. |
| `-PowerOfTwo` | [Wgpu.PowerOfTwo](Types.md#wgpupoweroftwo) |  | Rounds each side to a power of two: Nearest, Up or Down. |
| `-Scale` | double |  | A factor for both sides, such as 0.5 for half the size. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The texture to resize; it is left as it is. |
| `-Width` | uint |  | The new width; without -Height, the height keeps the aspect ratio. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuNoTexture`, `WgpuResize`, `WgpuCrop`, `WgpuPad` and `WgpuTransform`; `WgpuNormalMap`, for `-NormalMips` with the Min or Max filter; from reading the texture back, `WgpuAspect`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; from decoding its texels and encoding them again, `WgpuTextureDecode`, `WgpuConvert`, `WgpuMipLevels`, `WgpuCompress`, `WgpuBasisEncode`, `WgpuTextureFile` and `WgpuTextureFormat`; from writing the `-Format` named, `WgpuUnknownFormat`, `WgpuTexels`, `WgpuCompressFormat` and `WgpuKtx2`; and from making the new texture, `WgpuTexture`, `WgpuTexelCount` and `WgpuTextureWrite`. Each is described in the [Error Reference](Error-Reference.md).

### Convert-WgpuTexture

Flips or rotates a texture, changes its channels and colors, or filters it, into a new texture: -Flip mirrors it and -Rotate turns it 90, 180 or 270 degrees clockwise, turning first; then -AssignColorSpace, -ColorKey, -Swizzle, -ReconstructZ, -Normalize, -FlipGreen, -Unpremultiply, -AlphaThreshold, -Exposure, -ToneMap, -Brightness and -Contrast, -Gamma, -Grayscale, -Invert, -Premultiply and -ColorSpace change each texel, in that order, whatever order they are given in; then -Blur blurs the texels, -Sharpen sharpens them and -Edges finds their edges, in that order; and last -FixAlphaBorder colors the texels with alpha 0 from the nearest texels with alpha above 0. The filters work on level 0 of each layer, and of each depth slice of a 3D texture, on its own.

The new texture keeps the texture's mip levels, rendered again from level 0 as -MipFilter, -MipEdge, -KeepCoverage and -NormalMips say, and its format, unless -ColorSpace or -AssignColorSpace switches it to the format's sRGB or linear form, or -Format names another: a color format, a BC format, encoded on the GPU, or a Basis Universal format, which -ColorSpace switches to the form it names where the format has both and -AssignColorSpace leaves as it is; -Format alone converts each level as it is. The texture is left as it is. A cube map is not flipped or rotated, as its faces would no longer meet at their edges. A swizzle, the normal steps, brightness, contrast, gamma, inversion and the gradients and Laplacian of -Edges work on the values the texture stores, the others on linear values, which for an sRGB texture are the values a shader reads. The normal steps read a normal n stored as n / 2 + 1 / 2 in a format that holds no negative values, and as n in any other.

Alias: `Convert-GpuTexture`

```powershell
Convert-WgpuTexture [-Texture] <Texture> [-Flip <FlipDirection>] [-Rotate <uint>] [-Swizzle <string>] [-ReconstructZ] [-Normalize] [-FlipGreen] [-Premultiply] [-Unpremultiply] [-ColorSpace <ColorSpace>] [-AssignColorSpace <ColorSpace>] [-Grayscale] [-GrayscaleWeights <Object>] [-Invert] [-Brightness <double>] [-Contrast <double>] [-Gamma <double>] [-Exposure <double>] [-ToneMap <ToneMapCurve>] [-ColorKey <Object>] [-ColorKeyTolerance <double>] [-AlphaThreshold <double>] [-Blur <double[]>] [-Sharpen <double>] [-SharpenRadius <double>] [-SharpenThreshold <double>] [-Edges <EdgeOperator>] [-EdgeThreshold <double[]>] [-EdgeSigma <double>] [-FilterEdge <FilterEdge>] [-FixAlphaBorder] [-BorderReach <uint>] [-MipFilter <MipFilter>] [-MipEdge <MipEdge>] [-KeepCoverage <double>] [-NormalMips <NormalMips>] [-Format <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AlphaThreshold` | double |  | Gives alpha 0 to texels whose alpha is below this, and 1 to the rest, as an alpha test would: above 0 and at most 1. |
| `-AssignColorSpace` | [Wgpu.ColorSpace](Types.md#wgpucolorspace) |  | Relabels the texels as Linear or Srgb without changing them, switching the format to that form: for a texture loaded with the wrong one. |
| `-Blur` | double[] |  | Blurs the texels by a Gaussian of this standard deviation in texels, or of two, across and down, reaching 4 standard deviations. The blur works on linear values weighed by alpha, so no color of a transparent texel bleeds in. |
| `-BorderReach` | uint |  | How far -FixAlphaBorder carries color, in texels: a texel with alpha 0 farther than this from every texel with alpha above 0 keeps its color. Every texel with alpha 0 is colored when omitted. |
| `-Brightness` | double |  | Brightens or darkens the stored values, from -100 to 100, as ImageMagick's -brightness-contrast. |
| `-ColorKey` | object |  | Gives alpha 0 to the texels of this color, as the texture stores it: red, green and blue from 0 to 1, rounded to the texture's own precision, or '#RRGGBB'. Other texels keep their alpha. |
| `-ColorKeyTolerance` | double |  | How far each of red, green and blue may be from -ColorKey's, from 0 to 1; 0, the same stored value, when omitted. |
| `-ColorSpace` | [Wgpu.ColorSpace](Types.md#wgpucolorspace) |  | Converts the texels to Linear or Srgb so their colors look the same: a format with an sRGB form, as Rgba8Unorm has Rgba8UnormSrgb, switches to that form, and any other keeps its format with its values encoded or decoded. |
| `-Contrast` | double |  | Raises or lowers the stored values' contrast around their middle, from -100 to 100, as ImageMagick's -brightness-contrast. |
| `-Edges` | [Wgpu.EdgeOperator](Types.md#wgpuedgeoperator) |  | Replaces the texels with their edges: Sobel, Scharr or Prewitt, the size of each channel's gradient, where a step from 0 to 1 reads 1; Laplacian, the size of each channel's second derivative; or Canny, 1 on the thin, linked edges of the luminance and 0 elsewhere, as scikit-image's canny finds them. Alpha is kept. |
| `-EdgeSigma` | double |  | The standard deviation, in texels, of the Gaussian Canny smooths the luminance with before its gradient; 1 when omitted. |
| `-EdgeThreshold` | double[] |  | Canny's low and high thresholds: a texel on an edge is kept where its gradient's size is at least the low one and it links to a texel at least the high one, in the scale where a step from 0 to 1 reads 1; 0.1 and 0.2 when omitted. |
| `-Exposure` | double |  | Multiplies linear values by 2 to this power: stops of exposure, applied before -ToneMap. |
| `-FilterEdge` | [Wgpu.FilterEdge](Types.md#wgpufilteredge) |  | What -Blur, -Sharpen and -Edges read past the edge of the texture: Clamp, the edge texel repeated, the default; Wrap, the other side, for a texture that tiles; or Mirror, the texture mirrored with the edge texel repeated. For Canny it is what the smoothing reads, and the gradient after repeats the edge texel, as scikit-image's does. |
| `-FixAlphaBorder` | switch |  | Gives each texel with alpha 0 the color of the nearest texel with alpha above 0, its alpha left at 0, so filtering and mip levels blend no dark or keyed color in at a cutout's edge. |
| `-Flip` | [Wgpu.FlipDirection](Types.md#wgpuflipdirection) |  | Mirrors the texture: Horizontal swaps left and right, and Vertical top and bottom. |
| `-FlipGreen` | switch |  | Flips each normal's green, between the convention glTF reads, green up, and DirectX's, green down. |
| `-Format` | string |  | The new texture's format: a color format, a BC format, encoded on the GPU, or a Basis Universal format, switched by -ColorSpace to the form it names where the format has both; the texture's own when omitted. |
| `-Gamma` | double |  | Gamma corrects the stored values as ImageMagick's -gamma: each becomes value^(1/gamma), so a gamma above 1 lightens. |
| `-Grayscale` | switch |  | Makes the texture grayscale, with Rec. 709's weights in linear light or those -GrayscaleWeights names. |
| `-GrayscaleWeights` | object |  | The weights -Grayscale uses: Rec709, in linear light; Rec601, on the stored values, as Pillow's convert('L'); or three numbers for red, green and blue, in linear light. |
| `-Invert` | switch |  | Turns red, green and blue into 1 minus their stored values, as ImageMagick's -negate; alpha is kept. |
| `-KeepCoverage` | double |  | Scales the alpha of each mip level rendered again so the share of its texels with alpha at or above this, the alpha an alpha test passes at, stays level 0's: above 0 and at most 1. |
| `-MipEdge` | [Wgpu.MipEdge](Types.md#wgpumipedge) |  | What Triangle, Kaiser, Mitchell and Lanczos read past an edge of the image: Clamp, the default; Wrap, the other side; or Mirror. |
| `-MipFilter` | [Wgpu.MipFilter](Types.md#wgpumipfilter) |  | How the mip levels rendered again are made: Box, the default, each from the level above; Triangle, Kaiser, Mitchell or Lanczos from level 0; Min or Max, the smallest or largest value of each channel among the texels each new texel covers. |
| `-Normalize` | switch |  | Scales each normal to length 1. |
| `-NormalMips` | [Wgpu.NormalMips](Types.md#wgpunormalmips) |  | Keeps the normals of a normal map unit length in each mip level rendered again: Renormalize scales each texel's normal back to length 1, and SlopeSpace filters the normals' slopes. |
| `-Premultiply` | switch |  | Multiplies red, green and blue by alpha, in linear light for an sRGB texture. |
| `-ReconstructZ` | switch |  | Rebuilds each normal's Z, in blue, from its X and Y, in red and green, as for a normal map stored in two channels. |
| `-Rotate` | uint |  | Turns the texture clockwise by 90, 180 or 270 degrees. |
| `-Sharpen` | double |  | Sharpens the texels by an unsharp mask: this many times the difference between each texel and a Gaussian blur of it is added back, in linear values weighed by alpha. |
| `-SharpenRadius` | double |  | The standard deviation, in texels, of the blur -Sharpen compares each texel with; 1 when omitted. |
| `-SharpenThreshold` | double |  | The smallest difference from the blur, from 0 to 1, that -Sharpen adds back in a channel, so smooth areas keep their texels; 0 when omitted. |
| `-Swizzle` | string |  | Rearranges the channels: four letters of r, g, b, a, 0 and 1, one for each channel of the new texture, as bgra to swap red and blue or rrr1 to spread red with an opaque alpha. An sRGB texture's channels move as the values it stores. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The texture to change; it is left as it is. |
| `-ToneMap` | [Wgpu.ToneMapCurve](Types.md#wgputonemapcurve) |  | Maps HDR colors into the range from 0 to 1: Reinhard, AcesFilmic, AgX or Neutral. |
| `-Unpremultiply` | switch |  | Divides red, green and blue by alpha where alpha is above 0, in linear light for an sRGB texture. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuNoTexture`, `WgpuRotate`, `WgpuTransform`, `WgpuSwizzle`, `WgpuGrayscale`, `WgpuAdjust` and `WgpuColorSpace`; `WgpuNormalMap`, for a normal step on a format without the channel it writes, and `-NormalMips` with the Min or Max filter; `WgpuBlur`, `WgpuSharpen`, `WgpuEdges` and `WgpuFilterEdge`, for a filter setting out of range or given without its filter, and `WgpuFilter`, from the passes that filter the texels; and the errors of reading the texture back, decoding its texels and encoding them again, writing the `-Format` named, and making the new texture that [Resize-WgpuTexture](Texture-Cmdlets.md#resize-wgputexture) lists. Each is described in the [Error Reference](Error-Reference.md).

### ConvertTo-WgpuNormalMap

Makes a normal map from a height map: each texel's normal from the slopes of the heights around it, by a kernel, at a strength.

-Strength scales the slopes: a height step of 1 between neighboring texels, heights running from 0 to 1, gives a slope of the strength, as texconv's -nmapamp does; 1 when omitted. -HeightChannel reads the heights from red, green, blue, alpha or the luminance of red, green and blue; red when omitted. -NormalKernel takes the slopes by texconv's 3 by 3 Prewitt kernel when omitted, or by central differences, Sobel's 3 by 3 or NVIDIA Texture Tools' 5 by 5, each scaled so a steady ramp gives its own slope.

Past the edges the slope at each edge carries on, unless -HeightEdge says Wrap, for a height map that tiles, Clamp or Mirror; when the edges join as neighboring texels do, the verbose stream and the Notes say the map looks like it tiles. Green points up the image, as glTF reads it, and down, the convention named after DirectX, with -FlipGreen. Alpha is 1, or the height or texconv's occlusion term as -NormalAlpha says.

The new texture is Rgba8Unorm, or the -Format named: a color format, a BC format, encoded on the GPU, or a Basis Universal format. A format that holds no negative values stores each normal n as n / 2 + 1 / 2, and any other stores n. -TwoChannel keeps X and Y alone, where the format keeps them apart: in red and green for a two-channel format such as Bc5RgUnorm, and X in red, green and blue with Y in alpha for a Basis Universal LDR format, for a shader to rebuild Z. The new texture has the height map's mip levels, rendered from level 0 as -MipFilter and -MipEdge say, each texel scaled back to length 1, or filtered in slope space as -NormalMips says. What making it involved goes to the verbose stream and its Notes.

Alias: `ConvertTo-GpuNormalMap`

```powershell
ConvertTo-WgpuNormalMap [-Texture] <Texture> [-Strength <double>] [-HeightChannel <HeightChannel>] [-NormalKernel <NormalKernel>] [-HeightEdge <HeightEdge>] [-NormalAlpha <NormalAlpha>] [-FlipGreen] [-TwoChannel] [-Format <string>] [-NormalMips <NormalMips>] [-MipFilter <MipFilter>] [-MipEdge <MipEdge>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-FlipGreen` | switch |  | Points green down the image, the convention named after DirectX; up, as glTF reads normal maps, when omitted. |
| `-Format` | string |  | The new texture's format: a color format, a BC format, encoded on the GPU, or a Basis Universal format; Rgba8Unorm when omitted. |
| `-HeightChannel` | [Wgpu.HeightChannel](Types.md#wgpuheightchannel) |  | Where the heights are read: Red, Green, Blue, Alpha or Luminance; Red when omitted. |
| `-HeightEdge` | [Wgpu.HeightEdge](Types.md#wgpuheightedge) |  | What is read past the edges: Continue, the slope at the edge carried on, when omitted; Wrap, the other side; Clamp or Mirror. Alias: `-Edge`. |
| `-MipEdge` | [Wgpu.MipEdge](Types.md#wgpumipedge) |  | What Triangle, Kaiser, Mitchell and Lanczos read past an edge: Clamp, the default; Wrap; or Mirror. |
| `-MipFilter` | [Wgpu.MipFilter](Types.md#wgpumipfilter) |  | How the mip levels are filtered: Box, the default, each from the level above; Triangle, Kaiser, Mitchell or Lanczos from level 0. |
| `-NormalAlpha` | [Wgpu.NormalAlpha](Types.md#wgpunormalalpha) |  | What alpha holds: One when omitted, Height or Occlusion. Alias: `-Alpha`. |
| `-NormalKernel` | [Wgpu.NormalKernel](Types.md#wgpunormalkernel) |  | The kernel the slopes are taken by: Prewitt, texconv's, when omitted, Central, Sobel or Sobel5x5. Alias: `-Kernel`. |
| `-NormalMips` | [Wgpu.NormalMips](Types.md#wgpunormalmips) |  | How the mip levels keep their normals unit length: Renormalize when omitted, or SlopeSpace. |
| `-Strength` | double |  | How steep the slopes are: a height step of 1 between neighboring texels gives a slope of this; 1 when omitted. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The height map; it is left as it is. |
| `-TwoChannel` | switch |  | Keeps X and Y alone where -Format keeps them apart: in red and green for a two-channel format, and X in red, green and blue with Y in alpha for a Basis Universal LDR format. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuNoTexture`; `WgpuNormalMap`, for a strength that is not finite, a format that would store the normals as sRGB colors, a `-TwoChannel` the format cannot lay out, the Min or Max mip filter, and a height map that is not 2D or is a cube map; and `WgpuMipFilter` and `WgpuMipEdge`, for mip settings that change nothing. From reading the height map back, `WgpuAspect`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; from making the normals, `WgpuTextureDecode`, `WgpuTransform`, `WgpuConvert` and `WgpuMipLevels`; from writing the format the normal map takes, `WgpuUnknownFormat`, `WgpuTextureFormat`, `WgpuCompress`, `WgpuCompressFormat`, `WgpuBasisEncode`, `WgpuKtx2` and `WgpuTextureFile`; and from making it, `WgpuTexture`, `WgpuTexelCount` and `WgpuTextureWrite`. Each is described in the [Error Reference](Error-Reference.md).

### ConvertTo-WgpuCubeMap

Makes a cube map: from six textures, its faces, or from one image that holds every face, in a layout of square cells or a projection of every direction.

Six textures are the faces in the order +X, -X, +Y, -Y, +Z, -Z, given as one array or by -PositiveX, -NegativeX, -PositiveY, -NegativeY, -PositiveZ and -NegativeZ: square 2D textures of one size and format, whose texels, and the mip levels all six have, are kept as they are.

One image is read by its shape: a 2:1 image is a panorama, 4:3 a horizontal cross, 3:4 a vertical cross, and 6:1 and 1:6 strips of the faces in order. -Layout names the rest: a HorizontalTee, a VerticalCrossTurned, Godot's Grid2x3 and Grid3x2, and the square AngularMap and Octahedral map. A layout's faces are cut from its cells as they are. A panorama, an angular map or an octahedral map is resampled on the GPU: each texel of the cube is the mean of a grid of samples over it, each weighed by the solid angle it covers. A panorama's center faces +X and its left quarter -Z, as three.js and Khronos's glTF IBL sampler read it; -Yaw turns the image about the vertical axis first, +X toward -Z. The faces are a quarter of a panorama's or an angular map's width, and half an octahedral map's, unless -FaceSize says.

The cube keeps the source's format, or takes the -Format named: a color format, a BC format, encoded on the GPU, or a Basis Universal format. A cube made from one image has mip level 0 alone, which GenerateMips(0) fills out. What making it involved goes to the verbose stream and its Notes.

Alias: `ConvertTo-GpuCubeMap`

```powershell
ConvertTo-WgpuCubeMap [-Texture] <Texture[]> [-Layout <CubeLayout>] [-Yaw <double>] [-FaceSize <uint>] [-Format <string>] [<CommonParameters>]
ConvertTo-WgpuCubeMap -PositiveX <Texture> -NegativeX <Texture> -PositiveY <Texture> -NegativeY <Texture> -PositiveZ <Texture> -NegativeZ <Texture> [-Format <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-FaceSize` | uint |  | The faces' size in texels for a cube resampled from a panorama, an angular map or an octahedral map; a quarter of a panorama's or an angular map's width, and half an octahedral map's, when omitted. |
| `-Format` | string |  | The cube's format: a color format, a BC format, encoded on the GPU, or a Basis Universal format; the source's when omitted. |
| `-Layout` | [Wgpu.CubeLayout](Types.md#wgpucubelayout) |  | How the image holds the faces: needed for a HorizontalTee, a VerticalCrossTurned, a Grid2x3, a Grid3x2, an AngularMap or an Octahedral map, which the image's shape does not tell apart. |
| `-NegativeX` | [Wgpu.Texture](Types.md#wgputexture) |  | The -X face. |
| `-NegativeY` | [Wgpu.Texture](Types.md#wgputexture) |  | The -Y face. |
| `-NegativeZ` | [Wgpu.Texture](Types.md#wgputexture) |  | The -Z face. |
| `-PositiveX` | [Wgpu.Texture](Types.md#wgputexture) |  | The +X face. |
| `-PositiveY` | [Wgpu.Texture](Types.md#wgputexture) |  | The +Y face. |
| `-PositiveZ` | [Wgpu.Texture](Types.md#wgputexture) |  | The +Z face. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture)[] | by value | One image holding the faces, or six textures, the faces in the order +X, -X, +Y, -Y, +Z, -Z. |
| `-Yaw` | double |  | Turns a panorama, an angular map or an octahedral map about the vertical axis by this many degrees, +X toward -Z, before it is read. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuCubeMap`, for a count of textures other than one or six, six faces that differ in size or format or are not square 2D textures of one layer, an image of a shape no layout or projection has or that the `-Layout` named does not fit, `-Layout`, `-Yaw` or `-FaceSize` beside six faces or a layout of cells, a `-Yaw` that is not finite or a `-FaceSize` of 0, and from the passes that resample a projection. From reading the textures back, `WgpuTextureUsage`, `WgpuAspect`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; from decoding and converting their texels, `WgpuTextureDecode`, `WgpuConvert` and `WgpuMipLevels`; from writing the format the cube takes, `WgpuUnknownFormat`, `WgpuTextureFormat`, `WgpuTexels`, `WgpuCompress`, `WgpuCompressFormat`, `WgpuBasisEncode`, `WgpuKtx2` and `WgpuTextureFile`; and from making it, `WgpuTexture`, `WgpuTexelCount` and `WgpuTextureWrite`. Each is described in the [Error Reference](Error-Reference.md).

### ConvertFrom-WgpuCubeMap

Writes a cube map into one image: its faces in a layout of square cells, or a projection of every direction.

-Layout names the image: a HorizontalCross, the default, a VerticalCross, a VerticalCrossTurned, a HorizontalTee, a HorizontalStrip, a VerticalStrip, a Grid2x3, a Grid3x2, an AngularMap or an Octahedral map; -Panorama makes a panorama instead. A layout's cells take the faces as they are, and the cells no face takes are transparent black. A panorama, an angular map or an octahedral map is resampled on the GPU, each texel the mean of a grid of samples over it, each weighed by the solid angle it covers; an angular map's texels outside its disc are transparent black. A panorama is 4 faces wide and 2 high, an angular map 4 faces across and an octahedral map 2, the faces the cube's size or -FaceSize, and -Yaw turns the image about the vertical axis as ConvertTo-WgpuCubeMap reads it, so the same -Yaw both ways gives the image back.

The image comes from mip level 0, or -MipLevel, and keeps the cube's format, or takes the -Format named: a color format, a BC format, encoded on the GPU, or a Basis Universal format. What making it involved goes to the verbose stream and its Notes.

Alias: `ConvertFrom-GpuCubeMap`

```powershell
ConvertFrom-WgpuCubeMap [-Texture] <Texture> [-Layout <CubeLayout>] [-Panorama] [-Yaw <double>] [-FaceSize <uint>] [-MipLevel <uint>] [-Format <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-FaceSize` | uint |  | The size in texels of the faces a panorama, an angular map or an octahedral map is made for; the cube's when omitted. |
| `-Format` | string |  | The image's format: a color format, a BC format, encoded on the GPU, or a Basis Universal format; the cube's when omitted. |
| `-Layout` | [Wgpu.CubeLayout](Types.md#wgpucubelayout) |  | The layout or projection of the image; HorizontalCross when omitted. |
| `-MipLevel` | uint |  | The cube's mip level the image comes from; 0 when omitted. |
| `-Panorama` | switch |  | Makes a panorama, center +X and left quarter -Z. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The cube map: six square layers, its faces. |
| `-Yaw` | double |  | Turns a panorama, an angular map or an octahedral map about the vertical axis by this many degrees, +X toward -Z, as ConvertTo-WgpuCubeMap reads it. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuNoTexture`; `WgpuCubeMap`, for a texture that is not six square layers, `-Layout` with `-Panorama`, `-Yaw` or `-FaceSize` with a layout of cells, a `-Yaw` that is not finite or a `-FaceSize` of 0, and from the pass that resamples a projection; `WgpuMipLevels`, for a `-MipLevel` past the cube's last; and the errors of reading the cube back, converting its texels, writing the format the image takes and making it that [ConvertTo-WgpuCubeMap](Texture-Cmdlets.md#convertto-wgpucubemap) lists. Each is described in the [Error Reference](Error-Reference.md).

### ConvertTo-WgpuIrradianceMap

Makes the irradiance cube of an environment: each texel the light a surface facing its direction receives from every direction, weighed by the cosine of the angle between them and divided by pi, the color a white diffuse surface reflects, which a shader multiplies by the surface's albedo.

The environment is a cube map, or a panorama or layout, converted first as ConvertTo-WgpuCubeMap converts it, with -Layout and -Yaw. Each texel sums every texel of the environment, brought to 64 texels a face or fewer by the Box filter, by the exact solid angle each covers, and divides by the sum of the weights, so an environment of one color gives that color. The cube is 32 texels a face, or -FaceSize, in Rgba16Float, or the -Format named, with one mip level.

-Harmonics also gives the texture a Harmonics property: the irradiance as the nine spherical harmonics of each color channel cmgen's --sh-irradiance writes, which a shader can evaluate in place of the cube. What making it involved goes to the verbose stream and its Notes.

Alias: `ConvertTo-GpuIrradianceMap`

```powershell
ConvertTo-WgpuIrradianceMap [-Texture] <Texture> [-FaceSize <uint>] [-Harmonics] [-Format <string>] [-Layout <CubeLayout>] [-Yaw <double>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-FaceSize` | uint |  | The faces' size in texels; 32 when omitted. |
| `-Format` | string |  | The format: a color format, a BC format, encoded on the GPU, or a Basis Universal format; Rgba16Float when omitted. |
| `-Harmonics` | switch |  | Gives the texture the irradiance's spherical harmonics too, as its Harmonics property. |
| `-Layout` | [Wgpu.CubeLayout](Types.md#wgpucubelayout) |  | How an environment given as one image holds the faces, as ConvertTo-WgpuCubeMap's -Layout says. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The environment: a cube map, or a panorama or layout. |
| `-Yaw` | double |  | Turns an environment given as a panorama, an angular map or an octahedral map about the vertical axis by this many degrees, +X toward -Z. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuNoTexture`; `WgpuCubeMap`, for an environment that is neither six square layers nor one image a cube is read from, `-Layout` or `-Yaw` beside six faces, a `-FaceSize` of 0, and from its passes; and the errors of reading the environment back, reading a cube from one image, writing the format the map takes and making it that [ConvertTo-WgpuCubeMap](Texture-Cmdlets.md#convertto-wgpucubemap) lists. Each is described in the [Error Reference](Error-Reference.md).

### ConvertTo-WgpuSpecularMap

Makes the prefiltered specular cube of an environment, which split-sum image-based lighting reads beside the BRDF table: each mip level the environment as the GGX lobe of one roughness reflects it, the view taken along each texel's direction.

-RoughnessMapping Linear, the default, gives level n of N perceptual roughness n / (N - 1), down to 2x2, as Khronos's glTF IBL sampler writes the levels and glTF viewers read them; Quadratic gives it 1 - sqrt(1 - n / (N - 1)), down to 16x16, as Filament's cmgen writes them. -MipLevels sets N, 0 for the full chain. Level 0, roughness 0, is the environment itself, its faces' size. Each texel of the other levels takes 1024 samples, or -Samples, of the GGX lobe at Hammersley's points, each reading the environment's mip level whose texels cover about the solid angle the sample covers, as Khronos's sampler computes that level for Linear and cmgen for Quadratic.

The environment is a cube map, or a panorama or layout, converted first as ConvertTo-WgpuCubeMap converts it, with -Layout and -Yaw. The cube is Rgba16Float, or the -Format named. What making it involved goes to the verbose stream and its Notes.

Alias: `ConvertTo-GpuSpecularMap`

```powershell
ConvertTo-WgpuSpecularMap [-Texture] <Texture> [-RoughnessMapping <RoughnessMapping>] [-Samples <uint>] [-MipLevels <uint>] [-Format <string>] [-Layout <CubeLayout>] [-Yaw <double>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Format` | string |  | The format: a color format, a BC format, encoded on the GPU, or a Basis Universal format; Rgba16Float when omitted. |
| `-Layout` | [Wgpu.CubeLayout](Types.md#wgpucubelayout) |  | How an environment given as one image holds the faces, as ConvertTo-WgpuCubeMap's -Layout says. |
| `-MipLevels` | uint |  | Mip levels: down to 2x2 for Linear and to 16x16 for Quadratic when omitted, and 0 for the full chain down to 1x1. |
| `-RoughnessMapping` | [Wgpu.RoughnessMapping](Types.md#wgpuroughnessmapping) |  | How the mip levels step through roughness: Linear, as Khronos's sampler, when omitted, or Quadratic, as cmgen. |
| `-Samples` | uint |  | Samples a texel; 1024 when omitted. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The environment: a cube map, or a panorama or layout. |
| `-Yaw` | double |  | Turns an environment given as a panorama, an angular map or an octahedral map about the vertical axis by this many degrees, +X toward -Z. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuNoTexture`; `WgpuSamples`, for a `-Samples` of 0; `WgpuMipLevels`, for more `-MipLevels` than the faces have; `WgpuCubeMap`, for an environment that is neither six square layers nor one image a cube is read from, `-Layout` or `-Yaw` beside six faces, and from its passes; and the errors of reading the environment back, reading a cube from one image, writing the format the map takes and making it that [ConvertTo-WgpuCubeMap](Texture-Cmdlets.md#convertto-wgpucubemap) lists. Each is described in the [Error Reference](Error-Reference.md).

### New-WgpuBrdfTable

Makes the BRDF table split-sum image-based lighting reads beside the specular map: for the cosine of the view's angle across and perceptual roughness down, the part of the specular light F0 scales, in red, and the part it adds, in green, with Smith's height-correlated GGX visibility, as Khronos's glTF IBL sampler and Filament compute them.

Texel (x, y) holds cosine (x + 0.5) / size and roughness (y + 0.5) / size, so a shader reads it at (NdotV, roughness) and lights a surface with the specular map's color times F0 times red plus green. The table is 128 texels a side, or -Size, from 1024 samples a texel, or -Samples, in Rg16Float, or the -Format named.

Alias: `New-GpuBrdfTable`

```powershell
New-WgpuBrdfTable [-Size <uint>] [-Samples <uint>] [-Format <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to make it on; the default device when omitted. |
| `-Format` | string |  | The format: a color format, a BC format, encoded on the GPU, or a Basis Universal format; Rg16Float when omitted. |
| `-Samples` | uint |  | Samples a texel; 1024 when omitted. |
| `-Size` | uint |  | The table's side in texels; 128 when omitted. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuBrdfTable`, for a `-Size` of 0; `WgpuSamples`, for a `-Samples` of 0; `WgpuCubeMap`, from its pass; from reading the table back, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; from writing the format it takes, `WgpuUnknownFormat`, `WgpuTextureFormat`, `WgpuTexels`, `WgpuConvert`, `WgpuCompress`, `WgpuCompressFormat`, `WgpuBasisEncode`, `WgpuKtx2` and `WgpuTextureFile`; from making it, `WgpuTexture`, `WgpuTexelCount` and `WgpuTextureWrite`; and without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md).

### Join-WgpuTexture

Packs channels of several textures, or numbers, into one new texture: -Red, -Green, -Blue and -Alpha each take a texture, whose red channel they read; a texture and a channel letter as a pair, as ($mask, 'a'); or a number. An omitted color channel is 0, and an omitted alpha 1.

The textures are 2D, of one layer, on one device; one smaller than the largest is resized to its size first, with Bicubic where it grows. An sRGB texture gives the values it stores. The new texture is Rgba8Unorm, or the -Format named: a color format, a BC format, encoded on the GPU, or a Basis Universal format. What making it involved goes to the verbose stream and its Notes.

Alias: `Join-GpuTexture`

```powershell
Join-WgpuTexture [[-Red] <Object>] [[-Green] <Object>] [[-Blue] <Object>] [[-Alpha] <Object>] [-Format <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Alpha` | object |  | The new texture's alpha, as -Red takes it; 1 when omitted. |
| `-Blue` | object |  | The new texture's blue, as -Red takes it; 0 when omitted. |
| `-Format` | string |  | The new texture's format: a color format, a BC format, encoded on the GPU, or a Basis Universal format such as BasisUastc4x4; Rgba8Unorm when omitted. |
| `-Green` | object |  | The new texture's green, as -Red takes it; 0 when omitted. |
| `-Red` | object |  | The new texture's red: a texture, whose red channel it takes; a texture and a channel letter as a pair; or a number. 0 when omitted. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuJoin` and `WgpuOtherDevice`; from reading the textures back, `WgpuTextureUsage`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; from decoding and resizing them, `WgpuTextureDecode`, `WgpuConvert` and `WgpuTransform`; from writing the format the new texture takes, `WgpuUnknownFormat`, `WgpuTextureFormat`, `WgpuCompress`, `WgpuCompressFormat`, `WgpuBasisEncode`, `WgpuKtx2` and `WgpuTextureFile`; and from making it, `WgpuTexture`, `WgpuTexelCount` and `WgpuTextureWrite`. Each is described in the [Error Reference](Error-Reference.md).

### Compare-WgpuTexture

Compares a texture with a reference, such as a block-compressed copy with its original: for each mip level both have, each layer and each channel both store, a row with the mean squared error (MSE), the peak signal-to-noise ratio (PSNR) and the structural similarity (SSIM) of the texture's values against the reference's. With -Difference it writes a picture of the error instead.

The textures need the same dimension, width, height and layers, and an NV12 or P010 texture compares only with another NV12 or P010 texture; their formats and mip levels may differ. Each value is read as the texture stores it: from 0 to 1, an sRGB texture in its encoded form, a block-compressed one decoded on the CPU first; snorm values from -1 to 1; an integer format through the norm format with its bits, as R8Uint reads as R8Unorm; depth from 0 to 1 and stencil over 255, a depth-stencil texture's depth as Red and its stencil as Green; a multisampled texture as a resolve would store it, each texel's samples averaged and rounded to the format; and an NV12 or P010 texture's luma as Red at the texture's size and its chroma as Green and Blue at half its width and height.

PSNR's peak is 1, 2 for values from -1 to 1, and for a float texture the reference channel's largest value in that level and layer, or 1 when that is 0 or below. SSIM is Wang et al.'s, as scikit-image computes it with a Gaussian window of sigma 1.5 and the peak as its data range, averaged over the level less a 5-texel border; a level under 11 texels on a side has none.

The difference picture is an Rgba8Unorm texture of the textures' size, layers and shared mip levels. Each texel shows the error on a fixed log scale: black at 2^-12 or less, full at 1 or more, each twelfth of brightness doubling the error, so the same brightness means the same error in every picture. Red, green and blue show in their own colors with alpha opaque, or -Channel shows one channel, alpha included, as gray. The chroma of an NV12 or P010 texture shows each chroma texel's error over the 2 by 2 texels that share it.

Alias: `Compare-GpuTexture`

```powershell
Compare-WgpuTexture [-Reference] <Texture> [-Texture] <Texture> [<CommonParameters>]
Compare-WgpuTexture [-Reference] <Texture> [-Texture] <Texture> -Difference [-Channel <TextureChannel>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Channel` | [Wgpu.TextureChannel](Types.md#wgputexturechannel) |  | The channel the picture shows, as gray: Red, Green, Blue or Alpha; red, green and blue in their own colors when omitted. |
| `-Difference` | switch |  | Writes a picture of the error, an Rgba8Unorm texture, instead of the rows. |
| `-Reference` | [Wgpu.Texture](Types.md#wgputexture) |  | The texture to compare with, such as the original a copy was made from. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The texture to check against the reference, such as a block-compressed copy of it. |

Writes [Wgpu.TextureComparison](Types.md#wgputexturecomparison) or [Wgpu.Texture](Types.md#wgputexture).

#### Errors

`WgpuNoTexture`; `WgpuCompare`, for textures of another dimension, size or number of layers, and for a `-Channel` the textures do not both store; and `WgpuSsim`, from SSIM's pass. From reading the textures back, `WgpuTextureUsage`, `WgpuAspect`, `WgpuBufferTooLarge`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; from decoding their texels, `WgpuTextureDecode` and `WgpuTexels`; and with `-Difference`, from making the picture, `WgpuTexture`, `WgpuTexelCount` and `WgpuTextureWrite`. Each is described in the [Error Reference](Error-Reference.md).

### Measure-WgpuTexture

Measures a texture's values: for each mip level, layer and channel, a row with the minimum, maximum, mean, population standard deviation and a histogram of the values.

Each value is read as Compare-WgpuTexture reads it: from 0 to 1 as the texture stores it, an sRGB texture in its encoded form, a block-compressed one decoded on the CPU first; snorm values from -1 to 1; an integer format through the norm format with its bits; depth from 0 to 1 and stencil over 255; a multisampled texture as a resolve would store it; and an NV12 or P010 texture's luma as Red and its chroma as Green and Blue, measured at the chroma plane's size.

The histogram's bins split -Range evenly, 256 bins from 0 to 1 when omitted, so each 8-bit value has a bin of its own. Values below the range count in the first bin and values above it in the last. A NaN value counts in no bin and makes the row's minimum, maximum, mean and standard deviation NaN.

Alias: `Measure-GpuTexture`

```powershell
Measure-WgpuTexture [-Texture] <Texture> [-Bins <uint>] [-Range <double[]>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Bins` | uint |  | The histogram's bins: 256 when omitted. |
| `-Range` | double[] |  | The values the histogram's bins split: the low end and the high end, 0 and 1 when omitted. |
| `-Texture` | [Wgpu.Texture](Types.md#wgputexture) | by value | The texture to measure. |

Writes [Wgpu.TextureMeasurement](Types.md#wgputexturemeasurement).

#### Errors

`WgpuNoTexture`, and `WgpuMeasure`, for a `-Bins` of 0, a `-Range` that is not two finite numbers with the first below the second, and a histogram too large to hold in memory. From reading the texture back, `WgpuTextureUsage`, `WgpuAspect`, `WgpuBufferTooLarge`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`; and from decoding its texels, `WgpuTextureDecode` and `WgpuTexels`. Each is described in the [Error Reference](Error-Reference.md).

### Get-WgpuTextureFormat

Lists every texture format wgpu has, with its block size, channels and number type, the features it needs, and what the adapter can do with it.

Alias: `Get-GpuTextureFormat`

```powershell
Get-WgpuTextureFormat [[-Name] <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device whose adapter's capabilities to report; the default device when omitted. |
| `-Name` | string |  | Only the formats whose name contains this text. |

Writes [Wgpu.TextureFormatInfo](Types.md#wgputextureformatinfo).

#### Errors

Without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`, each described in the [Error Reference](Error-Reference.md).

### New-WgpuSampler

Creates a sampler: how a shader filters and wraps when it samples a texture.

Alias: `New-GpuSampler`

```powershell
New-WgpuSampler [-Filter <FilterMode>] [-MagFilter <FilterMode>] [-MinFilter <FilterMode>] [-MipmapFilter <FilterMode>] [-AddressMode <AddressMode>] [-AddressModeU <AddressMode>] [-AddressModeV <AddressMode>] [-AddressModeW <AddressMode>] [-LodMinClamp <float>] [-LodMaxClamp <float>] [-Compare <CompareFunction>] [-Anisotropy <ushort>] [-BorderColor <BorderColor>] [-Label <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AddressMode` | [Wgpu.AddressMode](Types.md#wgpuaddressmode) |  | The address mode on every axis; ClampToEdge when omitted. |
| `-AddressModeU` | [Wgpu.AddressMode](Types.md#wgpuaddressmode) |  | The address mode across. |
| `-AddressModeV` | [Wgpu.AddressMode](Types.md#wgpuaddressmode) |  | The address mode down. |
| `-AddressModeW` | [Wgpu.AddressMode](Types.md#wgpuaddressmode) |  | The address mode through a 3D texture. |
| `-Anisotropy` | ushort |  | The anisotropic filtering clamp, 1 to 16; 1 when omitted. Above 1 needs every filter Linear. |
| `-BorderColor` | [Wgpu.BorderColor](Types.md#wgpubordercolor) |  | The border color for ClampToBorder. |
| `-Compare` | [Wgpu.CompareFunction](Types.md#wgpucomparefunction) |  | Make a comparison sampler with this comparison. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to create it on; the default device when omitted. |
| `-Filter` | [Wgpu.FilterMode](Types.md#wgpufiltermode) |  | The mag, min and mipmap filters together; Nearest when omitted. |
| `-Label` | string |  | The sampler's label. |
| `-LodMaxClamp` | float |  | The highest mip level of detail; 32 when omitted. |
| `-LodMinClamp` | float |  | The lowest mip level of detail; 0 when omitted. |
| `-MagFilter` | [Wgpu.FilterMode](Types.md#wgpufiltermode) |  | The filter when a texel covers more than a pixel. |
| `-MinFilter` | [Wgpu.FilterMode](Types.md#wgpufiltermode) |  | The filter when a texel covers less than a pixel. |
| `-MipmapFilter` | [Wgpu.FilterMode](Types.md#wgpufiltermode) |  | The filter between mip levels. |

Writes [Wgpu.Sampler](Types.md#wgpusampler).

#### Errors

`WgpuSampler`, and without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`; each described in the [Error Reference](Error-Reference.md).
