---
title: Shaders
weight: 40
---

naga, the shader compiler wgpu uses, is compiled into the module, so a shader is parsed, validated, read and translated with no GPU at all. It reads WGSL, GLSL and SPIR-V, and writes WGSL, SPIR-V, GLSL, HLSL, the Metal Shading Language and a Graphviz graph of its own form. A shader parsed once is a `Wgpu.Shader`, which belongs to no device: every pipeline made from it compiles it for the pipeline's device. The parameters are on [Shaders](Shader-Cmdlets.md) under Cmdlets and types, and the `Wgpu.Shader` class on [Types](Types.md#wgpushader).

## Check that a shader is valid

`Test-WgpuShader` writes `$true` or `$false`, for WGSL and for GLSL, and with `-Diagnostics` writes each problem with its line and column instead:

```powershell
PS> Test-WgpuShader '@compute @workgroup_size(1) fn main() {}'
True

PS> Test-WgpuShader 'fn main( {'
False

PS> Test-WgpuShader "#version 450`nlayout(local_size_x = 1) in;`nvoid main() {}"
True
```

A shader with a mistake in it. `id.x` is a `u32` and `rate` an `f32`, and WGSL converts neither to the other:

```powershell
PS> Get-Content ./ramp.wgsl
override rate: f32 = 0.1;
@group(0) @binding(0) var<storage, read> base: array<f32>;
@group(0) @binding(1) var<storage, read_write> ramp: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&ramp)) { ramp[id.x] = base[id.x] + id.x * rate; }
}
PS> Test-WgpuShader -Path ./ramp.wgsl
False

PS> Test-WgpuShader -Path ./ramp.wgsl -Diagnostics | Select-Object Severity, Line, Column
Severity Line Column
-------- ---- ------
Error       6     64

PS> $fixed = (Get-Content ./ramp.wgsl -Raw).Replace('id.x * rate', 'f32(id.x) * rate')
PS> Test-WgpuShader $fixed
True
```

`Get-ChildItem` pipes files to every shader cmdlet, `-Path` reads every file its wildcards match and `-LiteralPath` one file named as it is written, and each diagnostic names the file it is in:

```powershell
PS> Set-Content ./fixed.wgsl $fixed
PS> Get-ChildItem ./*.wgsl | Test-WgpuShader
True
False

PS> Test-WgpuShader -Path ./*.wgsl -Diagnostics | Select-Object Label, Line, Column
Label                       Line Column
-----                       ---- ------
C:\Temp\wgpu-demo\ramp.wgsl    6     64
```

A diagnostic carries the line it is on and naga's whole report, which points at the place:

```powershell
PS> Test-WgpuShader "fn main() {`n    let x: f32 = 1u;`n}" -Diagnostics | Format-List
Label      : Wgpu shader
Severity   : Error
Message    : the type of `x` is expected to be `f32`, but got `u32`
Line       : 2
Column     : 9
Length     : 1
SourceLine :     let x: f32 = 1u;
Detail     : error: the type of `x` is expected to be `f32`, but got `u32`
               ┌─ wgsl:2:9
               │
             2 │     let x: f32 = 1u;
               │         ^ definition of `x`
```

naga checks a shader as a device with every feature would. `-Device` also compiles it on a device, against the features that device was opened with, so a shader that needs a feature the device lacks is not valid there:

```powershell
PS> $half = 'enable f16; @compute @workgroup_size(1) fn main() { let h: f16 = 1.0h; }'
PS> Test-WgpuShader $half
True

PS> $plain = Open-WgpuDevice -Feature TimestampQuery
PS> Test-WgpuShader $half -Device $plain
False
```

## Read what a shader declares

A shader with a uniform struct, two bind groups and an override constant:

```powershell
PS> Get-Content ./params.wgsl
struct Params { scale: f32, offset: vec2<f32>, count: u32 }
override gain: f32 = 1.5;
@group(0) @binding(0) var<storage, read_write> data: array<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@group(1) @binding(3) var<storage, read> extra: array<i32>;
@compute @workgroup_size(8, 4, 2)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    data[id.x] = data[id.x] * p.scale * gain + f32(extra[0]) + p.offset.x + f32(p.count);
}
```

`Get-WgpuShaderInfo`, and the `Info` method of a shader, read its entry points, its bindings and its override constants, which is what `Invoke-WgpuCompute` binds resources by; `-Shader` gives the cmdlet a shader `New-WgpuShader` parsed already. A .NET method resolves a relative path against the process's working directory, which PowerShell's location does not change, so the method takes the file's full path:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $info = Get-WgpuShaderInfo -Path ./params.wgsl
PS> $info
Label                         Language EntryPoints Bindings         Overrides
-----                         -------- ----------- --------         ---------
C:\Temp\wgpu-demo\params.wgsl Wgsl     {main}      {data, p, extra} {gain}

PS> (Get-WgpuShaderInfo -Shader (New-WgpuShader -Path ./params.wgsl)).Overrides.Name
gain
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $info = [Wgpu.Shader]::FromFile((Join-Path $PWD params.wgsl)).Info()
PS> $info
Label                         Language EntryPoints Bindings         Overrides
-----                         -------- ----------- --------         ---------
C:\Temp\wgpu-demo\params.wgsl Wgsl     {main}      {data, p, extra} {gain}
```
{{< /tab >}}
{{< /tabs >}}

Each entry point with its stage and workgroup size, each binding with its kind, its WGSL type, the element type an array of it holds and the size of a struct in bytes, and each override with its default:

```powershell
PS> $info.EntryPoints
Name Stage   WorkgroupSize
---- -----   -------------
main Compute {8, 4, 2}

PS> $info.Bindings | Select-Object Group, Binding, Name, Kind, WgslType, ElementType, Size
Group       : 0
Binding     : 0
Name        : data
Kind        : StorageBuffer
WgslType    : array<f32>
ElementType : Float32
Size        : 0

Group       : 0
Binding     : 1
Name        : p
Kind        : UniformBuffer
WgslType    : Params
ElementType :
Size        : 24

Group       : 1
Binding     : 3
Name        : extra
Kind        : ReadOnlyStorageBuffer
WgslType    : array<i32>
ElementType : Int32
Size        : 0

PS> $info.Overrides
Name Id WgslType Default
---- -- -------- -------
gain    f32          1.5
```

An override declared with an `@id` is known by that number as well as its name, and one with no default must be given a value before the shader runs:

```powershell
PS> (Get-WgpuShaderInfo 'override gain: f32 = 1.5; @id(7) override bias: f32; @compute @workgroup_size(1) fn main() { _ = gain + bias; }').Overrides
Name Id WgslType Default
---- -- -------- -------
gain    f32          1.5
bias  7 f32
```

A GLSL buffer block with no instance name is named for its set and binding, so it binds by that name, by position, or as `0:0`:

```powershell
PS> (Get-WgpuShaderInfo "#version 450`nlayout(local_size_x = 32) in;`nlayout(set = 0, binding = 0) buffer Data { float values[]; };`nvoid main() {}").Bindings | Select-Object Name, Kind, ElementType
Name                Kind ElementType
----                ---- -----------
binding0_0 StorageBuffer     Float32
```

## Translate a shader

`ConvertTo-WgpuShader`, and the `ConvertTo` method of a shader, write it in another language. HLSL is for Direct3D, at shader model 6.0 unless `-ShaderModel` names another from 5.0 to 6.9:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> ConvertTo-WgpuShader $fixed -To Hlsl
static const float rate = 0.1;

ByteAddressBuffer base : register(t0);
RWByteAddressBuffer ramp : register(u1);

uint NagaBufferLengthRW(RWByteAddressBuffer buffer)
{
    uint ret;
    buffer.GetDimensions(ret);
    return ret;
}

[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID)
{
    if ((id.x < ((NagaBufferLengthRW(ramp) - 0) / 4))) {
        float _e11 = asfloat(base.Load(id.x*4));
        ramp.Store(id.x*4, asuint((_e11 + (float(id.x) * rate))));
        return;
    } else {
        return;
    }
}

PS> ConvertTo-WgpuShader $fixed -To Hlsl -ShaderModel 9.9
ConvertTo-WgpuShader: HLSL has no shader model 9.9 naga writes; it writes 5.0 to 6.9
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> [Wgpu.Shader]::new($fixed).ConvertTo('Hlsl') -split "`n" | Select-Object -First 4
static const float rate = 0.1;

ByteAddressBuffer base : register(t0);
RWByteAddressBuffer ramp : register(u1);
```
{{< /tab >}}
{{< /tabs >}}

GLSL is written for one entry point, as desktop GLSL 450 or, with `-GlslEs`, OpenGL ES 310, and `-GlslVersion` names another version. An override constant is fixed to its default in a language that has none:

```powershell
PS> (ConvertTo-WgpuShader $fixed -To Glsl -GlslEs) -split "`n" | Select-Object -First 4
#version 310 es

precision highp float;
precision highp int;

PS> (ConvertTo-WgpuShader $fixed -To Glsl -GlslVersion 430) -split "`n" | Select-Object -First 3
#version 430 core
#extension GL_ARB_compute_shader : require
#extension GL_ARB_shader_storage_buffer_object : require
```

SPIR-V comes out as `uint[]` words, starting with its magic number, or with `-OutFile` as a binary file, which reads back like any other shader. It carries the names of the shader's bindings, structs and members, as glslc writes them, so a shader read back from it binds by the same names:

```powershell
PS> $words = ConvertTo-WgpuShader $fixed -To SpirV
PS> $words.GetType().Name
UInt32[]

PS> '0x{0:X8}' -f $words[0]
0x07230203

PS> ConvertTo-WgpuShader $fixed -To SpirV -OutFile ./ramp.spv
PS> (Get-Item ./ramp.spv).Length -eq $words.Count * 4
True

PS> Test-WgpuShader -Path ./ramp.spv
True
```

SPIR-V is written and read in Vulkan's clip space, whose y axis points down where WebGPU's points up. A vertex shader's position is flipped as it is written and flipped back as it is read, so SPIR-V written here draws as the shader it came from does, and SPIR-V from glslc, glslangValidator or `dxc -spirv`, written for Vulkan, draws here as it does there. A compute shader has no position, so nothing in it changes.

The Metal Shading Language is for Apple GPUs, version 2.4 unless `-MslVersion` names another, and `Dot` is a Graphviz graph of naga's intermediate form; the first lines of each:

```powershell
PS> (ConvertTo-WgpuShader $fixed -To Msl) -split "`n" | Select-Object -First 1
// language: metal2.4

PS> (ConvertTo-WgpuShader $fixed -To Msl -MslVersion 3.0) -split "`n" | Select-Object -First 1
// language: metal3.0

PS> (ConvertTo-WgpuShader $fixed -To Dot) -split "`n" | Select-Object -First 3
digraph Module {
	subgraph cluster_globals {
		label="Globals"
```

Each result carries a `Label` note naming the shader it came from, the file's path for a file, so the translations of several files stay apart. `-OutFile` holds one shader, so it refuses a wildcard that matches more than one file:

```powershell
PS> Get-ChildItem ./fixed.wgsl, ./ramp.spv | ConvertTo-WgpuShader -To Wgsl | ForEach-Object Label
C:\Temp\wgpu-demo\fixed.wgsl
C:\Temp\wgpu-demo\ramp.spv

PS> ConvertTo-WgpuShader -Path ./*.wgsl -To Hlsl -OutFile ./both.hlsl
ConvertTo-WgpuShader: -OutFile holds one shader, and ./*.wgsl matches 3 files: C:\Temp\wgpu-demo\fixed.wgsl, C:\Temp\wgpu-demo\params.wgsl, C:\Temp\wgpu-demo\ramp.wgsl
```

## Bring in GLSL and SPIR-V

GLSL is read by its `#version` line or by `-Language Glsl`, as the stage `-Stage` names, compute when none is given; a `.vert`, `.frag` or `.comp` file names its stage itself. `-Define` gives the preprocessor's definitions:

```powershell
PS> Get-Content ./tint.frag
#version 450
layout(location = 0) out vec4 color;
void main() { color = vec4(TINT, 1.0); }
PS> ConvertTo-WgpuShader -Path ./tint.frag -Define @{ TINT = 'vec3(1.0, 0.5, 0.0)' } -To Wgsl
struct FragmentOutput {
    @location(0) color: vec4<f32>,
}

var<private> color: vec4<f32>;

fn main_1() {
    color = vec4<f32>(1f, 0.5f, 0f, 1f);
    return;
}

@fragment
fn main() -> FragmentOutput {
    main_1();
    let _e3 = color;
    return FragmentOutput(_e3);
}
```

SPIR-V is read from a `.spv` file, or from its words, in Vulkan's clip space, as [Translate a shader](#translate-a-shader) says. A shader read from SPIR-V keeps no source text. The WGSL written from it takes the names the SPIR-V carries, and shows the struct of one member that naga's SPIR-V writer wraps each buffer in; binding a resource and `Get-WgpuShaderInfo` look through that struct to the member it holds:

```powershell
PS> ConvertTo-WgpuShader -Path ./ramp.spv -To Wgsl
struct type_4 {
    member: array<f32>,
}

const rate: f32 = 0.1f;

@group(0) @binding(0)
var<storage> base: type_4;
@group(0) @binding(1)
var<storage, read_write> ramp: type_4;
var<private> id_1: vec3<u32>;

fn main_1() {
    let _e5 = id_1;
    if (_e5.x < arrayLength((&ramp.member))) {
        let _e15 = base.member[_e5.x];
        ramp.member[_e5.x] = (_e15 + (f32(_e5.x) * rate));
        return;
    } else {
        return;
    }
    return;
}

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    id_1 = id;
    main_1();
}
```

## Parse a shader once

`New-WgpuShader` parses and validates a shader into a `Wgpu.Shader`, which every shader cmdlet and every pipeline takes in place of text. The class's constructor takes text, its `FromFile` method a file and its `FromSpirV` method SPIR-V words; the device's `CreateShader` method is the constructor by another name.

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $shader = New-WgpuShader $fixed -Label ramp
PS> $shader
Label Language EntryPoints
----- -------- -----------
ramp  Wgsl     {main}

PS> (New-WgpuShader -Path ./params.wgsl).EntryPoints.Name
main

PS> Set-Content -LiteralPath './params[1].wgsl' (Get-Content ./params.wgsl -Raw)
PS> (New-WgpuShader -LiteralPath './params[1].wgsl').Label
C:\Temp\wgpu-demo\params[1].wgsl

PS> $spirv = New-WgpuShader -SpirV $words
PS> $spirv.Language
SpirV

PS> $null -eq $spirv.Source
True
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $shader = [Wgpu.Shader]::new($fixed)
PS> $shader
Label       Language EntryPoints
-----       -------- -----------
Wgpu shader Wgsl     {main}

PS> [Wgpu.Shader]::FromFile((Join-Path $PWD params.wgsl)).EntryPoints.Name
main

PS> [Wgpu.Device]::GetDefault().CreateShader($fixed).Label
Wgpu shader

PS> $spirv = [Wgpu.Shader]::FromSpirV($words)
PS> $spirv.Language
SpirV

PS> $null -eq $spirv.Source
True
```
{{< /tab >}}
{{< /tabs >}}

Since a shader belongs to no device, one shader runs on any device:

```powershell
PS> Get-Content ./double.wgsl
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
PS> $doubler = New-WgpuShader -Path ./double.wgsl
PS> Invoke-WgpuCompute $doubler ([float[]](1, 2))
2
4

PS> Invoke-WgpuCompute $doubler ([float[]](3, 4)) -Device (Open-WgpuDevice -Fallback)
6
8
```

GLSL from a file takes its definitions the same way:

```powershell
PS> Get-Content ./scale.comp
#version 450
layout(local_size_x = 32) in;
layout(set = 0, binding = 0) buffer Data { float values[]; };
void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i < values.length()) { values[i] *= SCALE; }
}
PS> $glsl = New-WgpuShader -Path ./scale.comp -Define @{ SCALE = '3.0' }
PS> $glsl.Language
Glsl

PS> Invoke-WgpuCompute $glsl ([float[]](1, 2))
3
6
```

A shader that does not validate is refused with its line and column, and a file that is not there with PowerShell's reason:

```powershell
PS> New-WgpuShader "@compute @workgroup_size(1) fn main() {`n    let x: f32 = 1u;`n}"
New-WgpuShader: the shader is invalid: line 2, column 9: the type of `x` is expected to be `f32`, but got `u32`

PS> New-WgpuShader -Path ./missing.wgsl
New-WgpuShader: Cannot find path 'C:\Temp\wgpu-demo\missing.wgsl' because it does not exist.
```
