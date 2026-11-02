---
title: Wgpu Wiki
toc: false
---

<p align="center">
  <em>GPU compute, rendering and shader tools for PowerShell, through wgpu compiled into the module, on seven platforms.</em>
</p>

---

Wgpu puts the GPU behind cmdlets, in PowerShell 7.4 or later and in Windows PowerShell 5.1. It carries wgpu 30, which reaches the GPU through Direct3D 12, Vulkan, Metal or OpenGL, on Windows x64 and Arm64, Linux x64 and Arm64, macOS on Apple silicon and on Intel, and FreeBSD x64. Buffers and textures stay on the GPU between calls, compute shaders run over arrays and buffers, `Invoke-WgpuRender` draws into textures and image files, and the shader cmdlets check and translate shaders with no GPU at all.

```powershell
Install-Module Wgpu -Scope CurrentUser
```

## What it looks like

An expression mapped over an array, on the machine's default adapter:

```powershell
PS> (Invoke-WgpuMap 'x * x' ([float[]](1..8))) -join ','
1,4,9,16,25,36,49,64
```

A compute shader of your own, given its array by the name it declares:

```powershell
PS> Get-Content ./double.wgsl
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
PS> (Invoke-WgpuCompute (Get-Content ./double.wgsl -Raw) @{ v = [float[]](1..8) }) -join ','
2,4,6,8,10,12,14,16
```

A triangle drawn into a PNG, its corners placed and colored by a vertex shader:

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
PS> Invoke-WgpuRender (Get-Content ./triangle.wgsl -Raw) 256 256 -VertexCount 3 -OutFile ./triangle.png
```

![The triangle Invoke-WgpuRender drew: red at the top, green at the bottom left, blue at the bottom right, blended between them, on black](/images/triangle.png)

## Data stays on the GPU

A buffer or a texture is an object a script holds, and it stays on the GPU between calls, so a run over it copies nothing. On the machine these pages ran on, doubling 32 MiB of floats took about 10 ms from an array, copied in and back, and half a millisecond over a buffer already on the GPU. [How Data Moves](How-Data-Moves.md) has the measurements, and [How To Work With Buffers](How-To-Work-With-Buffers.md) runs a simulation that keeps its data there.

## Shaders without a GPU

naga, wgpu's shader compiler, is compiled into the module, so `Test-WgpuShader`, `Get-WgpuShaderInfo` and `ConvertTo-WgpuShader` check, read and translate WGSL, GLSL and SPIR-V with no GPU at all:

```powershell
PS> Test-WgpuShader (Get-Content ./double.wgsl -Raw)
True

PS> Test-WgpuShader 'fn main( {'
False
```

[Shaders Without A GPU](Shaders-Without-A-GPU.md) reads a shader's mistakes, what it declares, and its translations.

## Where to start

- To learn the module from the beginning, the [tutorials](docs/tutorials/) go a chapter at a time from installing it to compute shaders, textures, drawing, commands and tasks.
- For a task in hand, the [how-tos](docs/how-to/) are recipes, from choosing an adapter to preparing textures for a renderer.
- To look something up, the [reference](docs/reference/) has every task with its examples, every cmdlet's syntax and parameters, the classes and their methods, and every error id; the [Error Reference](Error-Reference.md) finds an error by its id.
- To understand how it works, the [explanation](docs/explanation/) pages cover what the module adds to PowerShell, devices, how data moves, and the graphics APIs on each platform.
- To build or change the module, [Building From Source](docs/building/) has the build, the tests and the tools behind these pages.

## Where the output on these pages comes from

Every example on these pages was run on Windows 11 Pro 10.0.26200 on an AMD Ryzen 9 7900X, with an NVIDIA GeForce RTX 5070 and the processor's AMD Radeon graphics, in PowerShell 7.6.7 and in Windows PowerShell 5.1.26100.9711, against the module `cargo pwrs build --release` built from the code these pages describe, with PoWerRuSt 0.8.0 and cargo-pwrs 0.8.0. The output shown is PowerShell 7's, which the tools in the repository write into each page from a run of it. Windows PowerShell ran every example but those marked for PowerShell 7, and none ended in an error its page does not show; it formats tables and errors in its own way. [Where It Has Been Run](Where-It-Has-Been-Run.md) lists every machine, host and build.

## License

MIT, in [LICENSE](https://github.com/Variably-Constant/Wgpu/blob/main/LICENSE) on the repository.
