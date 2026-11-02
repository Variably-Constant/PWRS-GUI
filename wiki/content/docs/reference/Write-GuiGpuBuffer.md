---
title: Write-GuiGpuBuffer
weight: 9
---

Overwrites elements of a GPU buffer in the egui renderer with new values. Source: `crates/pwrs-gui/src/gpu.rs`, `WriteGuiGpuBuffer`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Write-GuiGpuBuffer -Syntax
```

```text
Write-GuiGpuBuffer [-Buffer] <GpuBuffer> [-Data] <Object> [-Offset <ulong>] [<CommonParameters>]
```

## Description

`Write-GuiGpuBuffer` converts `-Data` to the buffer's element type, as [New-GuiGpuBuffer](New-GuiGpuBuffer.md) converts values, and writes them over the buffer's elements from `-Offset`. The rest of the buffer is left as it was. The values must fit between the offset and the buffer's end; nothing is written when they do not.

The renderer stores what the session writes, so when its device is lost it fills the buffer again from these writes on the new device, as [Reset-GuiGpuDevice](Reset-GuiGpuDevice.md) describes.

## Parameters

### -Buffer

| | |
|---|---|
| Type | `PwrsGui.GpuBuffer` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiGpuRemoved` once [Remove-GuiGpuBuffer](Remove-GuiGpuBuffer.md) has freed it |

The buffer to write.

### -Data

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: mandatory, position 1 |
| Pipeline input | no |
| Refused with | `GuiGpuRange` for values past the buffer's end, and the Wgpu module's conversion errors |

The values, converted to the buffer's element type.

### -Offset

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0 |

The first element to overwrite.

## Inputs

None.

## Outputs

None.

## Examples

The examples from the command's help, on a buffer of eight zeroed floats:

```powershell
$buf = New-GuiGpuBuffer -Length 8
```

Three values over the first three elements:

```powershell
Write-GuiGpuBuffer $buf ([float[]](1, 2, 3))
Read-GuiGpuBuffer $buf
```

```text
1
2
3
0
0
0
0
0
```

One value at element 6, converted to the buffer's type:

```powershell
Write-GuiGpuBuffer $buf 7 -Offset 6
Read-GuiGpuBuffer $buf
```

```text
1
2
3
0
0
0
7
0
```

## Errors

The command's errors are non-terminating, so the example stops on its error with `-ErrorAction Stop`. Values that would run past the buffer's end are refused, and the buffer is left as it was:

```powershell
try { Write-GuiGpuBuffer $buf ([float[]](1..4)) -Offset 6 -ErrorAction Stop } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
Read-GuiGpuBuffer $buf
```

```text
GuiGpuRange,Pwrs.Modules.PWRS_GUI.WriteGuiGpuBufferCommand: elements 6 to 10 run past the end of a buffer of 8 elements
1
2
3
0
0
0
7
0
```

The [Error Reference](Error-Reference.md) describes these and the session's errors.
