---
title: New-GuiImage
weight: 32
---

Describes a picture, from a file or from bytes. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiImage`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiImage -Syntax
```

```text
New-GuiImage [-Path] <string> [-Width <double>] [-Height <double>] [-Fit <ImageFit>] [-AltText <string>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]

New-GuiImage -Bytes <byte[]> [-Width <double>] [-Height <double>] [-Fit <ImageFit>] [-AltText <string>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiImage` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as the picture the file `-Path` names, read as the widget is made, or the bytes `-Bytes` gives. The picture is a PNG, JPEG, GIF or WebP, the last two animated, a BMP, an ICO or an SVG, told by its bytes rather than the file's name; anything else is refused naming what was found. In a `-Content` block `Image` stands for `New-GuiImage`.

The picture shows at its size in pixels as points, shrunk to fit the space it gets, its proportions unchanged. `-Width` or `-Height` sets that side and scales the other; both make a box that `-Fit` fills: `Contain`, the whole picture inside it; `Cover`, the box filled and what spills over cropped; `Fill`, the picture stretched to it; `None`, the picture at its own size, centered and cropped. `-AltText` is what a screen reader reads, and what shows where the picture cannot be drawn. [Update-GuiWidget](Update-GuiWidget.md) gives it a new `Path` or `Bytes`, and sets `Width`, `Height`, `Fit` and `AltText`.

## Parameters

### -Path

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Path`: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The picture's file, resolved from the current location as PowerShell resolves a path, with wildcards taken literally.

### -Bytes

| | |
|---|---|
| Type | `System.Byte[]` |
| Parameter sets | `Bytes`: named, mandatory |
| Pipeline input | no |
| Default | none |

The picture's bytes.

### -Width

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the picture's own, shrunk to fit |

The picture's width in points, above 0; without `-Height`, its height follows from its proportions.

### -Height

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the picture's own, shrunk to fit |

The picture's height in points, above 0; without `-Width`, its width follows from its proportions.

### -Fit

| | |
|---|---|
| Type | `PwrsGui.ImageFit` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Contain` |

How the picture fills the box `-Width` and `-Height` make: `Contain`, `Cover`, `Fill` or `None`. It takes both sides.

### -AltText

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The picture's text, which a screen reader reads and which shows where the picture cannot be drawn.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the picture, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the picture.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Image` and its `Properties` with `Bytes`, the picture's bytes whichever way they came, and `Width`, `Height`, `Fit` and `AltText` when they are given.

## Examples

An 8 by 4 red PNG, from its bytes, in a 160 by 80 box it covers:

```powershell
$red = [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEklEQVR4nGP4z8CAFWEXJUsCAFpeH+EeQoQoAAAAAElFTkSuQmCC')
$w = Show-GuiWindow -NoWait -Title 'Picture' {
    Image -Bytes $red -Width 160 -Height 80 -Fit Cover -AltText 'A red block' -Name Pic
}
$p = (Get-GuiSnapshot -Window $w.Id).Tree.Properties
'{0} by {1}, {2}, alt text {3}' -f $p.width, $p.height, $p.fit, $p.alt
Close-GuiWindow -Window $w.Id
```

```text
160 by 80, cover, alt text A red block
```

The same picture from a file, then replaced by the bytes while the window is open:

```powershell
$file = Join-Path ([IO.Path]::GetTempPath()) 'pwrs-gui-red.png'
[IO.File]::WriteAllBytes($file, $red)
$w = Show-GuiWindow -NoWait -Title 'Picture' { Image $file -Width 80 -Name Pic }
Update-GuiWidget -Window $w.Id -Name Pic -Property @{ Bytes = $red; Width = 40 }
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.width
Close-GuiWindow -Window $w.Id
Remove-Item -LiteralPath $file
```

```text
40
```

Bytes that are no picture, and a fit without a box:

```powershell
try { New-GuiImage -Bytes ([byte[]](1, 2, 3, 4)) } catch { $_.FullyQualifiedErrorId }
try { New-GuiImage -Bytes $red -Width 40 -Fit Cover } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadImage,Pwrs.Modules.PWRS_GUI.NewGuiImageCommand
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiImageCommand
-Fit says how the picture fills the box -Width and -Height make, so it takes both
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoFile` | `-Path` names no file that can be read |
| `GuiBadImage` | the bytes are none of the formats taken, named by what was found |
| `GuiBadSize` | `-Width` or `-Height` is not above 0 |
| `GuiBadProperty` | `-Fit` is given without both `-Width` and `-Height`, or `Update-GuiWidget` sets `Path` and `Bytes` together |

Each is described in the [Error Reference](Error-Reference.md).
