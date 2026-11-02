---
title: Adapters and devices
weight: 10
---

The syntax, the parameters and the errors of `Get-WgpuAdapter`, `Open-WgpuDevice`, `Get-WgpuDevice`, `Wait-WgpuDevice` and `Close-WgpuDevice`. The examples are on [Devices](Devices.md).

### Get-WgpuAdapter

Lists the GPU adapters wgpu finds, with every feature and limit each supports.

Alias: `Get-GpuAdapter`

```powershell
Get-WgpuAdapter [[-Name] <string>] [-Backend <Backend[]>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Backend` | [Wgpu.Backend](Types.md#wgpubackend)[] |  | Only adapters reached through these backends; without it, Vulkan and DX12 on Windows and every backend elsewhere. Noop adds wgpu's adapter that runs nothing. |
| `-Name` | string |  | Only adapters whose name contains this text. |

Writes [Wgpu.Adapter](Types.md#wgpuadapter).

#### Errors

`WgpuLoader`, raised on Windows when psapi.dll or dbghelp.dll cannot be loaded and pinned before the first adapters are listed; described in the [Error Reference](Error-Reference.md). Pinning them keeps NVIDIA's driver from deadlocking a thread that loads them while another exits.

### Open-WgpuDevice

Opens a GPU device with every feature and limit its adapter supports.

The device is held in Rust; buffers, shaders and pipelines are made on it. -Default makes it the device cmdlets use when given no -Device.

Alias: `Open-GpuDevice`

```powershell
Open-WgpuDevice [[-Adapter] <string>] [-Backend <Backend[]>] [-LowPower] [-Fallback] [-Feature <string[]>] [-Experimental] [-Label <string>] [-TracePath <string>] [-MemoryHint <MemoryHint>] [-Validation] [-GpuBasedValidation] [-Default] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Adapter` | string |  | Open the first adapter whose name contains this text; without it, wgpu picks one. |
| `-Backend` | [Wgpu.Backend](Types.md#wgpubackend)[] |  | Look for adapters through these backends only; without it, Vulkan and DX12 on Windows and every backend elsewhere. On Windows with NVIDIA's driver, a thread that used Gl deadlocks the process if it exits while other threads open or close devices. |
| `-Default` | switch |  | Make the device the one cmdlets use when given no -Device. |
| `-Experimental` | switch |  | Allow wgpu's experimental features (ray queries, mesh shaders, cooperative matrices), which wgpu documents as unfinished. |
| `-Fallback` | switch |  | When wgpu picks, take its software adapter (WARP, lavapipe or llvmpipe). |
| `-Feature` | string[] |  | Request these features only, by name; without it every feature the adapter has is requested. |
| `-GpuBasedValidation` | switch |  | Turn on the backend's GPU-based validation as well, which is slower. |
| `-Label` | string |  | The device's label, shown in errors and in graphics debuggers. |
| `-LowPower` | switch |  | When wgpu picks, prefer the low-power adapter over the high-performance one. |
| `-MemoryHint` | [Wgpu.MemoryHint](Types.md#wgpumemoryhint) |  | Favor speed (Performance) or memory use (MemoryUsage) in the backend's allocator. |
| `-TracePath` | string |  | Write an API trace of everything done on the device into this folder, which must exist. |
| `-Validation` | switch |  | Turn on the backend's validation layers and debug labels. |

Writes [Wgpu.Device](Types.md#wgpudevice).

#### Errors

`WgpuTracePath`, `WgpuLoader`, `WgpuNoAdapter`, `WgpuUnknownFeature`, `WgpuFeatureUnsupported`, `WgpuFeatureExperimental` and `WgpuDevice`, each described in the [Error Reference](Error-Reference.md).

### Get-WgpuDevice

Gets the device cmdlets use when given no -Device, opening one on the adapter wgpu prefers if none is open.

Alias: `Get-GpuDevice`

```powershell
Get-WgpuDevice [<CommonParameters>]
```

Writes [Wgpu.Device](Types.md#wgpudevice).

#### Errors

Opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`, each described in the [Error Reference](Error-Reference.md).

### Wait-WgpuDevice

Waits until a device has finished all the work submitted to it.

Ctrl+C ends the wait; the GPU finishes the work on its own.

Alias: `Wait-GpuDevice`

```powershell
Wait-WgpuDevice [[-Device] <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) | by value | The device to wait on; the default device when omitted. |

Writes nothing to the pipeline.

#### Errors

Without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Waiting: `WgpuSubmitThread`, `WgpuPoll`, `WgpuStopped`, `WgpuDeviceLost` and `WgpuUncaptured`. Each is described in the [Error Reference](Error-Reference.md).

### Close-WgpuDevice

Destroys a device: every object made on it stops working.

It returns once the work already given to the device has finished, when the device reports itself lost and raises Wgpu.DeviceLost. Ctrl+C ends that wait.

Alias: `Close-GpuDevice`

```powershell
Close-WgpuDevice [[-Device] <Device>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) | by value | The device to destroy; the default device when omitted. |

Writes nothing to the pipeline.

#### Errors

`WgpuNoDevice`, without `-Device` when no default device is open, and from waiting for the work already given to the device, `WgpuPoll` and `WgpuStopped`; each described in the [Error Reference](Error-Reference.md).
