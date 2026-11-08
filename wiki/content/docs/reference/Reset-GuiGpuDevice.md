---
title: Reset-GuiGpuDevice
weight: 9
---

Destroys the egui renderer's GPU device, as a driver reset loses one, and returns once the renderer goes on with a new device. Source: `crates/pwrs-gui/src/gpu.rs`, `ResetGuiGpuDevice`, `GpuDeviceEvent` and `watch_loss`; the renderer's side is `destroy_device` in `crates/pwrs-gui-gpu/src/lib.rs` and the restart in `crates/pwrs-gui-egui/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Reset-GuiGpuDevice -Syntax
```

```text
Reset-GuiGpuDevice [-WhatIf] [-Confirm] [<CommonParameters>]
```

## Description

A GPU device is lost when its driver resets, as Windows resets one whose shader overruns the GPU's watchdog, or when it is destroyed. The egui renderer then ends its run and starts another on a new device: it opens its windows again where they were, at their sizes, and makes every GPU buffer again, filled with what the session wrote into it through [New-GuiGpuBuffer](New-GuiGpuBuffer.md) and [Write-GuiGpuBuffer](Write-GuiGpuBuffer.md). What a shader wrote into a buffer on the GPU is gone with the device.

`Reset-GuiGpuDevice` destroys the device on purpose, so a script or a test can see that happen, and returns once the renderer is on the new device. It starts the egui renderer first when none is running.

Every runspace that has used a GPU command is told of each loss, whatever caused it, by the engine event `PwrsGui.DeviceLost`, whose `MessageData` is a `PwrsGui.GpuDeviceEvent` with the `Reason` wgpu gave. A script can redo the compute work whose results the loss took. The event is queued for `Wait-Event` and `Get-Event`; a `Register-EngineEvent -Action` block takes it instead, and runs when the engine is idle, as at an interactive prompt.

## Parameters

### -WhatIf and -Confirm

The common risk-mitigation parameters: `-WhatIf` says what the command would do without doing it, and `-Confirm` waits for the person to confirm first.

## Inputs

None.

## Outputs

None. `PwrsGui.DeviceLost`'s `MessageData` is a `PwrsGui.GpuDeviceEvent`:

| Property | Type | What it gives |
|---|---|---|
| `Reason` | `System.String` | why the device was lost, as wgpu reported it |

## Examples

The examples from the command's help. A buffer the session wrote, doubled by a shader on the GPU:

```powershell
$double = @'
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
'@
$buf = New-GuiGpuBuffer ([float[]](1, 2, 3))
Invoke-GuiGpuCompute $double -Binding $buf
Read-GuiGpuBuffer $buf
```

```text
2
4
6
```

The device destroyed, and the event that tells of it:

```powershell
Reset-GuiGpuDevice
$lost = Wait-Event -SourceIdentifier PwrsGui.DeviceLost -Timeout 20
$lost.MessageData.Reason
Remove-Event -EventIdentifier $lost.EventIdentifier
```

```text
Destroyed
```

On the new device the buffer has what the session wrote, not what the shader made of it:

```powershell
Read-GuiGpuBuffer $buf
```

```text
1
2
3
```

## Errors

A device that does not report itself lost within 10 seconds of its destruction fails the command with `GuiGpuNotLost`; the [Error Reference](Error-Reference.md) describes it with the session's errors.
