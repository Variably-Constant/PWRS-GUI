---
title: How To Choose An Adapter
weight: 1
---

Pick the GPU a script runs on: one with the features the script needs, a discrete GPU before an integrated one, opened with only those features and made the default, and the software adapter where nothing fits, as on a machine or a CI runner with no GPU. The parameters are on [Adapters and devices](Device-Cmdlets.md), and [Devices](Devices.md) shows each part on its own.

## See what the machine has

An adapter is a GPU, or a software renderer, reached through one graphics API, so one GPU can be listed once for each. Each lists its optional features by the names `Open-WgpuDevice -Feature` takes. This script needs half-precision floats in shaders and timestamps:

```powershell
PS> $need = 'ShaderF16', 'TimestampQuery'
PS> Get-WgpuAdapter | Select-Object Name, Backend, DeviceType, @{ n = 'HasAll'; e = { $a = $_; -not ($need | Where-Object { $a.Features -notcontains $_ }) } }
Name                          Backend    DeviceType HasAll
----                          -------    ---------- ------
NVIDIA GeForce RTX 5070        Vulkan   DiscreteGpu   True
AMD Radeon(TM) Graphics        Vulkan IntegratedGpu   True
NVIDIA GeForce RTX 5070          Dx12   DiscreteGpu   True
AMD Radeon(TM) Graphics          Dx12 IntegratedGpu   True
Microsoft Basic Render Driver    Dx12           Cpu   True
```

## Pick the best one that has the features

`Open-WgpuDevice` given no adapter takes a discrete GPU first, then an integrated one, then the rest, and of each kind the first listed. This function keeps the adapters that have every feature named and picks from them in the same order:

```powershell
PS> function Select-Adapter([string[]] $Need) {
>>     $fit = @(Get-WgpuAdapter | Where-Object { $a = $_; -not ($Need | Where-Object { $a.Features -notcontains $_ }) })
>>     foreach ($kind in 'DiscreteGpu', 'IntegratedGpu', 'Other', 'VirtualGpu', 'Cpu') {
>>         $found = $fit | Where-Object { "$($_.DeviceType)" -eq $kind } | Select-Object -First 1
>>         if ($found) { return $found }
>>     }
>> }
PS> $pick = Select-Adapter $need
PS> $pick | Select-Object Name, Backend, DeviceType
Name                    Backend  DeviceType
----                    -------  ----------
NVIDIA GeForce RTX 5070  Vulkan DiscreteGpu
```

## Open it and make it the default

`Open-WgpuDevice` opens the adapter by its name and API. `-Feature` asks for only the features named, so the script fails at once on a machine whose GPU lacks one, instead of at the shader that uses it. `-Default` makes every cmdlet given no `-Device` use it; the `SetDefault` method does the same for a device opened through the class:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $gpu = Open-WgpuDevice $pick.Name -Backend $pick.Backend -Feature $need -Default -Label work
PS> Get-WgpuDevice | Select-Object Adapter, Backend, Label
Adapter                 Backend Label
-------                 ------- -----
NVIDIA GeForce RTX 5070  Vulkan work
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::new($pick.Name, $pick.Backend)
PS> $gpu.SetDefault()
PS> [Wgpu.Device]::GetDefault() | Select-Object Adapter, Backend, Label
Adapter                 Backend Label
-------                 ------- -----
NVIDIA GeForce RTX 5070  Vulkan Wgpu device 1
```

The constructor opens a device with every feature but the experimental ones, and names it Wgpu device and its number.
{{< /tab >}}
{{< /tabs >}}

## Fall back where nothing fits

The software adapter is listed with the GPUs, as an adapter of kind Cpu: WARP on Windows, lavapipe or llvmpipe from Mesa on Linux and FreeBSD. It runs every shader, slowly, and the function takes it last, so where no GPU has the features, or there is no GPU, as on a CI runner, the pick is the software adapter. `-Fallback` opens it directly:

```powershell
PS> Open-WgpuDevice -Fallback -Label fallback | Select-Object Adapter, DeviceType, Label
Adapter                       DeviceType Label
-------                       ---------- -----
Microsoft Basic Render Driver        Cpu fallback
```

Where not even the software adapter has a feature, the pick comes back empty, which a script checks before it starts. No adapter here samples ASTC, a compressed texture format of phone GPUs:

```powershell
PS> $pick = Select-Adapter 'TextureCompressionAstc'
PS> if (-not $pick) { 'no adapter here has TextureCompressionAstc' }
no adapter here has TextureCompressionAstc
```

wgpu's noop backend, `Open-WgpuDevice -Backend Noop`, takes every call, runs no shader and reads back zeros, so a script can check that its calls are valid where there is no adapter at all:

```powershell
PS> $noop = Open-WgpuDevice -Backend Noop
PS> Invoke-WgpuMap 'x * 2.0' ([float[]](1, 2, 3)) -Device $noop
0
0
0
```

## Check what you got

A device carries its adapter's limits, and a short run shows the device works before the real work starts:

```powershell
PS> $gpu = Open-WgpuDevice (Select-Adapter $need).Name -Feature $need -Default -Label work
PS> $gpu.Features -contains 'ShaderF16'
True

PS> [math]::Round($gpu.MaxStorageBufferBindingSize / 1GB, 1)
2

PS> Invoke-WgpuMap 'x * 2.0' ([float[]](1, 2, 3))
2
4
6
```

## Prefer the integrated GPU

`-LowPower` puts an integrated GPU before a discrete one, the order of wgpu's low-power preference, for the adapter that uses the least power:

```powershell
PS> (Open-WgpuDevice -LowPower).DeviceType
IntegratedGpu
```

On Windows the module looks for adapters through Vulkan and Direct3D 12, and through OpenGL only when `-Backend Gl` names it: NVIDIA's driver deadlocks the process when a thread that used OpenGL ends while another opens or closes a device. A script that needs OpenGL there keeps it to one thread that lives as long as the process.
