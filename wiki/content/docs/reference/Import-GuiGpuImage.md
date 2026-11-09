---
title: Import-GuiGpuImage
weight: 9
---

Loads an image file into a texture in the egui renderer. Source: `crates/pwrs-gui/src/gpu.rs`, `ImportGuiGpuImage`; the renderer's side, which decodes the file, is `import_image` in `crates/pwrs-gui-egui/src/gpu_thread.rs`, and the layer in `crates/pwrs-gui-gpu/src/texture.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Import-GuiGpuImage -Syntax
```

```text
Import-GuiGpuImage [-Path] <string> [-Format <TextureFormat>] [-Srgb] [-Label <string>] [<CommonParameters>]

Import-GuiGpuImage -LiteralPath <string> [-Format <TextureFormat>] [-Srgb] [-Label <string>] [<CommonParameters>]
```

## Description

`Import-GuiGpuImage` reads an image file and makes a texture of it on the GPU device of the session's egui renderer, as [New-GuiGpuTexture](New-GuiGpuTexture.md) makes one from an array, and writes it as a `PwrsGui.GpuTexture`. The renderer decodes PNG, JPEG, GIF, WebP, BMP and ICO files into eight bits a channel of red, green, blue and alpha, and the texture is as wide and as high as the image.

The texture is `Rgba8Unorm`, as `Import-WgpuImage` reads an image file without `-Srgb`. A GpuSurface shows its own texels as they are, so a draw that samples the image and writes what it reads shows the image as it is in the file. `-Srgb` makes the texture `Rgba8UnormSrgb`, whose samples are the colors in linear light, for a shader that lights or blends them; such a shader encodes its result as sRGB again before writing it, or the surface shows it darker. The texels are the file's bytes either way, and [Read-GuiGpuTexture](Read-GuiGpuTexture.md) reads them back as bytes.

`-Path` takes wildcards, and every file it matches loads into a texture of its own; `-LiteralPath` takes a path as written, and is where the files `Get-ChildItem` pipes bind. A file that cannot be read or decoded is reported as a non-terminating error, and the files after it still load. The texture's label is the file's full path unless `-Label` names one.

## Parameters

### -Path

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Path`: mandatory, position 0 |
| Pipeline input | by property name |
| Refused with | `GuiGpuImage` for a path that names no file |

The image file; a wildcard loads every file it matches.

### -LiteralPath

| | |
|---|---|
| Type | `System.String` |
| Aliases | `PSPath`, `LP`, `FullName` |
| Parameter sets | `LiteralPath`: mandatory |
| Pipeline input | by property name |
| Refused with | `GuiGpuImage` for a path that names no file |

The image file, named exactly as written, wildcard characters included.

### -Format

| | |
|---|---|
| Type | `PwrsGui.TextureFormat` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Rgba8Unorm`, or `Rgba8UnormSrgb` with `-Srgb` |
| Refused with | `GuiGpuImageFormat` for a format other than `Rgba8Unorm` and `Rgba8UnormSrgb`, and with `-Srgb` for one other than `Rgba8UnormSrgb` |

The texture's format: `Rgba8Unorm` or `Rgba8UnormSrgb`.

### -Srgb

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |

Read the image's color as sRGB, into a texture of `Rgba8UnormSrgb`.

### -Label

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the file's full path |

The texture's label, which the device's errors name it by.

## Inputs

Objects with a `Path`, `PSPath` or `FullName` property, by property name to `-Path` and `-LiteralPath`, as `Get-ChildItem` writes files.

## Outputs

`PwrsGui.GpuTexture`, one for each file, with the properties [New-GuiGpuTexture](New-GuiGpuTexture.md) lists.

## Examples

The examples write their own image first: a PNG of 2 by 2 texels, red and green over blue and white, from its 75 bytes.

```powershell
$quarters = 'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEklEQVR42mP4z8DwHwyBNBgAAEnICfcD2WTxAAAAAElFTkSuQmCC'
[System.IO.File]::WriteAllBytes((Join-Path $PWD 'photo.png'), [Convert]::FromBase64String($quarters))
```

The first example from the command's help, the texture as wide and as high as the image, labeled with the file's path, and its texels the file's bytes, row by row from the top:

```powershell
$photo = Import-GuiGpuImage .\photo.png
$photo | Format-List Width, Height, Format
Split-Path $photo.Label -Leaf
(Read-GuiGpuTexture $photo) -join ', '
```

```text
Width  : 2
Height : 2
Format : Rgba8Unorm

photo.png
255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255
```

The same file read as sRGB, labeled, its texels the same bytes:

```powershell
$albedo = Import-GuiGpuImage .\photo.png -Srgb -Label albedo
$albedo
(Read-GuiGpuTexture $albedo) -join ', '
```

```text
Label  Width Height Format         Adapter
-----  ----- ------ ------         -------
albedo 2     2      Rgba8UnormSrgb NVIDIA GeForce RTX 5070

255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255
```

Every PNG in a folder, as `Get-ChildItem` pipes them:

```powershell
$null = New-Item -ItemType Directory icons
Copy-Item .\photo.png .\icons\one.png
Copy-Item .\photo.png .\icons\two.png
Get-ChildItem .\icons -Filter *.png | Import-GuiGpuImage | ForEach-Object { '{0}: {1} x {2}' -f (Split-Path $_.Label -Leaf), $_.Width, $_.Height }
```

```text
one.png: 2 x 2
two.png: 2 x 2
```

## Errors

The command's errors are non-terminating, so each attempt here stops on its own with `-ErrorAction Stop`: a path that names no file, a file that is no image, a format an image is not read into, and `-Srgb` against another format. The folder the examples ran in is taken out of the messages:

```powershell
Set-Content .\notes.txt 'not an image'
foreach ($attempt in { Import-GuiGpuImage .\missing.png -ErrorAction Stop },
                     { Import-GuiGpuImage .\notes.txt -ErrorAction Stop },
                     { Import-GuiGpuImage .\photo.png -Format R32Float -ErrorAction Stop },
                     { Import-GuiGpuImage .\photo.png -Format Rgba8Unorm -Srgb -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message.Replace("$PWD\", '') }
}
```

```text
GuiGpuImage,Pwrs.Modules.PWRS_GUI.ImportGuiGpuImageCommand: Cannot find path 'missing.png' because it does not exist.
GuiGpuImage,Pwrs.Modules.PWRS_GUI.ImportGuiGpuImageCommand: notes.txt: the image could not be read: The image format could not be determined
GuiGpuImageFormat,Pwrs.Modules.PWRS_GUI.ImportGuiGpuImageCommand: photo.png: an image is imported as Rgba8UnormSrgb or Rgba8Unorm, not R32Float
GuiGpuImageFormat,Pwrs.Modules.PWRS_GUI.ImportGuiGpuImageCommand: -Srgb reads the image into Rgba8UnormSrgb, and -Format names Rgba8Unorm
```

A wildcard that matches a file that is no image loads the images around it, and reports that one:

```powershell
Copy-Item .\notes.txt .\icons\three.png
Import-GuiGpuImage .\icons\*.png -ErrorAction SilentlyContinue -ErrorVariable failed | ForEach-Object { Split-Path $_.Label -Leaf }
$failed | ForEach-Object { $_.FullyQualifiedErrorId }
```

```text
one.png
two.png
GuiGpuImage,Pwrs.Modules.PWRS_GUI.ImportGuiGpuImageCommand
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed`.
