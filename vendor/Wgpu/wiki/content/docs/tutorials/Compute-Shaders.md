---
title: Compute Shaders
weight: 3
---

A compute shader is a function the GPU runs many times at once, each run on its own part of the data. This chapter writes one in WGSL, WebGPU's shading language, gives it arrays, decides how many times it runs, passes it settings, and builds it once to run again and again.

## A first shader

This shader doubles every number of the array it is given:

```powershell
PS> Get-Content ./double.wgsl
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
PS> $double = Get-Content ./double.wgsl -Raw
```

The first line declares a binding, a resource the shader is given: `v`, storage it may read and write, holding an array of `f32`, numbered group 0, binding 0. `@compute` marks `main` as the entry point, and `@workgroup_size(64)` says it runs in groups of 64 at a time. Each run reads its own number from `global_invocation_id`, and `id.x` picks the element it doubles. Runs come in whole groups, so the last group can run past the end of the array, and the `arrayLength` test keeps those runs from writing.

`Invoke-WgpuCompute` runs it:

```powershell
PS> Invoke-WgpuCompute $double ([float[]](1..5))
2
4
6
8
10
```

It read from the shader that `v` is an array of `f32` the shader may write, so it sent the array as Float32 and sent it back afterward.

## Several resources

A shader binds as many resources as it needs. This one adds `b` into `a`, and writes only `a`:

```powershell
PS> Get-Content ./add.wgsl
@group(0) @binding(0) var<storage, read_write> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&a)) { a[id.x] = a[id.x] + b[id.x]; }
}
PS> $add = Get-Content ./add.wgsl -Raw
```

The resources go in the order the shader declares its bindings, or in a hashtable by binding name, whatever order that is in. Only `a` comes back, since the shader cannot write `b`:

```powershell
PS> Invoke-WgpuCompute $add @([float[]](1, 2, 3), [float[]](10, 20, 30))
11
22
33

PS> Invoke-WgpuCompute $add @{ b = [float[]](10, 20, 30); a = [float[]](1, 2, 3) }
11
22
33
```

## How many times it runs

Given no size, `Invoke-WgpuCompute` runs the shader once for each element of the first array it binds, in whole groups of 64: five elements take one group, and 100 take two. `-Workgroups` counts the groups itself, so one group over 100 elements doubles only the first 64:

```powershell
PS> $out = Invoke-WgpuCompute $double ([float[]](1..100)) -Workgroups 1
PS> $out[63], $out[64]
128
65
```

`-Invocations` counts runs instead, and rounds them up to whole groups.

## Settings in a struct

A shader takes settings through a uniform binding, read-only and the same for every run, often a struct. An `override` is a constant fixed when the shader is compiled:

```powershell
PS> Get-Content ./affine.wgsl
struct Params { scale: f32, offset: f32 }
override bias: f32 = 0.0;
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * p.scale + p.offset + bias; }
}
PS> $affine = Get-Content ./affine.wgsl -Raw
```

The struct takes a hashtable of its members, which the module lays out as WGSL does, and the override takes its value from `-Constant`:

```powershell
PS> Invoke-WgpuCompute $affine @{ v = [float[]](1, 2, 3); p = @{ scale = 10; offset = 0.5 } }
10.5
20.5
30.5

PS> Invoke-WgpuCompute $affine @{ v = [float[]](1, 2, 3); p = @{ scale = 10; offset = 0.5 } } -Constant @{ bias = 100 }
110.5
120.5
130.5
```

## Build it once

`Invoke-WgpuCompute` compiles the shader's text on every call. A pipeline is the shader compiled once, its constants fixed, and run as often as needed; with a buffer, nothing comes back until it is read:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $tenfold = New-WgpuComputePipeline $affine -Constant @{ bias = 0 }
PS> $values = New-WgpuBuffer ([float[]](1, 2, 3))
PS> Invoke-WgpuCompute $tenfold @{ v = $values; p = @{ scale = 10 } }
PS> Invoke-WgpuCompute $tenfold @{ v = $values; p = @{ scale = 10 } }
PS> Read-WgpuBuffer $values
100
200
300
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $tenfold = $gpu.CreateComputePipeline($affine, $null, @{ bias = 0 }, $null)
PS> $values = $gpu.Upload([float[]](1, 2, 3))
PS> $tenfold.Dispatch(@{ v = $values; p = @{ scale = 10 } }, 1)
PS> $tenfold.Dispatch(@{ v = $values; p = @{ scale = 10 } }, 1)
PS> $values.Read()
100
200
300
```
{{< /tab >}}
{{< /tabs >}}

The offset was left out, so it is zero, and each run multiplied the buffer's numbers by ten.

## Where next

[Shaders Without A GPU](Shaders-Without-A-GPU.md) checks, inspects and translates shaders like these with no device at all.
