---
title: Devices
weight: 10
---

An adapter is a GPU, or a software renderer, as one graphics API reaches it, so a GPU that both Vulkan and Direct3D 12 reach is listed once for each. A device is an adapter opened for work. Every buffer, texture, shader and pipeline is made on a device, and works only with others made on the same one. A cmdlet that does work takes its device as `-Device`, and without one uses the default device. The parameters are on [Adapters and devices](Device-Cmdlets.md), and the `Wgpu.Adapter` and `Wgpu.Device` classes on [Types](Types.md#wgpudevice).

## List the adapters

`Get-WgpuAdapter` lists every adapter on the default backends, which are Vulkan and Direct3D 12 on Windows and every backend the platform has elsewhere:

```powershell
PS> Get-WgpuAdapter
Name                          Backend DeviceType    Driver
----                          ------- ----------    ------
NVIDIA GeForce RTX 5070       Vulkan  DiscreteGpu   NVIDIA
AMD Radeon(TM) Graphics       Vulkan  IntegratedGpu AMD proprietary driver
NVIDIA GeForce RTX 5070       Dx12    DiscreteGpu   32.0.16.1742
AMD Radeon(TM) Graphics       Dx12    IntegratedGpu 32.0.21042.62
Microsoft Basic Render Driver Dx12    Cpu           10.0.26100.9711
```

`-Name` keeps the adapters whose name contains some text, ignoring case, and `-Backend` names the graphics APIs to look through:

```powershell
PS> Get-WgpuAdapter -Name nvidia -Backend Vulkan | Select-Object Name, DeviceType, Driver, DriverInfo
Name                     DeviceType Driver DriverInfo
----                     ---------- ------ ----------
NVIDIA GeForce RTX 5070 DiscreteGpu NVIDIA 617.42
```

OpenGL on Windows and wgpu's noop backend are listed only when `-Backend` names them. On Windows, NVIDIA's driver deadlocks the process when a thread that used OpenGL exits while other threads open or close devices, so a script that uses OpenGL there should keep it to one thread that lives as long as the process.

```powershell
PS> Get-WgpuAdapter -Backend Gl, Noop | Select-Object Name, Backend, DeviceType
Name                              Backend DeviceType
----                              ------- ----------
NVIDIA GeForce RTX 5070/PCIe/SSE2      Gl      Other
noop wgpu backend                    Noop        Cpu
```

## Compare adapters by their limits and features

An adapter carries every limit a device opened on it gets, such as the largest buffer it can create, and lists its optional features by the names `Open-WgpuDevice -Feature` takes:

```powershell
PS> $adapter = Get-WgpuAdapter -Name nvidia -Backend Vulkan
PS> $adapter.Limits.MaxBufferSize
4294967295

PS> $adapter.Limits.MaxComputeWorkgroupSizeX
1024

PS> $adapter.Features -contains 'TimestampQuery'
True
```

The adapters side by side, by their largest buffer and their number of features:

```powershell
PS> Get-WgpuAdapter | Select-Object Name, Backend, @{ n = 'MaxBufferSize'; e = { $_.Limits.MaxBufferSize } }, @{ n = 'Features'; e = { $_.Features.Count } }
Name                          Backend MaxBufferSize Features
----                          ------- ------------- --------
NVIDIA GeForce RTX 5070        Vulkan    4294967295       69
AMD Radeon(TM) Graphics        Vulkan    2147483648       65
NVIDIA GeForce RTX 5070          Dx12    2147483647       52
AMD Radeon(TM) Graphics          Dx12    2147483647       50
Microsoft Basic Render Driver    Dx12    2147483647       52
```

`DownlevelFlags` lists the WebGPU capabilities an older adapter may lack and this one has, and `ShaderModel` the shader model wgpu reports for it:

```powershell
PS> $adapter.DownlevelFlags.Count
26

PS> $adapter.ShaderModel
Sm5
```

## The default device

A cmdlet given no `-Device` uses the default device, which the first cmdlet that needs a device opens. It is on the adapter wgpu prefers: a discrete GPU first, then an integrated one, then the others, and the first listed of each kind.

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> Get-WgpuDevice
Adapter                 Backend DeviceType  Label
-------                 ------- ----------  -----
NVIDIA GeForce RTX 5070 Vulkan  DiscreteGpu Wgpu device 1
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> [Wgpu.Device]::GetDefault()
Adapter                 Backend DeviceType  Label
-------                 ------- ----------  -----
NVIDIA GeForce RTX 5070 Vulkan  DiscreteGpu Wgpu device 1
```
{{< /tab >}}
{{< /tabs >}}

The same device comes back each time, until another is made the default or it is closed.

## Open a device on an adapter

`Open-WgpuDevice` opens the first adapter whose name contains some text, through the backends it names; the `Wgpu.Device` constructor takes the same two things:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $radeon = Open-WgpuDevice radeon -Backend Vulkan
PS> $radeon | Select-Object Adapter, Backend, DeviceType
Adapter                 Backend    DeviceType
-------                 -------    ----------
AMD Radeon(TM) Graphics  Vulkan IntegratedGpu
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $radeon = [Wgpu.Device]::new('radeon', 'Vulkan')
PS> $radeon | Select-Object Adapter, Backend, DeviceType
Adapter                 Backend    DeviceType
-------                 -------    ----------
AMD Radeon(TM) Graphics  Vulkan IntegratedGpu
```
{{< /tab >}}
{{< /tabs >}}

Given no name, `Open-WgpuDevice` picks an adapter in the same order as the default device. `-LowPower` puts an integrated GPU before a discrete one, and `-Fallback` takes a software adapter: WARP on Windows, or lavapipe or llvmpipe from Mesa on Linux and FreeBSD.

```powershell
PS> Open-WgpuDevice -LowPower | Select-Object Adapter, Backend, DeviceType
Adapter                 Backend    DeviceType
-------                 -------    ----------
AMD Radeon(TM) Graphics  Vulkan IntegratedGpu

PS> Open-WgpuDevice -Fallback | Select-Object Adapter, Backend, DeviceType
Adapter                       Backend DeviceType
-------                       ------- ----------
Microsoft Basic Render Driver    Dx12        Cpu
```

## Make another device the default

`-Default` makes the device `Open-WgpuDevice` opens the default, and the `SetDefault` method makes any device the default:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $software = Open-WgpuDevice -Fallback -Default
PS> (Get-WgpuDevice).Adapter
Microsoft Basic Render Driver
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $software = [Wgpu.Device]::new('basic render', 'Dx12')
PS> $software.SetDefault()
PS> [Wgpu.Device]::GetDefault().Adapter
Microsoft Basic Render Driver
```
{{< /tab >}}
{{< /tabs >}}

## Keep each object on its device

A buffer, a texture or a pipeline belongs to the device it was made on, and everything it works with must be on that device too:

```powershell
PS> $here = New-WgpuBuffer ([float[]](1, 2)) -Device $radeon
PS> $there = New-WgpuBuffer -Length 2 -Device $software
PS> Copy-WgpuBuffer $here $there
Copy-WgpuBuffer: a buffer copy runs between buffers of one device
```

## Ask for features

A device gets every limit of its adapter and every feature but wgpu's experimental ones, which `-Experimental` adds. `-Feature` asks for only the features it names, and a name wgpu does not have is refused, as is a feature the adapter lacks or an experimental one without `-Experimental`:

```powershell
PS> (Open-WgpuDevice).Features.Count
62

PS> (Open-WgpuDevice -Experimental).Features.Count
69

PS> (Open-WgpuDevice -Feature ShaderF16, TimestampQuery).Features
TimestampQuery
ShaderF16

PS> Open-WgpuDevice -Feature NoSuchFeature
Open-WgpuDevice: wgpu has no feature named NoSuchFeature

PS> Open-WgpuDevice -Fallback -Feature TextureCompressionAstc
Open-WgpuDevice: Microsoft Basic Render Driver does not support the feature TextureCompressionAstc

PS> Open-WgpuDevice -Feature ExperimentalRayQuery
Open-WgpuDevice: ExperimentalRayQuery is experimental; add -Experimental to request it
```

A device carries its adapter's limits in `Limits`, and the largest buffer and the largest storage buffer binding also as properties of their own:

```powershell
PS> $gpu = Open-WgpuDevice -Label gpu
PS> $gpu.MaxBufferSize
4294967295

PS> $gpu.MaxStorageBufferBindingSize
2147483644

PS> $gpu.Limits.MaxComputeInvocationsPerWorkgroup
1024
```

## Try a script with no GPU

wgpu's noop backend opens a device on any machine. It accepts every call and keeps what is written to a buffer, but runs no shader, so a buffer a shader should have changed reads back as it was written. A shader that doubles every element:

```powershell
PS> $double = '@group(0) @binding(0) var<storage, read_write> v: array<f32>;
>> @compute @workgroup_size(64)
>> fn main(@builtin(global_invocation_id) id: vec3<u32>) {
>>     if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
>> }'
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $noop = Open-WgpuDevice -Backend Noop
PS> $data = New-WgpuBuffer ([float[]](1, 2)) -Device $noop
PS> Invoke-WgpuCompute $double $data -Device $noop
PS> Read-WgpuBuffer $data
1
2
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $noop = [Wgpu.Device]::new($null, 'Noop')
PS> $data = $noop.Upload([float[]](1, 2))
PS> $noop.Run($double, $data, 1)
PS> $noop.Download($data)
1
2
```
{{< /tab >}}
{{< /tabs >}}

Work that must run without a GPU runs on a software adapter, which `-Fallback` picks.

## Turn on validation, write a trace

`-Validation` turns on the backend's validation layers, which report misuse the driver does not: the Vulkan SDK's layers, or Direct3D 12's debug layer from Windows' Graphics Tools. `-GpuBasedValidation` adds checks run on the GPU, which are slower. Where the layers are not installed the device opens without them, as on the machine these pages ran on:

```powershell
PS> (Open-WgpuDevice -Validation).Adapter
NVIDIA GeForce RTX 5070
```

`-TracePath` writes an API trace of everything done on the device into a folder, which must exist:

```powershell
PS> $null = New-Item -ItemType Directory -Path ./trace
PS> $traced = Open-WgpuDevice -TracePath ./trace
PS> Get-ChildItem ./trace | Select-Object -ExpandProperty Name
trace.ron

PS> Open-WgpuDevice -TracePath ./no-such-folder
Open-WgpuDevice: ./no-such-folder names no folder
```

`-MemoryHint MemoryUsage` asks the backend's allocator to favor using less memory over speed, and `-Label` names the device in errors and in graphics debuggers.

## Read what a device reports

A device's status says whether it was lost and why, which errors it raised outside any call, and whether it is the default device:

```powershell
PS> $gpu.Status()
IsLost LostReason PendingErrors IsDefault
------ ---------- ------------- ---------
False             {}            False
```

The backend counts the objects a device holds and the memory they take, and may keep a report of its memory allocator, which `AllocatorReport` returns, or `$null` where it keeps none. A device with nothing made on it yet already counts the objects wgpu makes for its own use:

```powershell
PS> $gpu.Counters() | Select-Object Buffers, ComputePipelines, ShaderModules, BufferMemory
Buffers ComputePipelines ShaderModules BufferMemory
------- ---------------- ------------- ------------
      4                2             2       524784

PS> $gpu.AllocatorReport()
TotalAllocatedBytes TotalReservedBytes Allocations Blocks
------------------- ------------------ ----------- ------
             524784          201326592           4      2
```

`TimestampPeriod` gives the nanoseconds of one tick of a timestamp query on the device's queue:

```powershell
PS> $gpu.TimestampPeriod()
1
```

`StartCapture` and `StopCapture` bracket a capture for a RenderDoc or Xcode attached to the process, and do nothing with neither attached:

```powershell
PS> $gpu.StartCapture()
PS> Invoke-WgpuMap 'x + 1.0' ([float[]](1, 2)) -Device $gpu
2
3

PS> $gpu.StopCapture()
```

## Close a device

Closing a device ends it: everything made on it stops working, and it reports itself lost. `Close-WgpuDevice` first waits for the work already given to the device; the `Destroy` method does not wait, and the device reports itself lost once that work has finished.

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $scratch = Open-WgpuDevice -Label scratch
PS> $data = New-WgpuBuffer ([float[]](1, 2, 3)) -Device $scratch
PS> $scratch | Close-WgpuDevice
PS> $scratch.Status() | Select-Object IsLost, LostReason
IsLost LostReason
------ ----------
  True Destroyed

PS> Read-WgpuBuffer $data
Read-WgpuBuffer: the device was lost: Destroyed
```

A device `Open-WgpuDevice` opened raises the engine event `Wgpu.DeviceLost` when it is lost, with its label:

```powershell
PS> Get-Event -SourceIdentifier Wgpu.DeviceLost | Select-Object -ExpandProperty MessageData
Device  Kind      Message
------  ----      -------
scratch Destroyed
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $scratch = [Wgpu.Device]::new($null, $null)
PS> $data = $scratch.Upload([float[]](1, 2, 3))
PS> $scratch.Destroy()
PS> $scratch.Status() | Select-Object IsLost, LostReason
IsLost LostReason
------ ----------
  True Destroyed

PS> $scratch.Download($data)
MethodInvocationException: Exception calling "Download" with "1" argument(s): "[WgpuDeviceLost] the device was lost: Destroyed"
```

A device the constructor opens raises no engine events, since no runspace opened it.
{{< /tab >}}
{{< /tabs >}}

Without `-Device`, `Close-WgpuDevice` closes the default device, and the next cmdlet that needs a device opens another:

```powershell
PS> Close-WgpuDevice
PS> Close-WgpuDevice
Close-WgpuDevice: no device is open

PS> (Get-WgpuDevice).Adapter
NVIDIA GeForce RTX 5070
```
