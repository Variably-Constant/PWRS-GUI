---
title: Why Wgpu
weight: 1
---

Wgpu puts the GPU behind PowerShell cmdlets: a script hands its arrays to a shader of its own, keeps buffers and textures on the GPU between calls, draws into textures and image files, and checks and translates shaders with no GPU at all. This squares an array on the machine's default adapter:

```powershell
PS> (Invoke-WgpuMap 'x * x' ([float[]](1..8))) -join ','
1,4,9,16,25,36,49,64
```

## Why wgpu

[wgpu](https://wgpu.rs/) is the Rust implementation of WebGPU, the API browsers give web pages for the GPU; Firefox's WebGPU runs on it. It is one API over Direct3D 12, Vulkan, Metal and OpenGL, so a script's shaders and calls are the same through each of them, on Windows, Linux, macOS and FreeBSD. Its shaders are written in WGSL, WebGPU's shading language, and naga, its shader compiler, reads GLSL and SPIR-V too, and writes WGSL, GLSL, SPIR-V, HLSL and MSL.

wgpu validates the calls it is given, so a mistake comes back as an error with a message where a driver could fault or misbehave. Reading a buffer after freeing it, which Vulkan and Direct3D 12 leave undefined, is refused, and the message names the buffer:

```powershell
PS> $buffer = New-WgpuBuffer ([float[]](1..4))
PS> Remove-WgpuBuffer $buffer
PS> Read-WgpuBuffer $buffer
Read-WgpuBuffer: the buffer cannot be read back: Validation Error

Caused by:
  In Queue::submit
    Buffer with '4 x Float32' label has been destroyed
```

## What the module adds

- wgpu and naga compiled into the module, with DXC, Direct3D 12's shader compiler, compiled in on Windows x64 and carried beside the library on Windows Arm64, so no Vulkan SDK, DXC or Visual C++ Redistributable needs installing.
- Objects that stay on the GPU between calls. Buffers, textures, shaders and pipelines are objects a script holds, and data moves only when it is read or written; [How Data Moves](How-Data-Moves.md) says where and what that costs.
- Arrays of any numeric type in and out: an array binds to a shader's buffer as the element type the shader declares.
- Every limit the adapter has, and every feature but wgpu's experimental ones, on a device opened without `-Feature`; `-Experimental` adds those.
- Shader tools with no GPU: Test-WgpuShader, Get-WgpuShaderInfo and ConvertTo-WgpuShader run with no GPU at all.
- Cmdlets in pipelines, methods on the objects, tasks, both hosts, runspaces and threads.
- The Rust library under the cmdlets, for Rust programs.

## What it does not do

- It draws offscreen, into textures and image files. It opens no window and presents nothing on a screen.
- It can use only what wgpu offers: wgpu's native features beyond WebGPU's when the adapter supports them, and the experimental ones with `-Experimental`. A vendor's own libraries, such as CUDA, are not part of it.
- A GPU is fast at work over many elements and slow to reach: each call that runs GPU work crosses to the GPU and back, so small arrays are often quicker on the CPU. [How Data Moves](How-Data-Moves.md) measures where the time goes.
