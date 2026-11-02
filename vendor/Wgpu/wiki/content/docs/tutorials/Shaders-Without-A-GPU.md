---
title: Shaders Without A GPU
weight: 4
---

naga, the shader compiler wgpu uses, is compiled into the module, so a shader can be checked, read and translated on a machine with no GPU at all. This chapter checks a shader and reads its mistakes, reads what a shader declares, and translates one into the languages other APIs take.

## Is it valid?

`Test-WgpuShader` writes `$true` for a shader naga parses and validates, and `$false` for one it does not:

```powershell
PS> Test-WgpuShader '@compute @workgroup_size(1) fn main() {}'
True

PS> Test-WgpuShader 'fn main( {'
False
```

This shader has a mistake: `id.x` is a `u32` and `rate` an `f32`, and WGSL converts neither to the other by itself:

```powershell
PS> Get-Content ./ramp.wgsl
override rate: f32 = 0.1;
@group(0) @binding(0) var<storage, read_write> ramp: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&ramp)) { ramp[id.x] = id.x * rate; }
}
```

`-Diagnostics` writes each problem with the line and column it is at, and naga's own report:

```powershell
PS> $problem = Test-WgpuShader -Path ./ramp.wgsl -Diagnostics
PS> $problem.Line, $problem.Column
5
51

PS> $problem.Message
Entry point main at Compute is invalid: Expression [10] is invalid: Operation Multiply can't work with [8] (of type Scalar(Scalar { kind: Uint, width: 4 })) and [9] (of type Scalar(Scalar { kind: Float, width: 4 }))
```

Converting the index to `f32` fixes it:

```powershell
PS> $ramp = (Get-Content ./ramp.wgsl -Raw).Replace('id.x * rate', 'f32(id.x) * rate')
PS> Test-WgpuShader $ramp
True
```

## What it declares

`Get-WgpuShaderInfo` reads a shader's entry points, the bindings they use and its override constants, which is what `Invoke-WgpuCompute` reads to know what each resource is:

```powershell
PS> $info = Get-WgpuShaderInfo $ramp
PS> $info.EntryPoints
Name Stage   WorkgroupSize
---- -----   -------------
main Compute {64, 1, 1}

PS> $info.Bindings | Select-Object Group, Binding, Name, Kind, WgslType
Group    : 0
Binding  : 0
Name     : ramp
Kind     : StorageBuffer
WgslType : array<f32>

PS> $info.Overrides
Name Id WgslType           Default
---- -- --------           -------
rate    f32      0.100000001490116
```

The default is 0.1 as an `f32` holds it, the nearest number to 0.1 that 32 bits can store.

## Into other languages

`ConvertTo-WgpuShader` translates a shader into the languages the other graphics APIs take: GLSL for OpenGL, HLSL for Direct3D, the Metal Shading Language for Metal, and SPIR-V, Vulkan's binary form. The workgroup size reads differently in each:

```powershell
PS> (ConvertTo-WgpuShader $ramp -To Glsl) -split "`n" | Select-String 'local_size'
layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

PS> (ConvertTo-WgpuShader $ramp -To Hlsl) -split "`n" | Select-String 'numthreads'
[numthreads(64, 1, 1)]

PS> (ConvertTo-WgpuShader $ramp -To SpirV).Length
213
```

SPIR-V comes back as an array of 32-bit words. naga reads SPIR-V as well as writing it, so a shader written for Vulkan runs here too.

## Where next

[Textures And Images](Textures-And-Images.md) puts images on the GPU, the other kind of data a shader reads.
