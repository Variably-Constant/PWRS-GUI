---
title: Read-GuiGpuTexture
weight: 9
---

Copies a texture's texels back from the egui renderer and writes them as one array. Source: `crates/pwrs-gui/src/gpu.rs`, `ReadGuiGpuTexture`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/texture.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Read-GuiGpuTexture -Syntax
```

```text
Read-GuiGpuTexture [-Texture] <GpuTexture> [-AsBytes] [<CommonParameters>]
```

## Description

`Read-GuiGpuTexture` waits for the work the renderer was given before it, copies every texel of a `PwrsGui.GpuTexture` back from the GPU, and writes them as one array, rows packed from the top. The array is of the format's number type, as [New-GuiGpuTexture](New-GuiGpuTexture.md)'s table lists: `byte[]` for the 8-bit formats, `float[]` for the float formats, the 16-bit ones widened, `uint[]` for `R32Uint` and `int[]` for `R32Sint`. `-AsBytes` writes the texels' bytes as they are on the GPU instead, as `byte[]`. [Write-GuiGpuTexture](Write-GuiGpuTexture.md) writes them.

## Parameters

### -Texture

| | |
|---|---|
| Type | `PwrsGui.GpuTexture` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | by value |
| Refused with | `GuiGpuRemoved` once [Remove-GuiGpuTexture](Remove-GuiGpuTexture.md) has freed it |

The texture to read.

### -AsBytes

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |

Write the texels as `byte[]` instead of the format's number type.

## Inputs

`PwrsGui.GpuTexture`, by value to `-Texture`.

## Outputs

One array: `byte[]`, `float[]`, `uint[]` or `int[]`, by the texture's format, or `byte[]` with `-AsBytes`.

## Examples

The examples from the command's help, on a texture of two texels of two 16-bit floats each:

```powershell
$tex = New-GuiGpuTexture 2 1 -Format Rg16Float -Data ([float[]](0.5, 1, 2, -4))
Read-GuiGpuTexture $tex
```

```text
0.5
1
2
-4
```

The same texels as they are on the GPU, two bytes to a value, the low byte first:

```powershell
($tex | Read-GuiGpuTexture -AsBytes) -join ', '
```

```text
0, 56, 0, 60, 0, 64, 0, 196
```

## Errors

The command's errors are non-terminating, so the example stops on its error with `-ErrorAction Stop`. A texture is refused once it is removed:

```powershell
$gone = New-GuiGpuTexture 4 4 -Label 'scratch'
Remove-GuiGpuTexture $gone
try { Read-GuiGpuTexture $gone -ErrorAction Stop } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
```

```text
GuiGpuRemoved,Pwrs.Modules.PWRS_GUI.ReadGuiGpuTextureCommand: the GPU texture scratch was removed
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiRendererGone`.
