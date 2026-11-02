---
title: How To Run GLSL And SPIR-V Shaders
weight: 5
---

Run shaders written for Vulkan or OpenGL as they are: GLSL compute shaders from their source, with the preprocessor's definitions, and SPIR-V, the compiled form Vulkan's tools write. naga, the shader compiler inside the module, reads both into the same form it reads WGSL into, so they run on every backend, Direct3D 12 and Metal included.

## Run a GLSL compute shader

This shader works out `a * x + y` over two arrays, a step of many linear algebra routines. Its workgroup size is a preprocessor definition, `SIZE`, so a script can tune it for a GPU:

```powershell
PS> Get-Content ./saxpy.comp
#version 450
layout(local_size_x = SIZE) in;
layout(set = 0, binding = 0) readonly buffer Xs { float x[]; };
layout(set = 0, binding = 1) buffer Ys { float y[]; };
layout(set = 0, binding = 2) uniform Params { float a; };
void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i < y.length()) { y[i] = a * x[i] + y[i]; }
}
```

The `.comp` extension says it is a GLSL compute shader, and `-Define` gives `SIZE` its value. The shader parsed once runs as often as needed:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $saxpy = New-WgpuShader -Path ./saxpy.comp -Define @{ SIZE = '64' }
PS> $saxpy | Select-Object Language, @{ n = 'WorkgroupSize'; e = { $_.EntryPoints[0].WorkgroupSize -join ',' } }
Language WorkgroupSize
-------- -------------
    Glsl 64,1,1
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $source = (Get-Content ./saxpy.comp -Raw).Replace('local_size_x = SIZE', 'local_size_x = 64')
PS> $saxpy = [Wgpu.Shader]::new($source, 'Glsl', 'Compute')
PS> $saxpy | Select-Object Language, @{ n = 'WorkgroupSize'; e = { $_.EntryPoints[0].WorkgroupSize -join ',' } }
Language WorkgroupSize
-------- -------------
    Glsl 64,1,1
```

The constructor takes no definitions, so this replaces `SIZE` in the text first.
{{< /tab >}}
{{< /tabs >}}

The blocks bind in their binding order, the uniform block's member by name, and the array the shader writes comes back:

```powershell
PS> Invoke-WgpuCompute $saxpy @([float[]](1, 2, 3), [float[]](10, 20, 30), @{ a = 2 })
12
24
36
```

## Read what the module made of it

GLSL names its blocks, where WGSL names its variables. `Get-WgpuShaderInfo` shows the bindings as the module sees them, which is how a hashtable of resources names them:

```powershell
PS> (Get-WgpuShaderInfo $saxpy).Bindings | Select-Object Group, Binding, Name, Kind, WgslType
Group    : 0
Binding  : 0
Name     : binding0_0
Kind     : ReadOnlyStorageBuffer
WgslType : Xs

Group    : 0
Binding  : 1
Name     : binding0_1
Kind     : StorageBuffer
WgslType : Ys

Group    : 0
Binding  : 2
Name     : binding0_2
Kind     : UniformBuffer
WgslType : Params
```

`ConvertTo-WgpuShader -To Wgsl` writes the shader as WGSL, a start for moving it over for good; [How To Check And Translate Shaders](How-To-Check-And-Translate-Shaders.md) checks and translates whole folders of them.

## Run SPIR-V

SPIR-V is what glslc, glslangValidator and `dxc -spirv` write for Vulkan. Here the module writes it from the GLSL, as those tools would, into a `.spv` file, which then loads like any shader file:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> ConvertTo-WgpuShader $saxpy -To SpirV -OutFile ./saxpy.spv
PS> $compiled = New-WgpuShader -Path ./saxpy.spv
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> ConvertTo-WgpuShader $saxpy -To SpirV -OutFile ./saxpy.spv
PS> $compiled = [Wgpu.Shader]::FromSpirV([System.IO.File]::ReadAllBytes((Join-Path $PWD saxpy.spv)))
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> $compiled.Language
SpirV

PS> Invoke-WgpuCompute $compiled @([float[]](1, 2, 3), [float[]](10, 20, 30), @{ a = 2 })
12
24
36
```

The SPIR-V carries the shader's names, as glslc's does, so the resources that ran the GLSL run it too. It keeps no source text, so a mistake in it is reported without a line and column. glslc's `-O` and glslangValidator's `-g0` write SPIR-V without the names, and a hashtable that names a block's members is refused for it.

## Draw with SPIR-V

A render pipeline takes one shader with both a vertex and a fragment entry point. A GLSL file holds a single stage, so drawing takes WGSL or SPIR-V. A SPIR-V module may hold both stages, as this one, written from a WGSL triangle, does; glslc and `dxc -spirv` write a vertex and a fragment shader to files of their own, and `spirv-link`, from SPIRV-Tools, joins the two into one module that draws. SPIR-V is written and read in Vulkan's clip space, whose y axis points down where WebGPU's points up, so the module flips a vertex shader's position as it writes SPIR-V and flips it back as it reads it: the triangle draws the same pixels either way, and SPIR-V written for Vulkan by glslc or `dxc -spirv` draws as it does in Vulkan:

```powershell
PS> Get-Content ./triangle.wgsl
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    var corners = array<vec2<f32>, 3>(vec2<f32>(0.0, 0.8), vec2<f32>(-0.8, -0.8), vec2<f32>(0.8, -0.8));
    var colors = array<vec3<f32>, 3>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0));
    var out: Out;
    out.position = vec4<f32>(corners[i], 0.0, 1.0);
    out.color = colors[i];
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
PS> ConvertTo-WgpuShader -Path ./triangle.wgsl -To SpirV -OutFile ./triangle.spv
PS> $both = New-WgpuShader -Path ./triangle.spv
PS> $both.EntryPoints | Select-Object Name, Stage
Name    Stage
----    -----
vs     Vertex
fs   Fragment

PS> $fromSpirV = Read-WgpuTexture (Invoke-WgpuRender $both 64 64 -VertexCount 3)
PS> $fromWgsl = Read-WgpuTexture (Invoke-WgpuRender (Get-Content ./triangle.wgsl -Raw) 64 64 -VertexCount 3)
PS> [Linq.Enumerable]::SequenceEqual([byte[]]$fromSpirV, [byte[]]$fromWgsl)
True
```

[Bring in GLSL and SPIR-V](Shaders.md#bring-in-glsl-and-spir-v) has the rest: GLSL stages by file extension, `-Language` and `-Stage` for text, and SPIR-V from its words.
