---
title: Remove-GuiGpuBuffer
weight: 9
---

Takes a GPU buffer out of the egui renderer now, rather than when its object is collected, and frees its memory. Source: `crates/pwrs-gui/src/gpu.rs`, `RemoveGuiGpuBuffer`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and the layer in `crates/pwrs-gui-gpu/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Remove-GuiGpuBuffer -Syntax
```

```text
Remove-GuiGpuBuffer [-Buffer] <GpuBuffer> [-WhatIf] [-Confirm] [<CommonParameters>]
```

## Description

`Remove-GuiGpuBuffer` frees the GPU memory of a `PwrsGui.GpuBuffer` that [New-GuiGpuBuffer](New-GuiGpuBuffer.md) made. Without it, the renderer frees a buffer when PowerShell collects its object, which may be long after a script has done with it; a script that makes large buffers in a loop frees each as it goes.

A GpuSurface whose draws read the buffer goes on drawing with it, and the buffer's memory is freed only once [Set-GuiGpuSurface](Set-GuiGpuSurface.md) changes the surface's draws or the surface's window closes.

The object stays, and every command given it afterward refuses it with `GuiGpuRemoved`. Removing a buffer already removed does nothing.

## Parameters

### -Buffer

| | |
|---|---|
| Type | `PwrsGui.GpuBuffer` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | by value |

The buffer to free.

### -WhatIf and -Confirm

The common risk-mitigation parameters: `-WhatIf` says what the command would do without doing it, and `-Confirm` waits for the person to confirm first.

## Inputs

`PwrsGui.GpuBuffer`, by value to `-Buffer`.

## Outputs

None.

## Examples

The example from the command's help, then the buffer refused once it is gone, with a non-terminating error that `-ErrorAction Stop` turns into one the example catches:

```powershell
$buf = New-GuiGpuBuffer -Length 1048576
$buf | Remove-GuiGpuBuffer
try { Read-GuiGpuBuffer $buf -Count 1 -ErrorAction Stop } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
```

```text
GuiGpuRemoved,Pwrs.Modules.PWRS_GUI.ReadGuiGpuBufferCommand: the GPU buffer 1048576 x Float32 was removed
```

What a removal would do, without doing it:

```powershell
$kept = New-GuiGpuBuffer -Length 16 -Label 'scratch'
Remove-GuiGpuBuffer $kept -WhatIf
(Read-GuiGpuBuffer $kept).Length
```

```text
What if: Performing the operation "Free the buffer's GPU memory" on target "scratch".
16
```

## Errors

`Remove-GuiGpuBuffer` raises none of its own. The [Error Reference](Error-Reference.md) describes the session's errors, such as `GuiRendererGone`.
