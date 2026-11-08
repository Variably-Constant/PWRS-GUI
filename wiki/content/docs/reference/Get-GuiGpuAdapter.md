---
title: Get-GuiGpuAdapter
weight: 8
---

Gets the GPU adapters the renderer's backends find. Source: `crates/pwrs-gui/src/inspect.rs`, `GetGuiGpuAdapter` and `GpuAdapter`; the renderer's side is `describe` and `backends` in `crates/pwrs-gui-egui/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070 and the Radeon graphics of its AMD Ryzen 9 7900X. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its names for unsigned integer types, `uint32` and `uint64` where PowerShell 7 writes `uint` and `ulong`, except where its output is shown too.

## Syntax

```powershell
Get-Command Get-GuiGpuAdapter -Syntax
```

```text
Get-GuiGpuAdapter [<CommonParameters>]
```

## Description

`Get-GuiGpuAdapter` gets the list from the running renderer, or starts one as [Set-GuiOption](Set-GuiOption.md) says when none is running, and writes one `PwrsGui.GpuAdapter` for every adapter each of its backends finds. One GPU is listed once per backend that reaches it. The backends are:

- on Windows, DirectX 12, Vulkan and OpenGL, OpenGL only where the system has `opengl32.dll`, which Windows Server Core has not, or the Mesa add-on gives it one, whose adapter is `llvmpipe` on the processor ([The Mesa Add-On](The-Mesa-Add-On.md));
- on macOS, Metal;
- on Linux and FreeBSD, Vulkan and OpenGL.

The `PWRS_GUI_BACKEND` environment variable narrows them to those it names. `InUse` marks the adapter and backend the renderer draws with.

Starting a renderer waits for it with no time limit, as every command that opens a window does, and Ctrl+C ends the wait.

## Parameters

`Get-GuiGpuAdapter` takes only the common parameters.

## Inputs

None.

## Outputs

`PwrsGui.GpuAdapter`, one per adapter and backend:

| Property | Type | What it gives |
|---|---|---|
| `Name` | `System.String` | the adapter's name, as its driver reports it |
| `Backend` | `System.String` | `Dx12`, `Vulkan`, `Metal` or `Gl` |
| `DeviceType` | `System.String` | `DiscreteGpu`, `IntegratedGpu`, `VirtualGpu`, `Cpu` or `Other` |
| `Driver` | `System.String` | the driver's name |
| `DriverInfo` | `System.String` | what the driver says of itself, such as its version |
| `Vendor` | `System.UInt32` | the PCI vendor id |
| `Device` | `System.UInt32` | the PCI device id |
| `InUse` | `System.Boolean` | whether the running renderer draws with this adapter through this backend |

```powershell
Get-GuiGpuAdapter | Get-Member -MemberType Properties
```

```text
   TypeName: PwrsGui.GpuAdapter

Name       MemberType Definition
----       ---------- ----------
Backend    Property   string Backend {get;set;}
Device     Property   uint Device {get;set;}
DeviceType Property   string DeviceType {get;set;}
Driver     Property   string Driver {get;set;}
DriverInfo Property   string DriverInfo {get;set;}
InUse      Property   bool InUse {get;set;}
Name       Property   string Name {get;set;}
Vendor     Property   uint Vendor {get;set;}
```

## Examples

The examples from the command's help.

Every adapter, as a list:

<!-- varies: the driver versions -->
```powershell
Get-GuiGpuAdapter
```

```text
Name       : NVIDIA GeForce RTX 5070
Backend    : Vulkan
DeviceType : DiscreteGpu
Driver     : NVIDIA
DriverInfo : 617.42
Vendor     : 4318
Device     : 12036
InUse      : False

Name       : AMD Radeon(TM) Graphics
Backend    : Vulkan
DeviceType : IntegratedGpu
Driver     : AMD proprietary driver
DriverInfo : 2026.04.17 (AMD proprietary shader compiler)
Vendor     : 4098
Device     : 5710
InUse      : False

Name       : NVIDIA GeForce RTX 5070
Backend    : Dx12
DeviceType : DiscreteGpu
Driver     : 32.0.16.1742
DriverInfo :
Vendor     : 4318
Device     : 12036
InUse      : True

Name       : AMD Radeon(TM) Graphics
Backend    : Dx12
DeviceType : IntegratedGpu
Driver     : 32.0.21042.62
DriverInfo :
Vendor     : 4098
Device     : 5710
InUse      : False

Name       : Microsoft Basic Render Driver
Backend    : Dx12
DeviceType : Cpu
Driver     : 10.0.26100.9711
DriverInfo :
Vendor     : 5140
Device     : 140
InUse      : False

Name       : NVIDIA GeForce RTX 5070/PCIe/SSE2
Backend    : Gl
DeviceType : Other
Driver     :
DriverInfo : 4.6.0 NVIDIA 617.42
Vendor     : 4318
Device     : 0
InUse      : False
```

The adapter the renderer draws with:

<!-- varies: the driver versions -->
```powershell
Get-GuiGpuAdapter | Where-Object InUse
```

```text
Name       : NVIDIA GeForce RTX 5070
Backend    : Dx12
DeviceType : DiscreteGpu
Driver     : 32.0.16.1742
DriverInfo :
Vendor     : 4318
Device     : 12036
InUse      : True
```

One line per adapter:

```powershell
Get-GuiGpuAdapter | Format-Table Name, Backend, DeviceType, InUse
```

```text
Name                              Backend DeviceType    InUse
----                              ------- ----------    -----
NVIDIA GeForce RTX 5070           Vulkan  DiscreteGpu   False
AMD Radeon(TM) Graphics           Vulkan  IntegratedGpu False
NVIDIA GeForce RTX 5070           Dx12    DiscreteGpu    True
AMD Radeon(TM) Graphics           Dx12    IntegratedGpu False
Microsoft Basic Render Driver     Dx12    Cpu           False
NVIDIA GeForce RTX 5070/PCIe/SSE2 Gl      Other         False
```

## Errors

A renderer that cannot start fails the command with `GuiStartFailed`, or `GuiNoDisplay` where there is no display; both are terminating, and described in the [Error Reference](Error-Reference.md) with the rest.
