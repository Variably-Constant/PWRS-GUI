---
title: Recording commands
weight: 70
---

The syntax, the parameters and the errors of `New-WgpuCommandEncoder`, `Submit-WgpuCommand` and `New-WgpuQuerySet`. The examples are on [Recording](Recording.md).

### New-WgpuCommandEncoder

Opens a command encoder to record passes, copies and queries into.

Alias: `New-GpuCommandEncoder`

```powershell
New-WgpuCommandEncoder [[-Label] <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to record for; the default device when omitted. |
| `-Label` | string |  | The encoder's label. |

Writes [Wgpu.CommandEncoder](Types.md#wgpucommandencoder).

#### Errors

`WgpuEncoder`, and without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`; each described in the [Error Reference](Error-Reference.md). An encoder's methods throw the errors [Wgpu.CommandEncoder](Types.md#wgpucommandencoder) lists.

### Submit-WgpuCommand

Submits finished command buffers to a device's queue and waits for the GPU to run them.

Alias: `Submit-GpuCommand`

```powershell
Submit-WgpuCommand [-CommandBuffer] <CommandBuffer[]> [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-CommandBuffer` | [Wgpu.CommandBuffer](Types.md#wgpucommandbuffer)[] | by value | The command buffers, in the order they run. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device whose queue runs them; the device they were recorded on when omitted. |

Writes nothing to the pipeline.

#### Errors

`WgpuSubmitted`, `WgpuOtherDevice`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuStopped`, `WgpuDeviceLost` and `WgpuUncaptured`, each described in the [Error Reference](Error-Reference.md).

### New-WgpuQuerySet

Creates a set of queries the GPU writes results into: timestamps, occlusion counts or pipeline statistics.

Alias: `New-GpuQuerySet`

```powershell
New-WgpuQuerySet [-Kind] <QueryKind> [-Count] <uint> [-Label <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Count` | uint |  | How many queries the set holds. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to create it on; the default device when omitted. |
| `-Kind` | [Wgpu.QueryKind](Types.md#wgpuquerykind) |  | What the queries record. |
| `-Label` | string |  | The query set's label. |

Writes [Wgpu.QuerySet](Types.md#wgpuqueryset).

#### Errors

`WgpuQuerySet`, raised also for a kind the device's features do not cover, and without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`; each described in the [Error Reference](Error-Reference.md). A query set's methods throw the errors [Wgpu.QuerySet](Types.md#wgpuqueryset) lists.
