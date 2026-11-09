---
title: New-GuiGpuBuffer
weight: 9
---

Creates a GPU buffer in the egui renderer, from an array of any numeric type or of a length. Source: `crates/pwrs-gui/src/gpu.rs`, `NewGuiGpuBuffer` and `GpuBuffer`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its names for unsigned integer types, `uint32` and `uint64` where PowerShell 7 writes `uint` and `ulong`, except where its output is shown too.

## Syntax

```powershell
Get-Command New-GuiGpuBuffer -Syntax
```

```text
New-GuiGpuBuffer [[-Data] <Object>] [-ElementType <ElementType>] [-Usage <BufferUsage[]>] [-Label <string>] [<CommonParameters>]

New-GuiGpuBuffer -Length <ulong> [-ElementType <ElementType>] [-Usage <BufferUsage[]>] [-Label <string>] [<CommonParameters>]
```

## Description

`New-GuiGpuBuffer` makes a buffer on the GPU device of the session's egui renderer, which starts as [Set-GuiOption](Set-GuiOption.md) says when none is running, and writes it as a `PwrsGui.GpuBuffer`. The buffer's data stays on the GPU until a command reads it, so a buffer made once can be bound to many compute runs without crossing back.

`-Data` takes an array of any numeric type, a number, or numbers piped in, which are gathered into one buffer. Without `-ElementType` the buffer takes the array's own element type, and `Float32` for an array that has none, such as `1..100`. With it, the values are converted as the Wgpu module converts them: a typed array as its elements convert, truncating, and other numbers as PowerShell casts them, a value out of the type's range refused. `-Length` makes a buffer of that many zeroed elements instead.

The buffer may be used as `-Usage` names; without it, for everything but mapping and acceleration-structure input. It is freed in the renderer when the object is collected, or at once by [Remove-GuiGpuBuffer](Remove-GuiGpuBuffer.md); a GpuSurface whose draws read it uses it until its draws change. When the renderer's device is lost, the renderer goes on with a new one and fills the buffer again from what the session wrote into it, as [Reset-GuiGpuDevice](Reset-GuiGpuDevice.md) describes.

[Read-GuiGpuBuffer](Read-GuiGpuBuffer.md) reads a buffer back, [Write-GuiGpuBuffer](Write-GuiGpuBuffer.md) overwrites its elements, [Invoke-GuiGpuCompute](Invoke-GuiGpuCompute.md) binds it to a compute shader's binding, and [Set-GuiGpuSurface](Set-GuiGpuSurface.md) gives it to a GpuSurface's draws as a vertex, index or binding buffer.

## Parameters

### -Data

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | `Data`: position 0 |
| Pipeline input | by value |
| Refused with | `GuiGpuEmpty` for no elements, and the Wgpu module's conversion errors, as [Errors](#errors) shows |

The values: an array of any numeric type, a number, or numbers piped in.

### -Length

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | `Length`: mandatory |
| Pipeline input | no |
| Refused with | `GuiGpuEmpty` for 0, `GuiGpuBufferTooLarge` for more bytes than a buffer can address |

The number of zeroed elements.

### -ElementType

| | |
|---|---|
| Type | `PwrsGui.ElementType`: `Float32`, `Float64`, `Float16`, `Int8`, `UInt8`, `Int16`, `UInt16`, `Int32`, `UInt32`, `Int64`, `UInt64` |
| Aliases | `Type` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the array's own element type, or `Float32` |

The type of the buffer's elements.

### -Usage

| | |
|---|---|
| Type | `PwrsGui.BufferUsage[]`: `MapRead`, `MapWrite`, `CopySrc`, `CopyDst`, `Index`, `Vertex`, `Uniform`, `Storage`, `Indirect`, `QueryResolve`, `BlasInput`, `TlasInput` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | every usage but `MapRead`, `MapWrite`, `BlasInput` and `TlasInput` |

What the buffer may be used for.

### -Label

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the length and the element type, as `1024 x Float32` |

The buffer's label, which the device's errors name it by.

## Inputs

Numbers, by value to `-Data`.

## Outputs

`PwrsGui.GpuBuffer`:

| Property | Type | What it gives |
|---|---|---|
| `Label` | `System.String` | the buffer's label |
| `ElementType` | `PwrsGui.ElementType` | the type of its elements |
| `Length` | `System.UInt64` | the number of elements it has |
| `Size` | `System.UInt64` | its size in bytes, the elements' bytes rounded up to a multiple of 4 |
| `Usage` | `PwrsGui.BufferUsage[]` | what it may be used for |
| `Adapter` | `System.String` | the adapter of the renderer's device; null for a renderer another module started |

## Examples

The examples from the command's help, each buffer then shown.

A buffer of 1,024 floats:

```powershell
$buf = New-GuiGpuBuffer ([float[]](1..1024))
$buf
```

```text
Label          ElementType Length Size Adapter
-----          ----------- ------ ---- -------
1024 x Float32 Float32     1024   4096 NVIDIA GeForce RTX 5070
```

A buffer of 4,096 zeroed unsigned integers:

```powershell
$zeros = New-GuiGpuBuffer -Length 4096 -Type UInt32
$zeros | Format-List Label, ElementType, Length, Size
$zeros.Usage -join ', '
```

```text
Label       : 4096 x UInt32
ElementType : UInt32
Length      : 4096
Size        : 16384

CopySrc, CopyDst, Index, Vertex, Uniform, Storage, Indirect, QueryResolve
```

Numbers piped in, gathered into one buffer of the type given:

```powershell
$ints = 1..100 | New-GuiGpuBuffer -Type Int32
Read-GuiGpuBuffer $ints -Count 5
```

```text
1
2
3
4
5
```

## Errors

The command's errors are non-terminating: the pipeline goes on and the error is written, so each attempt here stops on its own with `-ErrorAction Stop`. A buffer needs at least one element, and the values must convert to its type:

```powershell
foreach ($attempt in { New-GuiGpuBuffer -Length 0 -ErrorAction Stop },
                     { New-GuiGpuBuffer 300 -Type UInt8 -ErrorAction Stop },
                     { New-GuiGpuBuffer 'seven' -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
```

```text
GuiGpuEmpty,Pwrs.Modules.PWRS_GUI.NewGuiGpuBufferCommand: a buffer needs at least one element
WgpuOutOfRange,Pwrs.Modules.PWRS_GUI.NewGuiGpuBufferCommand: 300 does not fit in UInt8, which holds 0 to 255
WgpuNotNumeric,Pwrs.Modules.PWRS_GUI.NewGuiGpuBufferCommand: a string is not numeric data; give an array of numbers
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed`.
