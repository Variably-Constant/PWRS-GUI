---
title: Invoke-GuiGpuCompute
weight: 9
---

Runs a compute shader once in the egui renderer over arrays or GPU buffers, as the Wgpu module's `Invoke-WgpuCompute` does. Source: `crates/pwrs-gui/src/gpu.rs`, `InvokeGuiGpuCompute` and `GpuComputeResult`; the renderer's side is `crates/pwrs-gui-egui/src/gpu_thread.rs` and `compute` in `crates/pwrs-gui-gpu/src/lib.rs`, which runs the shader through the Wgpu module's library.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its names for unsigned integer types, `uint32` and `uint64` where PowerShell 7 writes `uint` and `ulong`, except where its output is shown too.

## Syntax

```powershell
Get-Command Invoke-GuiGpuCompute -Syntax
```

```text
Invoke-GuiGpuCompute [-Shader] <string> [[-Binding] <Object>] [-Workgroups <uint[]>] [-Invocations <ulong[]>] [-Indirect <GpuBuffer>] [-IndirectOffset <ulong>] [-Entry <string>] [-Constant <IDictionary>] [-Immediate <Object>] [-Language <ShaderLanguage>] [-PassThru] [<CommonParameters>]
```

## Description

`Invoke-GuiGpuCompute` compiles `-Shader`, WGSL or GLSL text, on the device of the session's egui renderer, binds its resources, dispatches it, and waits for the GPU. The shader is validated before it runs, and an error in it names its line and column. Runs go through the renderer's GPU thread one at a time, so a long run never stalls a window's frames.

`-Binding` gives the shader's resources in one of three forms, as `Invoke-WgpuCompute` takes them:

| Form | Bound |
|---|---|
| an array of resources | one per binding, in the order the shader declares them, by group and then binding |
| a hashtable | each key a binding's variable name, without regard to case, or `group:binding` |
| one resource | the shader's only binding |

A resource is a `PwrsGui.GpuBuffer` from [New-GuiGpuBuffer](New-GuiGpuBuffer.md), which stays on the GPU, or an array of numbers, copied to the GPU as the element type the shader declares for the binding and converted as the Wgpu module converts values. Each array bound `read_write` comes back: the command writes one array per such binding, in binding order, of its declared type. A buffer the shader writes has the result on the GPU, for [Read-GuiGpuBuffer](Read-GuiGpuBuffer.md) or the next run.

The run dispatches `-Workgroups` in x, y and z; or `-Invocations`, divided by the shader's workgroup size and rounded up; or the three `uint` counts the GPU reads from `-Indirect` at `-IndirectOffset`; or, with none of them, one invocation per element of the first buffer bound, in x. `-Constant` overrides the shader's override constants by name or `@id`, or a GLSL shader's specialization constants by `constant_id`, and `-Immediate` gives the entry point's immediate data.

`-PassThru` writes one `PwrsGui.ComputeResult` instead of the arrays, with every output by binding name and the time the run took, on the GPU where the device has timestamp queries.

## Parameters

### -Shader

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiGpuNoShader` when empty, and the Wgpu module's shader errors for one that does not validate |

WGSL or GLSL text.

### -Binding

| | |
|---|---|
| Type | `System.Object` |
| Aliases | `Data`, `Buffer`, `Resource` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | none: the shader takes no resources |
| Refused with | `GuiGpuBinding` for a name or `group:binding` the shader does not use, `GuiGpuBindingCount` for an array of another count than the shader's bindings, `GuiGpuStruct` for a hashtable given as one binding's value, `GuiGpuEmpty` for an empty array, `GuiGpuOtherSession` for a buffer of a renderer that has since ended |

The resources for the shader's bindings.

### -Workgroups

| | |
|---|---|
| Type | `System.UInt32[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiGpuDispatchSize` for more than three counts, or beside `-Invocations` or `-Indirect` |

Workgroups to dispatch in x, y and z, 1 for a dimension left out.

### -Invocations

| | |
|---|---|
| Type | `System.UInt64[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Invocations in x, y and z, divided by the shader's workgroup size and rounded up.

### -Indirect

| | |
|---|---|
| Type | `PwrsGui.GpuBuffer` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A buffer the GPU reads the workgroup counts from, as three `uint`.

### -IndirectOffset

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0 |

The byte offset of the counts in `-Indirect`.

### -Entry

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the shader's only compute entry point |
| Refused with | `GuiGpuEntry` for a GLSL shader's entry point other than `main` |

The entry point.

### -Constant

| | |
|---|---|
| Type | `System.Collections.IDictionary` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Override constant values, by name or `@id`; a GLSL shader's specialization constants by `constant_id`.

### -Immediate

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Immediate data, as an array of numbers.

### -Language

| | |
|---|---|
| Type | `PwrsGui.ShaderLanguage`: `Wgsl`, `Glsl` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | GLSL when the text starts with `#version`, WGSL otherwise |

The language of the shader text.

### -PassThru

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the arrays alone |

Write a `PwrsGui.ComputeResult` with every output and the time taken.

## Inputs

None.

## Outputs

One array of the declared type for each array bound `read_write`, in binding order; nothing when every binding the shader writes is a buffer. With `-PassThru`, `PwrsGui.ComputeResult`:

| Property | Type | What it gives |
|---|---|---|
| `Outputs` | `System.Collections.Hashtable` | each array the shader may have written, by binding name |
| `Workgroups` | `System.UInt32[]` | the workgroups dispatched in x, y and z; 0 for an indirect dispatch |
| `GpuTimeNs` | `System.Double` | nanoseconds the GPU spent in the pass, where the device has timestamp queries |
| `CpuTimeMs` | `System.Double` | milliseconds the run took in the renderer, binding and copying included |

## Examples

The examples from the command's help, with the shaders they name. These are the shaders:

```powershell
$double = @'
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
'@
$add = @'
@group(0) @binding(0) var<storage, read_write> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&a)) { a[id.x] = a[id.x] + b[id.x]; }
}
'@
$wgsl = @'
@group(0) @binding(0) var<storage, read_write> grid: array<f32>;
@compute @workgroup_size(8, 8) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    grid[id.y * 512u + id.x] = f32(id.x + id.y);
}
'@
```

An array doubled; the result is a new array of the shader's type, shown in part:

```powershell
$doubled = Invoke-GuiGpuCompute $double ([float[]](1..1024))
$doubled.GetType().Name
$doubled[0..3]
```

```text
Single[]
2
4
6
8
```

A buffer added to in place, by binding name; a buffer stays on the GPU, so the run writes nothing and the buffer is read after:

```powershell
$a = New-GuiGpuBuffer ([float[]](10..19))
Invoke-GuiGpuCompute $add @{ a = $a; b = [float[]](1..10) } -Invocations 10
Read-GuiGpuBuffer $a
```

```text
11
13
15
17
19
21
23
25
27
29
```

A grid of 512 by 512 filled in workgroups of 8 by 8:

```powershell
$grid = New-GuiGpuBuffer -Length (512 * 512)
Invoke-GuiGpuCompute $wgsl -Binding $grid -Workgroups 64, 64
Read-GuiGpuBuffer $grid -Offset 513 -Count 3
```

```text
2
3
4
```

The run's result, with the time it took:

<!-- varies: the times -->
```powershell
Invoke-GuiGpuCompute $double ([float[]](1..1024)) -PassThru | Format-List Workgroups, CpuTimeMs, GpuTimeNs
```

```text
Workgroups : {16, 1, 1}
CpuTimeMs  : 13.238
GpuTimeNs  : 2496
```

A GLSL shader, recognized by its `#version` line:

```powershell
$glsl = @'
#version 450
layout(local_size_x = 64) in;
layout(set = 0, binding = 0) buffer Data { float v[]; };
void main() { uint i = gl_GlobalInvocationID.x; if (i < v.length()) { v[i] = v[i] + 0.5; } }
'@
Invoke-GuiGpuCompute $glsl ([float[]](1, 2, 3))
```

```text
1.5
2.5
3.5
```

## Errors

The command's errors are non-terminating, so each attempt here stops on its own with `-ErrorAction Stop`. A binding the shader does not use, the wrong number of resources, two ways of sizing a run, and a shader that does not validate:

```powershell
foreach ($attempt in { Invoke-GuiGpuCompute $add @{ c = [float[]](1, 2) } -ErrorAction Stop },
                     { Invoke-GuiGpuCompute $add ([float[]](1, 2)) -ErrorAction Stop },
                     { Invoke-GuiGpuCompute $double ([float[]](1, 2)) -Workgroups 1 -Invocations 2 -ErrorAction Stop },
                     { Invoke-GuiGpuCompute '@compute @workgroup_size(1) fn main() { let x: f32 = 1u; }' -ErrorAction Stop }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
```

```text
GuiGpuBinding,Pwrs.Modules.PWRS_GUI.InvokeGuiGpuComputeCommand: the shader uses no binding named c; it uses a (0:0), b (0:1)
GuiGpuBindingCount,Pwrs.Modules.PWRS_GUI.InvokeGuiGpuComputeCommand: the shader uses 2 bindings (a (0:0), b (0:1)) and 1 resource was given; give one per binding in that order, or a hashtable by name
GuiGpuDispatchSize,Pwrs.Modules.PWRS_GUI.InvokeGuiGpuComputeCommand: give one of -Workgroups, -Invocations and -Indirect
WgpuShaderInvalid,Pwrs.Modules.PWRS_GUI.InvokeGuiGpuComputeCommand: the shader is invalid: line 1, column 45: the type of `x` is expected to be `f32`, but got `u32`
```

The [Error Reference](Error-Reference.md) describes these and the session's errors.
