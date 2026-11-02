---
title: Backends And Platforms
weight: 4
---

The module reaches a GPU through the graphics APIs its platform has, and compiles every shader for the API it runs on. It is built for seven platforms; [Where It Has Been Run](Where-It-Has-Been-Run.md) says which have been run, and where.

## The platforms and their backends

Without `-Backend`, the module looks for adapters through these:

| Platform | Runtime identifier | Backends by default |
|---|---|---|
| Windows x64 | `win-x64` | Vulkan, Direct3D 12 |
| Windows Arm64 | `win-arm64` | Vulkan, Direct3D 12 |
| Linux x64, glibc 2.35 or later | `linux-x64` | Vulkan, OpenGL |
| Linux Arm64, glibc 2.35 or later | `linux-arm64` | Vulkan, OpenGL |
| macOS on Apple silicon | `osx-arm64` | Metal, Vulkan, OpenGL |
| macOS on Intel | `osx-x64` | Metal, Vulkan, OpenGL |
| FreeBSD x64 | `freebsd-x64` | Vulkan, OpenGL |

A backend is compiled in only where its platform has it: Direct3D 12 on Windows and Metal on macOS. Each finds the system's own libraries when the module looks for adapters, and lists nothing where they are missing: Vulkan needs its loader and a driver, and OpenGL needs EGL on Linux and FreeBSD. On macOS, Vulkan comes through MoltenVK and OpenGL ES through ANGLE, each only where its libraries are installed, since the module carries neither; Metal is always there.

On Windows, OpenGL is left out of the default because NVIDIA's driver deadlocks the process when a thread that used it ends while another thread opens or closes a device; `-Backend Gl` asks for it. [Devices And Instances](Devices-And-Instances.md#nvidias-driver-on-windows) has the details.

## Software adapters

A software adapter runs on the CPU, slowly, and works where no GPU does. On Windows it is WARP, which lists as the Microsoft Basic Render Driver through Direct3D 12; on Linux and FreeBSD, Mesa's lavapipe through Vulkan and llvmpipe through OpenGL, where Mesa is installed. `-Fallback` opens one.

Devices on WARP take turns. Direct3D 12 makes one device per adapter in a process, so every device opened on WARP is a queue of that one device, and WARP crashes the process when two of its queues run work at the same time. Before the module hands WARP one device's work, it waits until WARP has finished the work another device handed it last; work on one device follows that device's earlier work without waiting.

`-Backend Noop` adds wgpu's noop adapter, which accepts all work and runs none of it: a buffer reads back zeros. It is there to try a script's calls on a machine with no adapter at all.

## Shader compilers

Every shader goes through naga, wgpu's shader compiler, which reads WGSL, GLSL and SPIR-V, validates it, and writes it in the language of the backend: SPIR-V for Vulkan, MSL for Metal, GLSL for OpenGL, and HLSL for Direct3D 12. Direct3D 12 compiles HLSL further with DXC, which the module links in on Windows x64 and ships as Microsoft's `dxcompiler.dll` on Windows Arm64. [How To Check And Translate Shaders](How-To-Check-And-Translate-Shaders.md) uses naga with no GPU at all.
