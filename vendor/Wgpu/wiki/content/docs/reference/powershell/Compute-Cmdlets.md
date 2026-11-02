---
title: Compute
weight: 30
---

The syntax, the parameters and the errors of `Invoke-WgpuCompute`, `Invoke-WgpuMap`, `Measure-WgpuArray`, `New-WgpuComputePipeline` and `New-WgpuBindGroup`. The examples are on [Compute](Compute.md).

### Invoke-WgpuCompute

Runs a compute shader once over arrays or buffers and writes the arrays it may have written.

The shader is WGSL or GLSL text, a Wgpu.Shader, or a pipeline from New-WgpuComputePipeline. -Binding gives its resources: an array in the order the shader declares its bindings, a hashtable by binding name or "group:binding", or for one binding the resource itself. An array of numbers is copied to the GPU as the element type the shader declares, and each array bound read_write comes back as an array of that type; a Wgpu.Buffer stays on the GPU. Without -Workgroups or -Invocations the run has one invocation per element of the first buffer.

Alias: `Invoke-GpuCompute`

```powershell
Invoke-WgpuCompute [-Shader] <Object> [[-Binding] <Object>] [-Workgroups <uint[]>] [-Invocations <ulong[]>] [-Indirect <Buffer>] [-IndirectOffset <ulong>] [-Entry <string>] [-Constant <IDictionary>] [-Immediate <Object>] [-Language <ShaderLanguage>] [-PassThru] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Binding` | object |  | The resources for the shader's bindings. Alias: `-Data`, `-Buffer`, `-Resource`. |
| `-Constant` | IDictionary |  | Override constant values, by name or `@id`. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to run on; the default device when omitted. |
| `-Entry` | string |  | The entry point; the shader's only compute entry point when omitted. |
| `-Immediate` | object |  | Immediate data, as an array of numbers. |
| `-Indirect` | [Wgpu.Buffer](Types.md#wgpubuffer) |  | Read the workgroup counts from this buffer on the GPU (three uint). |
| `-IndirectOffset` | ulong |  | The byte offset of the counts in -Indirect. |
| `-Invocations` | ulong[] |  | Invocations in x, y and z, divided by the shader's workgroup size and rounded up. |
| `-Language` | [Wgpu.ShaderLanguage](Types.md#wgpushaderlanguage) |  | The language of shader text; inferred when omitted. |
| `-PassThru` | switch |  | Write a Wgpu.ComputeResult with every output and the time taken, measured on the GPU where it can be. |
| `-Shader` | object |  | WGSL or GLSL text, a Wgpu.Shader, or a Wgpu.ComputePipeline. |
| `-Workgroups` | uint[] |  | Workgroups to dispatch in x, y and z (1 for a dimension left out). |

Writes `array` or [Wgpu.ComputeResult](Types.md#wgpucomputeresult).

#### Errors

The shader: `WgpuNoShader`, `WgpuShaderInvalid`, `WgpuSpirV`, `WgpuNoEntryPoint`, `WgpuEntryPointAmbiguous`, `WgpuNotCompute`, `WgpuLayout`, `WgpuShader` and `WgpuPipeline`. The resources: `WgpuBindingCount`, `WgpuBindingKey`, `WgpuNoBinding`, `WgpuBindingMissing`, `WgpuNotStruct`, `WgpuNoMember`, `WgpuMemberNotNumbers`, `WgpuMemberSize`, `WgpuWrongResource`, `WgpuOtherDevice`, `WgpuEmpty`, `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuBuffer` and `WgpuBindGroup`. The run: `WgpuDispatchSize`, `WgpuNoDispatchSize`, `WgpuTooManyWorkgroups`, `WgpuImmediates`, `WgpuQuery`, `WgpuDispatch`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`. Without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a value that does not convert to a number.

### Invoke-WgpuMap

Runs a WGSL expression over arrays on the GPU, one invocation per element, and writes the results as one array.

The expression reads `x`, the element of -InputObject, `i`, its index as u32, and each variable -Variable names: an array is read element by element like `x`, a number is a constant. The result has -ElementType, or the type of `x`.

Alias: `Invoke-GpuMap`

```powershell
Invoke-WgpuMap [-Expression] <string> [[-InputObject] <Object>] [-Variable <IDictionary>] [-ElementType <ElementType>] [-AsBuffer] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AsBuffer` | switch |  | Write the results as a Wgpu.Buffer left on the GPU. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to run on; the default device when omitted. |
| `-ElementType` | [Wgpu.ElementType](Types.md#wgpuelementtype) |  | The type of the results; the type of `x` when omitted. Alias: `-Type`. |
| `-Expression` | string |  | The WGSL expression, in terms of `x`, `i` and the variables. |
| `-InputObject` | object | by value | The array `x` runs over, or its elements piped in. |
| `-Variable` | IDictionary |  | More inputs: an array is read element by element, a number is a constant. |

Writes `array` or [Wgpu.Buffer](Types.md#wgpubuffer).

#### Errors

The input: `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuVariableName`, `WgpuVariableLength`, `WgpuNoWgslType` and `WgpuNotFinite`. The expression: `WgpuShaderInvalid`, `WgpuMapUnused`, `WgpuLayout`, `WgpuShader` and `WgpuPipeline`. The run: `WgpuBuffer`, `WgpuBindGroup`, `WgpuDispatch`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`. Without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a value that does not convert to a number.

### Measure-WgpuArray

Sums, averages, and finds the minimum and maximum of an array or a buffer on the GPU, like Measure-Object over numbers.

Int32 and UInt32 elements are added in 64 bits, and Int64 and UInt64 elements in 64 bits within each workgroup, which a workgroup whose sum leaves that range overflows. Every workgroup's sum is then added in 128 bits, and Sum is a double, exact up to 2^53. Float sums add each workgroup's elements in the element's own precision and the workgroups' sums in double.

Alias: `Measure-GpuArray`

```powershell
Measure-WgpuArray [-InputObject] <Object> [-ElementType <ElementType>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to run on; the default device when omitted. |
| `-ElementType` | [Wgpu.ElementType](Types.md#wgpuelementtype) |  | The element type to measure the numbers as; their own type, or Float32, when omitted. Alias: `-Type`. |
| `-InputObject` | object | by value | The numbers: an array, a Wgpu.Buffer, or numbers piped in. |

Writes [Wgpu.Measurement](Types.md#wgpumeasurement).

#### Errors

`WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuEmpty`, `WgpuNoWgslType`, `WgpuReduction`, and from running the reduction, `WgpuShader`, `WgpuPipeline`, `WgpuLayout`, `WgpuBuffer`, `WgpuBindGroup`, `WgpuDispatch`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`. Without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a value that does not convert to a number.

### New-WgpuComputePipeline

Builds a compute pipeline from a shader for one device.

Its bind group layouts come from the bindings the entry point uses, so resources bind in the order the shader declares them, or by name. Building it once and running it many times saves compiling the shader on every run.

Alias: `New-GpuComputePipeline`

```powershell
New-WgpuComputePipeline [-Shader] <Object> [-Entry <string>] [-Constant <IDictionary>] [-Language <ShaderLanguage>] [-Label <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Constant` | IDictionary |  | Override constant values, by name or `@id`. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to build it on; the default device when omitted. |
| `-Entry` | string |  | The entry point; the shader's only compute entry point when omitted. |
| `-Label` | string |  | The pipeline's label. |
| `-Language` | [Wgpu.ShaderLanguage](Types.md#wgpushaderlanguage) |  | The language of shader text; inferred when omitted. |
| `-Shader` | object | by value | WGSL or GLSL text, or a Wgpu.Shader. |

Writes [Wgpu.ComputePipeline](Types.md#wgpucomputepipeline).

#### Errors

`WgpuNoShader`, `WgpuShaderInvalid`, `WgpuSpirV`, `WgpuNoEntryPoint`, `WgpuEntryPointAmbiguous`, `WgpuNotCompute`, `WgpuLayout`, `WgpuShader` and `WgpuPipeline`, and without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a `-Constant` value that is not a number.

### New-WgpuBindGroup

Binds resources to one group of a compute pipeline's bindings.

The resources are an array in binding order, a hashtable by binding name or "group:binding", or one resource for a group with one binding.

Alias: `New-GpuBindGroup`

```powershell
New-WgpuBindGroup [-Pipeline] <ComputePipeline> [-Resource] <Object> [-Group <uint>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Group` | uint |  | The group index; 0 when omitted. |
| `-Pipeline` | [Wgpu.ComputePipeline](Types.md#wgpucomputepipeline) | by value | The pipeline whose bindings the group fills. |
| `-Resource` | object |  | The resources. |

Writes [Wgpu.BindGroup](Types.md#wgpubindgroup).

#### Errors

`WgpuNoPipeline`, `WgpuNoGroup` and `WgpuBindGroup`; and from matching the resources to the bindings, `WgpuBindingCount`, `WgpuBindingKey`, `WgpuNoBinding`, `WgpuBindingMissing`, `WgpuNotStruct`, `WgpuNoMember`, `WgpuMemberNotNumbers`, `WgpuMemberSize`, `WgpuWrongResource`, `WgpuOtherDevice`, `WgpuEmpty`, `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange` and `WgpuBuffer`. Each is described in the [Error Reference](Error-Reference.md).
