---
title: Getting Started
weight: 1
---

Install the module, see the GPUs it finds, run a first shader on one of them, and read what comes back, in PowerShell 7 or Windows PowerShell 5.1.

## Install

From the PowerShell Gallery, for the current user:

```powershell
Install-Module Wgpu -Scope CurrentUser
```

PowerShell 7.4 and later also take `Install-PSResource Wgpu`. PowerShell loads the module the first time one of its cmdlets runs. To build the module yourself instead, see [Building From Source](Building-From-Source.md).

The module runs on:

- PowerShell 7.4 or later, and on Windows also Windows PowerShell 5.1.
- Windows x64 and Arm64, Linux x64 and Arm64 with glibc 2.35 or later, macOS on Apple silicon and on Intel, and FreeBSD x64: the module carries a native library for each.
- A GPU and its driver: Vulkan or Direct3D 12 on Windows, Vulkan or OpenGL on Linux and FreeBSD, Metal on macOS. Without one, a software adapter takes its place where the system has one: WARP, which comes with Windows, and lavapipe or llvmpipe, which come with Mesa. The shader cmdlets need no GPU at all.

wgpu, its shader compiler naga, and on Windows x64 the DXC compiler are compiled into the module, with the C runtime on Windows, and on Windows Arm64 the module carries Microsoft's DXC beside its library, so no Visual C++ Redistributable or Vulkan SDK is needed.

## The GPUs it finds

The examples on this page ran in order in one PowerShell 7.6 session on the machine [Where It Has Been Run](Where-It-Has-Been-Run.md) describes, an RTX 5070 beside the Radeon graphics of a Ryzen 9 7900X. A machine with other GPUs prints their names, and the same numbers.

An adapter is a GPU, or a software renderer, reached through one backend, the API wgpu drives it with. `Get-WgpuAdapter` lists every adapter wgpu finds, so one GPU can appear once for each backend that reaches it:

```powershell
PS> Get-WgpuAdapter | Select-Object Name, Backend, DeviceType
Name                          Backend    DeviceType
----                          -------    ----------
NVIDIA GeForce RTX 5070        Vulkan   DiscreteGpu
AMD Radeon(TM) Graphics        Vulkan IntegratedGpu
NVIDIA GeForce RTX 5070          Dx12   DiscreteGpu
AMD Radeon(TM) Graphics          Dx12 IntegratedGpu
Microsoft Basic Render Driver    Dx12           Cpu
```

Both GPUs appear through Vulkan and through Direct3D 12, which wgpu calls Dx12. Microsoft Basic Render Driver is WARP, the software adapter, and its device type, Cpu, says it runs on the processor.

## A first shader

A compute shader is a function the GPU runs many times at once. This one, in WGSL, WebGPU's shading language, doubles every element of the array it is given, and [Compute Shaders](Compute-Shaders.md) explains it line by line:

```powershell
PS> $double = '@group(0) @binding(0) var<storage, read_write> v: array<f32>;
>> @compute @workgroup_size(64)
>> fn main(@builtin(global_invocation_id) id: vec3<u32>) {
>>     if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
>> }'
PS> Invoke-WgpuCompute $double ([float[]](1..8))
2
4
6
8
10
12
14
16
```

## Read what came back

`Invoke-WgpuCompute` copied the eight numbers to the GPU, ran the shader once for each of them, and copied them back as an array of the type they went in as. `Single` is .NET's name for the 32-bit float PowerShell calls `[float]`, and the array works like any other:

```powershell
PS> $doubled = Invoke-WgpuCompute $double ([float[]](1..8))
PS> $doubled.GetType().Name
Single[]

PS> ($doubled | Measure-Object -Sum).Sum
72
```

It ran on the default device, which the first cmdlet that needs one opens on the adapter wgpu prefers, a discrete GPU before an integrated one:

```powershell
PS> Get-WgpuDevice
Adapter                 Backend DeviceType  Label
-------                 ------- ----------  -----
NVIDIA GeForce RTX 5070 Vulkan  DiscreteGpu Wgpu device 1
```

## Help at the prompt

Each cmdlet carries its help, written from the same source as the reference pages, and has a second name with `Gpu` in place of `Wgpu`:

```powershell
PS> (Get-Help Invoke-WgpuCompute).Synopsis
Runs a compute shader once over arrays or buffers and writes the arrays it may have written.

PS> (Get-Command -Module Wgpu -CommandType Cmdlet).Count
43

PS> Get-Alias -Definition Invoke-WgpuCompute | Select-Object Name, Definition
Name              Definition
----              ----------
Invoke-GpuCompute Invoke-WgpuCompute
```

## Where next

[Arrays And Buffers](Arrays-And-Buffers.md) works on arrays without writing a shader, and keeps numbers on the GPU between steps.
