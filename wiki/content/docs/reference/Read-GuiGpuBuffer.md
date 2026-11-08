---
title: Read-GuiGpuBuffer
weight: 9
---

Copies a GPU buffer, or a run of its elements, back from the egui renderer and writes them as one array. Source: `crates/pwrs-gui/src/gpu.rs`, `ReadGuiGpuBuffer`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Read-GuiGpuBuffer -Syntax
```

```text
Read-GuiGpuBuffer [-Buffer] <GpuBuffer> [-Offset <ulong>] [-Count <ulong>] [-ElementType <ElementType>] [-AsMemory] [-Enumerate] [<CommonParameters>]
```

## Description

`Read-GuiGpuBuffer` copies the elements of a `PwrsGui.GpuBuffer` that [New-GuiGpuBuffer](New-GuiGpuBuffer.md) made, from `-Offset` for `-Count` elements, or all of them, off the GPU and writes them as one array of the buffer's element type: `float[]` for `Float32`, `uint[]` for `UInt32`, and so on. `-ElementType` reads the same bytes as another type. `-Enumerate` writes each element to the pipeline rather than the array, and `-AsMemory` writes a `Memory<T>` over the values, which Windows PowerShell 5.1, having no `Memory<T>`, writes as the array.

The copy waits for the GPU to finish the work already sent to it, so a read after [Invoke-GuiGpuCompute](Invoke-GuiGpuCompute.md) gets what the shader wrote. Ctrl+C ends the wait.

## Parameters

### -Buffer

| | |
|---|---|
| Type | `PwrsGui.GpuBuffer` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | by value |
| Refused with | `GuiGpuRemoved` once [Remove-GuiGpuBuffer](Remove-GuiGpuBuffer.md) has freed it |

The buffer to read.

### -Offset

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0 |
| Refused with | `GuiGpuRange` past the buffer's end |

The first element to read.

### -Count

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | to the buffer's end |
| Refused with | `GuiGpuRange` for elements past the buffer's end |

The number of elements to read.

### -ElementType

| | |
|---|---|
| Type | `PwrsGui.ElementType` |
| Aliases | `Type` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the buffer's own |

Read the bytes as this element type instead of the buffer's own.

### -AsMemory

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | an array |

Write a `Memory<T>` over the values instead of an array; an array on Windows PowerShell 5.1.

### -Enumerate

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | one array |

Write each element to the pipeline rather than one array.

## Inputs

`PwrsGui.GpuBuffer`, by value to `-Buffer`.

## Outputs

One array of the element type, or with `-Enumerate` its elements one by one, or with `-AsMemory` a `Memory<T>`.

## Examples

The examples from the command's help, on a buffer of the numbers 1 to 20:

```powershell
$buf = New-GuiGpuBuffer ([float[]](1..20))
```

The whole buffer, one array:

```powershell
$all = Read-GuiGpuBuffer $buf
$all.GetType().Name
$all.Length
```

```text
Single[]
20
```

Five elements from the eleventh:

```powershell
$buf | Read-GuiGpuBuffer -Offset 10 -Count 5
```

```text
11
12
13
14
15
```

The same bytes read as unsigned integers, the bit patterns of the floats:

```powershell
Read-GuiGpuBuffer $buf -Count 3 -Type UInt32
```

```text
1065353216
1073741824
1077936128
```

Each element to the pipeline:

```powershell
Read-GuiGpuBuffer $buf -Enumerate | Measure-Object -Sum | Select-Object Count, Sum
```

```text
Count    Sum
-----    ---
   20 210.00
```

<!-- powershell -->
```text
Count Sum
----- ---
   20 210
```

## Errors

The command's errors are non-terminating, so the example stops on its error with `-ErrorAction Stop`. A range past the buffer's end is refused before anything is copied:

```powershell
try { Read-GuiGpuBuffer $buf -Offset 15 -Count 10 -ErrorAction Stop } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
```

```text
GuiGpuRange,Pwrs.Modules.PWRS_GUI.ReadGuiGpuBufferCommand: elements 15 to 25 run past the end of a buffer of 20 elements
```

The [Error Reference](Error-Reference.md) describes these and the session's errors.
