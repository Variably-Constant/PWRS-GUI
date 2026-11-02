---
title: How To Check And Translate Shaders
weight: 6
---

Check every shader of a project as a build would, report each mistake with its file, line and column, fail the build while any is left, list what each shader declares, and write the shaders out for other toolchains, all with no GPU: naga, the shader compiler wgpu uses, is inside the module. [Shaders](Shaders.md) has each part on its own.

## A folder of shaders

A WGSL compute shader, a GLSL one, and a WGSL shader with a mistake:

```powershell
PS> Get-Content ./shaders/double.wgsl
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
PS> Get-Content ./shaders/square.comp
#version 450
layout(local_size_x = 64) in;
layout(set = 0, binding = 0) buffer Data { float values[]; };
void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i < values.length()) { values[i] = values[i] * values[i]; }
}
PS> Get-Content ./shaders/ramp.wgsl
override rate: f32 = 0.1;
@group(0) @binding(0) var<storage, read_write> ramp: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&ramp)) { ramp[id.x] = id.x * rate; }
}
```

## Check them all

`Test-WgpuShader` takes the files `Get-ChildItem` pipes, reads each by its extension, and writes `$true` or `$false` for each. With `-Diagnostics` it writes each problem with the file it is in, its line and its column, which this formats as `file:line:column: message`:

```powershell
PS> Get-ChildItem ./shaders | Test-WgpuShader
True
False
True

PS> Get-ChildItem ./shaders | Test-WgpuShader -Diagnostics | ForEach-Object { '{0}:{1}:{2}: {3}' -f (Split-Path -Leaf $_.Label), $_.Line, $_.Column, $_.Message }
ramp.wgsl:5:51: Entry point main at Compute is invalid: Expression [10] is invalid: Operation Multiply can't work with [8] (of type Scalar(Scalar { kind: Uint, width: 4 })) and [9] (of type Scalar(Scalar { kind: Float, width: 4 }))
```

`id.x` is a `u32` and `rate` an `f32`, and WGSL converts neither to the other by itself.

## Fail the build while a mistake is left

A build script counts the problems and stops on any, with an exit code a CI job reads:

```powershell
PS> $problems = @(Get-ChildItem ./shaders | Test-WgpuShader -Diagnostics)
PS> "$($problems.Count) shader problem(s)"
1 shader problem(s)
```

In a build script, `if ($problems) { exit 1 }` ends it there. Fixed, the folder passes:

```powershell
PS> (Get-Content ./shaders/ramp.wgsl -Raw).Replace('id.x * rate', 'f32(id.x) * rate') | Set-Content ./shaders/ramp.wgsl
PS> @(Get-ChildItem ./shaders | Test-WgpuShader -Diagnostics).Count
0
```

naga checks a shader as a device with every feature would. `-Device` compiles each one on that device as well, against the features it was opened with, which catches a shader that needs a feature the target GPU lacks.

## List what each one declares

`Get-WgpuShaderInfo` reads each shader's entry points, bindings and override constants, labeled with the file it came from:

```powershell
PS> Get-ChildItem ./shaders | Get-WgpuShaderInfo | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Label } }, Language, @{ n = 'Workgroup'; e = { $_.EntryPoints[0].WorkgroupSize -join 'x' } }, @{ n = 'Bindings'; e = { $_.Bindings.Name -join ', ' } }
File        Language Workgroup Bindings
----        -------- --------- --------
double.wgsl     Wgsl 64x1x1    v
ramp.wgsl       Wgsl 64x1x1    ramp
square.comp     Glsl 64x1x1    binding0_0
```

A GLSL block with no instance name is named for its set and binding, as `binding0_0` is here.

## Write them for other toolchains

`ConvertTo-WgpuShader` writes a shader in another language: SPIR-V for Vulkan, as `uint[]` words or a `.spv` file; HLSL for Direct3D, for a shader model from 5.0 to 6.9; the Metal Shading Language for Apple GPUs; GLSL for one entry point; and WGSL. This writes every shader of the folder as SPIR-V and as HLSL into a build folder:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $null = New-Item -ItemType Directory -Path ./build
PS> foreach ($file in Get-ChildItem ./shaders) {
>>     ConvertTo-WgpuShader -LiteralPath $file.FullName -To SpirV -OutFile "./build/$($file.BaseName).spv"
>>     ConvertTo-WgpuShader -LiteralPath $file.FullName -To Hlsl -ShaderModel 6.6 -OutFile "./build/$($file.BaseName).hlsl"
>> }
PS> Get-ChildItem ./build | Sort-Object Name | Select-Object -ExpandProperty Name
double.hlsl
double.spv
ramp.hlsl
ramp.spv
square.hlsl
square.spv
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $null = New-Item -ItemType Directory -Path ./build
PS> foreach ($file in Get-ChildItem ./shaders) {
>>     $shader = [Wgpu.Shader]::FromFile($file.FullName, $null, $null)
>>     $words = [uint32[]]$shader.ConvertTo('SpirV', $null)
>>     $bytes = [byte[]]::new($words.Length * 4)
>>     [Buffer]::BlockCopy($words, 0, $bytes, 0, $bytes.Length)
>>     [IO.File]::WriteAllBytes((Join-Path $PWD "build/$($file.BaseName).spv"), $bytes)
>>     [IO.File]::WriteAllText((Join-Path $PWD "build/$($file.BaseName).hlsl"), $shader.ConvertTo('Hlsl', $null))
>> }
PS> Get-ChildItem ./build | Sort-Object Name | Select-Object -ExpandProperty Name
double.hlsl
double.spv
ramp.hlsl
ramp.spv
square.hlsl
square.spv
```
{{< /tab >}}
{{< /tabs >}}

The SPIR-V files load back like any shader, so the build checks them too:

```powershell
PS> Get-ChildItem ./build/*.spv | Test-WgpuShader
True
True
True

PS> Select-String -Path ./build/double.hlsl -Pattern 'numthreads'
C:\Temp\wgpu-demo\build\double.hlsl:10:[numthreads(64, 1, 1)]
```

SPIR-V is written in Vulkan's clip space, so a vertex shader written to it draws the right way up in a Vulkan program, and [How To Run GLSL And SPIR-V Shaders](How-To-Run-GLSL-And-SPIR-V-Shaders.md) runs SPIR-V and GLSL here.
