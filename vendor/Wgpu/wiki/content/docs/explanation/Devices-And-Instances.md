---
title: Devices And Instances
weight: 2
---

To reach a GPU, the module makes an instance, chooses one of its adapters, and opens a device on that adapter.

## Instances and adapters

An instance is wgpu's view of the graphics APIs it was asked to use, and an adapter is one GPU, or one software renderer, as one of those APIs sees it: the same GPU is an adapter through Vulkan and another through Direct3D 12.

```powershell
PS> Get-WgpuAdapter
Name                          Backend DeviceType    Driver
----                          ------- ----------    ------
NVIDIA GeForce RTX 5070       Vulkan  DiscreteGpu   NVIDIA
AMD Radeon(TM) Graphics       Vulkan  IntegratedGpu AMD proprietary driver
NVIDIA GeForce RTX 5070       Dx12    DiscreteGpu   32.0.16.1742
AMD Radeon(TM) Graphics       Dx12    IntegratedGpu 32.0.21042.62
Microsoft Basic Render Driver Dx12    Cpu           10.0.26100.9711
```

The module makes an instance the first time anything needs adapters through a set of backends, with or without validation, and keeps it until no runspace of the process has the module. Closing a device, or a runspace that imported the module, therefore never ends the threads an instance runs, the OpenGL backend's window thread among them.

Each instance lists its adapters once, on the thread that makes it and before any other thread can reach it, because NVIDIA's Vulkan driver faults when threads query one instance's adapters at the same time. Every later call chooses from that list, so an adapter attached afterward, an external GPU for one, appears once every runspace has removed the module and it is imported again.

Without `-Backend`, the module looks through Vulkan and Direct3D 12 on Windows, and through every backend the platform has elsewhere; [Backends And Platforms](Backends-And-Platforms.md) has the list.

## Devices

A device is an adapter opened for work, with the queue that work is submitted on. The module opens every device with every feature its adapter supports, apart from wgpu's experimental ones, which `-Experimental` adds, and with every limit at the adapter's own maximum, so no shader is refused for a limit the hardware does not have. A buffer, texture or pipeline belongs to the device it was made on, and binding it on another is refused with WgpuOtherDevice.

Each device has a thread of its own that submits its work to the GPU, in the order it was given. A call that waits for its work waits for that thread, then for the GPU, in steps of 100 ms, so Ctrl+C and a canceled task's token end the wait promptly even where a driver's submit waits for earlier work to finish, as Mesa's software Vulkan's does. An OpenGL device submits on the thread that asks, for the reason below.

## The default device

Cmdlets given no `-Device` use the default device. It is opened the first time one needs it, on the adapter wgpu prefers, the high-performance GPU where there is a choice, through the default backends. There is one for the whole process, shared by every runspace in it. When it is lost, the next call that needs it opens a new one; `Open-WgpuDevice -Default` and a device's `SetDefault()` replace it, and removing the module releases it.

## How a device ends

A device lives as long as any object made on it does. Close-WgpuDevice and a device's `Destroy()` end it at once: the work already given to it finishes, then it reports itself lost, raises the `Wgpu.DeviceLost` engine event, and refuses every later call with WgpuDeviceLost. A driver that resets the GPU loses its devices the same way. [How To Handle Errors](How-To-Handle-Errors.md) shows a script finding out and going on.

## NVIDIA's driver on Windows

Three things in the module are there for NVIDIA's Windows driver, 617.42 on the machine these pages ran on:

- OpenGL is left out of the default backends on Windows. The driver deadlocks the process when a thread that used OpenGL ends while another thread opens or closes a device, and wgpu uses OpenGL on every thread that looks for OpenGL adapters. For the same reason an OpenGL device has no submission thread, which would end with the device.
- Each instance lists its adapters once, as above, since the driver faults on concurrent queries.
- Before making its first instance, the module loads `psapi.dll` and `dbghelp.dll` from System32 and keeps them loaded. The driver loads and frees both each time it makes an instance or a device, while holding a lock of its own; with them already loaded, its loads return at once instead of waiting for the loader, which a thread that is starting or ending holds.
