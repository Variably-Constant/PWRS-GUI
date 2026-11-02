---
title: Types
weight: 90
---

Every class and enumeration of the module, all in the `Wgpu` namespace: the 32 classes the cmdlets write and take and the methods return, with the errors their methods throw, and the 43 enumerations a parameter or a property takes, with each value's number. A property's type is the .NET type the module declares; where that is `object`, the text beside it says what it holds. A parameter of an enumeration's type takes a value's name as a string, as `-Backend Vulkan`, and PowerShell completes it.

## Classes

### Wgpu.Adapter

A GPU adapter wgpu can open, with everything it supports.

Written by [Get-WgpuAdapter](Device-Cmdlets.md#get-wgpuadapter).

| Property | Type | Holds |
|---|---|---|
| `Name` | string | The adapter's name, as the driver reports it. |
| `Backend` | [Wgpu.Backend](#wgpubackend) | The graphics API it is reached through. |
| `DeviceType` | [Wgpu.DeviceType](#wgpudevicetype) | DiscreteGpu, IntegratedGpu, VirtualGpu, Cpu or Other. |
| `Vendor` | uint | The vendor id, a PCI vendor id on most backends. |
| `Device` | uint | The device id, a PCI device id on most backends. |
| `PciBusId` | string | The PCI bus id, where the backend reports one. |
| `Driver` | string | The driver's name. |
| `DriverInfo` | string | The driver's version and details. |
| `SubgroupMinSize` | uint | The smallest subgroup size the adapter runs. |
| `SubgroupMaxSize` | uint | The largest subgroup size the adapter runs. |
| `Features` | string[] | Every optional feature the adapter supports, which Open-WgpuDevice requests. |
| `Limits` | [Wgpu.Limits](#wgpulimits) | Every limit of the adapter, which Open-WgpuDevice requests. |
| `DownlevelFlags` | string[] | The downlevel capabilities the adapter has, by wgpu's names for them. |
| `ShaderModel` | string | The shader model wgpu reports for the adapter. |

### Wgpu.AllocatorReport

The backend's report of a device's GPU memory.

Returned by the `AllocatorReport` method of [Wgpu.Device](#wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `TotalAllocatedBytes` | ulong | Bytes allocated to resources. |
| `TotalReservedBytes` | ulong | Bytes reserved from the driver, allocated or not. |
| `Allocations` | ulong | Number of allocations. |
| `Blocks` | ulong | Number of memory blocks the allocations live in. |

### Wgpu.BindGroup

A bind group: resources bound to one group of a pipeline's bindings, set on a pass with SetBindGroup.

Written by [New-WgpuBindGroup](Compute-Cmdlets.md#new-wgpubindgroup). Returned by the `BindGroup` method of [Wgpu.ComputePipeline](#wgpucomputepipeline), the `CreateBindGroup` method of [Wgpu.Device](#wgpudevice) and the `BindGroup` method of [Wgpu.RenderPipeline](#wgpurenderpipeline).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The bind group's label. |
| `Group` | uint | The group index it was made for. |
| `Bindings` | string[] | The names of the bindings it fills, in binding order. |

### Wgpu.Buffer

A GPU buffer of typed elements, held in Rust. Its data stays on the GPU until something reads it.

Written by [Invoke-WgpuMap](Compute-Cmdlets.md#invoke-wgpumap) and [New-WgpuBuffer](Buffer-Cmdlets.md#new-wgpubuffer). Returned by the `Upload`, `Zeros` and `CreateBuffer` methods of [Wgpu.Device](#wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The buffer's label. |
| `ElementType` | [Wgpu.ElementType](#wgpuelementtype) | The type of its elements. |
| `Length` | ulong | The number of elements it holds. |
| `Size` | ulong | Its size in bytes, the elements' bytes rounded up to a multiple of 4. |
| `Usage` | [Wgpu.BufferUsage](#wgpubufferusage)[] | What it may be used for. |
| `Adapter` | string | The adapter of the device it lives on. |

| Method | Returns | Does |
|---|---|---|
| `Read(Wgpu.ElementType elementType)` | object | Copies the buffer back and returns its elements as a .NET array of their type; `elementType` reads the same bytes as another type. |
| `ReadAsync(Wgpu.ElementType elementType, CancellationToken cancellationToken)` | Task[object] | Read on a thread of its own: returns at once with a task whose result is the array. |
| `ReadRange(ulong start, ulong count)` | object | Copies `count` elements from element `start` back, `count` running to the end when omitted. |
| `ReadRangeAsync(ulong start, ulong count, CancellationToken cancellationToken)` | Task[object] | ReadRange on a thread of its own: returns at once with a task whose result is the elements. |
| `ReadMemory()` | object | Copies the buffer back into a `Memory<T>` handed to .NET without a second copy (a `T[]` on Windows PowerShell). |
| `ReadMemoryAsync(CancellationToken cancellationToken)` | Task[object] | ReadMemory on a thread of its own: returns at once with a task whose result is the `Memory<T>`. |
| `ReadInto(object destination)` | void | Copies the buffer back into `destination` from its index 0: an array of the type Read returns, holding at least the buffer's elements. The elements past them keep their values. |
| `ReadIntoAsync(object destination, CancellationToken cancellationToken)` | Task | ReadInto on a thread of its own: `destination` is checked now, and the task ends when it holds the elements. |
| `Write(object data, ulong offset)` | void | Overwrites elements from element `offset` (0 when omitted) with `data`, converted to the buffer's element type. |
| `WriteAsync(object data, ulong offset, CancellationToken cancellationToken)` | Task | Write with the writing on a thread of its own: `data` is converted now, and the task ends when it is written. |
| `Fill(object value)` | void | Sets every element to `value`, a number converted to the element type as each number of untyped data is. |
| `FillAsync(object value, CancellationToken cancellationToken)` | Task | Fill with the filling on a thread of its own: `value` is converted now, and the task ends when every element holds it. |
| `Clear()` | void | Sets every byte of the buffer to zero. |
| `ClearAsync(CancellationToken cancellationToken)` | Task | Clear on a thread of its own: returns at once with a task that ends when every byte is zero. |
| `CopyTo(Wgpu.Buffer destination, ulong sourceOffset, ulong destinationOffset, ulong count)` | void | Copies `count` elements from element `sourceOffset` of this buffer to element `destinationOffset` of `destination`, as bytes. |
| `CopyToAsync(Wgpu.Buffer destination, ulong sourceOffset, ulong destinationOffset, ulong count, CancellationToken cancellationToken)` | Task | CopyTo on a thread of its own: returns at once with a task that ends when the elements are copied. |
| `Slice(ulong start, ulong count)` | [Wgpu.BufferSlice](#wgpubufferslice) | Elements `start` to `start + count` (to the end when `count` is omitted), bound in place of the whole buffer. A binding's offset must be a multiple of the device's MinStorageBufferOffsetAlignment, or of MinUniformBufferOffsetAlignment for a uniform. |
| `Reinterpret(Wgpu.ElementType elementType)` | [Wgpu.Buffer](#wgpubuffer) | The same GPU memory seen as elements of `elementType`. |
| `Destroy()` | void | Frees the buffer's GPU memory now; the object stops working. |

#### Errors

`Read`, `ReadAsync`, `ReadMemory` and `ReadMemoryAsync`, and a slice's `Read` and `ReadAsync`: `WgpuRead`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuMap` and `WgpuStopped`, and from those that return an array, `WgpuBufferTooLarge` for more elements than a .NET array holds. `ReadRange` and `ReadRangeAsync`: `WgpuRange` and those of `Read`. `ReadInto` and `ReadIntoAsync`, a buffer's and a slice's: `WgpuDestination`, `WgpuRead`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuMap` and `WgpuStopped`. `Write` and `WriteAsync`: `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuRange` and `WgpuWrite`, and PWRS's `PwrsConversionError` for a value that does not convert to a number; a write that starts or ends inside a 4-byte word reads that word back first, with the errors of `Read`. `Fill` and `FillAsync`: `WgpuOutOfRange` and `WgpuWrite`, PWRS's `PwrsConversionError` for a value that does not convert to a number, and the errors of `Read` for a buffer whose bytes do not fill whole 4-byte words; a value of 0 clears the buffer, with the errors of `Clear`. `Clear` and `ClearAsync`: `WgpuClear`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll` and `WgpuStopped`. `CopyTo` and `CopyToAsync`: `WgpuRange`, `WgpuOtherDevice`, `WgpuCopyAlignment`, `WgpuCopy`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll` and `WgpuStopped`. `Slice`: `WgpuRange`. A method throws them as the [Error Reference](Error-Reference.md) describes.

### Wgpu.BufferSlice

A range of a buffer's elements, bound in place of the whole buffer.

Returned by the `Slice` method of [Wgpu.Buffer](#wgpubuffer).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The buffer's label. |
| `Start` | ulong | The first element of the range. |
| `Length` | ulong | The number of elements in the range. |
| `ElementType` | [Wgpu.ElementType](#wgpuelementtype) | The type of the elements. |

| Method | Returns | Does |
|---|---|---|
| `Read()` | object | Copies the range back and returns its elements. |
| `ReadAsync(CancellationToken cancellationToken)` | Task[object] | Read on a thread of its own: returns at once with a task whose result is the elements. |
| `ReadInto(object destination)` | void | Copies the range back into `destination` from its index 0: an array of the type Read returns, holding at least the range's elements. The elements past them keep their values. |
| `ReadIntoAsync(object destination, CancellationToken cancellationToken)` | Task | ReadInto on a thread of its own: `destination` is checked now, and the task ends when it holds the elements. |

### Wgpu.CommandBuffer

A finished recording of GPU work, submitted once.

Returned by the `Finish` method of [Wgpu.CommandEncoder](#wgpucommandencoder).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The label of the encoder that recorded it. |
| `IsSubmitted` | bool | Whether it was submitted; a command buffer runs once. |

### Wgpu.CommandEncoder

Records GPU work: passes, copies and queries, finished into a command buffer and submitted to the device.

Written by [New-WgpuCommandEncoder](Command-Cmdlets.md#new-wgpucommandencoder). Returned by the `CreateCommandEncoder` method of [Wgpu.Device](#wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The encoder's label. |
| `IsFinished` | bool | Whether Finish was called; a finished encoder records nothing more. |

| Method | Returns | Does |
|---|---|---|
| `BeginComputePass(object label, Wgpu.QuerySet timestamps, uint beginningIndex, uint endIndex)` | [Wgpu.ComputePass](#wgpucomputepass) | Begins a compute pass. With `timestamps`, the GPU writes the time the pass begins into query `beginningIndex` and the time it ends into `endIndex` (0 and 1 when omitted). |
| `BeginRenderPass(object color, double[] clearColor, object depth, object resolveTarget, Wgpu.QuerySet occlusionQuerySet, object label)` | [Wgpu.RenderPass](#wgpurenderpass) | Begins a render pass drawing into `color`, a Wgpu.Texture or Wgpu.TextureView cleared to `clearColor` (red, green, blue and alpha; opaque black when omitted). `depth` is a depth texture to test against, its depth cleared to 1 and its stencil to 0, `resolveTarget` takes a multisampled `color` resolved, and `occlusionQuerySet` receives occlusion query counts. A hashtable in place of all of them names them by key: Color, ClearColor, Depth, ClearDepth, ClearStencil, ResolveTarget, OcclusionQuerySet and Label, where a clear key given 'Load' keeps what its texture holds. |
| `CopyTextureToTexture(Wgpu.Texture source, Wgpu.Texture destination, uint mipLevel)` | void | Copies mip level `mipLevel` (0 when omitted) of one texture into the same mip level of another of the same size and format. |
| `CopyBufferToBuffer(Wgpu.Buffer source, ulong sourceOffset, Wgpu.Buffer destination, ulong destinationOffset, ulong size)` | void | Copies `size` bytes (to the end of the source when omitted) between buffers; offsets and size are whole 4-byte words. |
| `ClearBuffer(Wgpu.Buffer buffer, ulong offset, ulong size)` | void | Zeroes `size` bytes from byte `offset` of a buffer (all of it when omitted). |
| `ResolveQuerySet(Wgpu.QuerySet querySet, uint first, uint count, Wgpu.Buffer destination, ulong destinationOffset)` | void | Resolves `count` queries from `first` into `destination` at byte `destinationOffset`. |
| `WriteTimestamp(Wgpu.QuerySet querySet, uint index)` | void | Writes a timestamp into query `index` between passes (feature TimestampQueryInsideEncoders). |
| `PushDebugGroup(string label)` | void | Opens a named group of commands for graphics debuggers. |
| `PopDebugGroup()` | void | Closes the innermost debug group. |
| `InsertDebugMarker(string label)` | void | Marks a point in the recording for graphics debuggers. |
| `Finish()` | [Wgpu.CommandBuffer](#wgpucommandbuffer) | Finishes the recording into a command buffer to submit. |

#### Errors

`WgpuEncoderFinished`, `WgpuOtherDevice`, `WgpuAttachment` and `WgpuFinish`; and from `BeginRenderPass`, `WgpuClearColor`, and with a hashtable `WgpuRenderPassOption`, `WgpuClearDepth` and `WgpuClearStencil`. wgpu checks a pass's commands when its encoder finishes, so an invalid command surfaces as `WgpuFinish`. A method throws them as the [Error Reference](Error-Reference.md) describes.

### Wgpu.ComputePass

A compute pass being recorded into an encoder.

Returned by the `BeginComputePass` method of [Wgpu.CommandEncoder](#wgpucommandencoder).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The pass's label. |
| `IsEnded` | bool | Whether End was called; the encoder takes no other command until it is. |

| Method | Returns | Does |
|---|---|---|
| `SetPipeline(Wgpu.ComputePipeline pipeline)` | void | Sets the pipeline the next dispatches run. |
| `SetBindGroup(uint index, Wgpu.BindGroup bindGroup, uint[] offsets)` | void | Sets bind group `index`, with dynamic offsets when its layout has them. |
| `SetImmediates(uint offset, object data)` | void | Sets immediate data from byte `offset`, from an array of numbers. |
| `Dispatch(uint x, uint y, uint z)` | void | Dispatches `x` by `y` by `z` workgroups (`y` and `z` 1 when omitted). |
| `DispatchIndirect(Wgpu.Buffer buffer, ulong offset)` | void | Dispatches the workgroup counts three uint at byte `offset` of `buffer` hold. |
| `WriteTimestamp(Wgpu.QuerySet querySet, uint index)` | void | Writes a timestamp into query `index` (feature TimestampQueryInsidePasses). |
| `BeginPipelineStatisticsQuery(Wgpu.QuerySet querySet, uint index)` | void | Starts gathering pipeline statistics into query `index`. |
| `EndPipelineStatisticsQuery()` | void | Stops gathering pipeline statistics. |
| `PushDebugGroup(string label)` | void | Opens a named group of commands for graphics debuggers. |
| `PopDebugGroup()` | void | Closes the innermost debug group. |
| `InsertDebugMarker(string label)` | void | Marks a point in the pass for graphics debuggers. |
| `End()` | void | Ends the pass, so its encoder takes commands again. |

#### Errors

`WgpuPassEnded`, which a method throws as the [Error Reference](Error-Reference.md) describes. wgpu checks the pass's commands when its encoder finishes, so an invalid command surfaces there as `WgpuFinish`.

### Wgpu.ComputePipeline

A compute pipeline, built for one device: a shader's entry point with its bind group layouts and override values fixed.

Written by [New-WgpuComputePipeline](Compute-Cmdlets.md#new-wgpucomputepipeline). Returned by the `CreateComputePipeline` method of [Wgpu.Device](#wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The pipeline's label. |
| `EntryPoint` | string | The entry point it runs. |
| `WorkgroupSize` | uint[] | Invocations per workgroup in x, y and z. |
| `Bindings` | [Wgpu.ShaderBinding](#wgpushaderbinding)[] | The bindings the entry point uses, in the order resources bind to them. |
| `ImmediateSize` | uint | Bytes of immediate data the entry point reads. |
| `Adapter` | string | The adapter of the device it was built on. |

| Method | Returns | Does |
|---|---|---|
| `Dispatch(object resources, uint x, uint y, uint z)` | void | Binds `resources` and runs the pipeline over `x` by `y` by `z` workgroups (`y` and `z` 1 when omitted), waiting for it to finish. |
| `DispatchAsync(object resources, uint x, uint y, uint z, CancellationToken cancellationToken)` | Task | Dispatch, returning at once with a task that ends when the GPU has finished the run. `resources` bind now, on the calling thread. |
| `BindGroup(uint group, object resources)` | [Wgpu.BindGroup](#wgpubindgroup) | The bind group `group` of this pipeline, binding `resources`: an array in binding order, or a hashtable by binding name or "group:binding". |

#### Errors

`Dispatch` and `DispatchAsync` throw the errors of binding and running that [Invoke-WgpuCompute](Compute-Cmdlets.md#invoke-wgpucompute) lists. `BindGroup` throws `WgpuNoGroup`, `WgpuBindGroup` and the errors of matching resources to bindings that [New-WgpuBindGroup](Compute-Cmdlets.md#new-wgpubindgroup) lists. Each is described in the [Error Reference](Error-Reference.md).

### Wgpu.ComputeResult

What Invoke-WgpuCompute -PassThru writes: every array the shader may have written, by binding name, and how long the run took.

Written by [Invoke-WgpuCompute](Compute-Cmdlets.md#invoke-wgpucompute).

| Property | Type | Holds |
|---|---|---|
| `Outputs` | object | A hashtable of each array the shader may have written, by binding name. |
| `Workgroups` | uint[] | The workgroups dispatched in x, y and z; 0 for an indirect dispatch. |
| `GpuTimeNs` | double? | Nanoseconds the GPU spent in the pass, where the device has timestamp queries and -PassThru asked for them. |
| `CpuTimeMs` | double | Milliseconds the whole call took, binding and copying included. |

### Wgpu.Counters

How many objects of each kind a device holds and the memory they take, as the backend counts them.

Returned by the `Counters` method of [Wgpu.Device](#wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `Buffers` | long | Buffers the device holds now, staging buffers among them. |
| `Textures` | long | Textures the device holds now. |
| `TextureViews` | long | Texture views the device holds now. |
| `BindGroups` | long | Bind groups the device holds now. |
| `BindGroupLayouts` | long | Bind group layouts the device holds now. |
| `RenderPipelines` | long | Render pipelines the device holds now. |
| `ComputePipelines` | long | Compute pipelines the device holds now. |
| `RayTracingPipelines` | long | Ray tracing pipelines the device holds now. |
| `PipelineLayouts` | long | Pipeline layouts the device holds now. |
| `Samplers` | long | Samplers the device holds now. |
| `CommandEncoders` | long | Command encoders the device holds now, finished or not. |
| `ShaderModules` | long | Shader modules the device holds now, one for each pipeline's shader. |
| `QuerySets` | long | Query sets the device holds now. |
| `Fences` | long | Fences the device holds now, which track submissions. |
| `BufferMemory` | long | Bytes of GPU memory attributed to buffers. |
| `TextureMemory` | long | Bytes of GPU memory attributed to textures. |
| `AccelerationStructureMemory` | long | Bytes of GPU memory attributed to acceleration structures. |
| `MemoryAllocations` | long | Number of GPU memory allocations. |

### Wgpu.Device

A GPU device held in Rust. Buffers, shaders, pipelines and everything else are made on it, and keep it alive while any of them lives.

Written by [Get-WgpuDevice](Device-Cmdlets.md#get-wgpudevice) and [Open-WgpuDevice](Device-Cmdlets.md#open-wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `Adapter` | string | The adapter's name. |
| `Backend` | [Wgpu.Backend](#wgpubackend) | The graphics API the device is reached through. |
| `DeviceType` | [Wgpu.DeviceType](#wgpudevicetype) | DiscreteGpu, IntegratedGpu, VirtualGpu, Cpu or Other. |
| `Label` | string | The label the device was opened with. |
| `Features` | string[] | Every optional feature the device has. |
| `Limits` | [Wgpu.Limits](#wgpulimits) | Every limit of the device. |
| `MaxBufferSize` | ulong | The largest buffer the device creates, in bytes. |
| `MaxStorageBufferBindingSize` | ulong | The largest storage buffer one binding can see, in bytes. |

| Method | Returns | Does |
|---|---|---|
| `[Wgpu.Device]::new(object adapter, Wgpu.Backend backend)` | [Wgpu.Device](#wgpudevice) | Opens a device on the adapter whose name contains `adapter`, or the one wgpu prefers, through `backend` when given. |
| `[Wgpu.Device]::GetDefault()` | [Wgpu.Device](#wgpudevice) | The device cmdlets use when given no -Device, opened now if none is. |
| `SetDefault()` | void | Makes this the device cmdlets use when given no -Device. |
| `Upload(object data, Wgpu.ElementType elementType)` | [Wgpu.Buffer](#wgpubuffer) | Copies `data`, an array of any numeric type, to a new buffer, as `elementType` when given and as its own type otherwise. |
| `Zeros(ulong length, Wgpu.ElementType elementType)` | [Wgpu.Buffer](#wgpubuffer) | A new buffer of `length` zeroed elements of `elementType`, Float32 when omitted. |
| `CreateBuffer(ulong length, Wgpu.ElementType elementType, Wgpu.BufferUsage[] usage, object label)` | [Wgpu.Buffer](#wgpubuffer) | A new zeroed buffer of `length` elements with the usages given, or every usage but mapping when none are. |
| `CreateShader(string source, Wgpu.ShaderLanguage language, Wgpu.ShaderStage stage)` | [Wgpu.Shader](#wgpushader) | Parses and validates a shader; the same as `[Wgpu.Shader]::new`. |
| `CreateComputePipeline(object shader, object entry, IDictionary constants, object label)` | [Wgpu.ComputePipeline](#wgpucomputepipeline) | A compute pipeline for `shader` (WGSL text or a Wgpu.Shader), with `entry` as its entry point and `constants` as its override values. |
| `CreateBindGroup(Wgpu.ComputePipeline pipeline, uint group, object resources)` | [Wgpu.BindGroup](#wgpubindgroup) | The bind group `group` of `pipeline`, binding `resources`: an array in binding order, or a hashtable by binding name or "group:binding". |
| `CreateCommandEncoder(object label)` | [Wgpu.CommandEncoder](#wgpucommandencoder) | A command encoder to record copies and passes into. |
| `CreateQuerySet(Wgpu.QueryKind kind, uint count, object label)` | [Wgpu.QuerySet](#wgpuqueryset) | A set of `count` queries of `kind`. |
| `Submit(Wgpu.CommandBuffer[] commands)` | void | Submits finished command buffers and waits for the GPU to run them. |
| `SubmitAsync(Wgpu.CommandBuffer[] commands, CancellationToken cancellationToken)` | Task | Submits finished command buffers and returns at once with a task that ends when the GPU has run them. |
| `Wait()` | void | Waits until the GPU has finished all submitted work. |
| `WaitAsync(CancellationToken cancellationToken)` | Task | Returns at once with a task that ends when the GPU has finished all work submitted so far. |
| `Run(object shader, object resources, uint x, uint y, uint z, object entry)` | void | Runs a compute shader once. `shader` is WGSL text, a Wgpu.Shader or a Wgpu.ComputePipeline; `resources` bind as Invoke-WgpuCompute binds them; `x`, `y` and `z` count workgroups. |
| `RunAsync(object shader, object resources, uint x, uint y, uint z, object entry, CancellationToken cancellationToken)` | Task | Runs a compute shader once, as Run does, and returns at once with a task that ends when the GPU has finished it. |
| `Download(Wgpu.Buffer buffer)` | object | Copies a buffer back from the device and returns its values. |
| `DownloadAsync(Wgpu.Buffer buffer, CancellationToken cancellationToken)` | Task[object] | Copies a buffer back on a thread of its own and returns at once with a task whose result is its values. |
| `Status()` | [Wgpu.DeviceStatus](#wgpudevicestatus) | Whether the device was lost and what it raised outside any call. |
| `Counters()` | [Wgpu.Counters](#wgpucounters) | The backend's object and memory counts for this device. |
| `AllocatorReport()` | [Wgpu.AllocatorReport](#wgpuallocatorreport) | The backend's memory report, or `$null` where the backend keeps none. |
| `TimestampPeriod()` | float | Nanoseconds per timestamp tick on this device's queue. |
| `StartCapture()` | void | Starts a capture in an attached RenderDoc or Xcode; does nothing without one. One capture at a time, which StopCapture ends. |
| `StopCapture()` | void | Ends the capture StartCapture began. |
| `Destroy()` | void | Destroys the device: every object made on it stops working, and the device reports itself lost once the work it was given has finished. |

#### Errors

The constructor and `GetDefault`: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. `Upload`: `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuEmpty`, `WgpuBufferTooLarge` and `WgpuBuffer`, and PWRS's `PwrsConversionError` for a value that does not convert to a number. `Zeros` and `CreateBuffer`: `WgpuEmpty`, `WgpuBufferTooLarge` and `WgpuBuffer`. `CreateShader`: `WgpuSpirV`, `WgpuStage` and `WgpuShaderInvalid`. `CreateComputePipeline`: `WgpuShaderInvalid`, `WgpuNoEntryPoint`, `WgpuEntryPointAmbiguous`, `WgpuNotCompute`, `WgpuLayout`, `WgpuShader` and `WgpuPipeline`, and PWRS's `PwrsConversionError` for a constant that is not a number. `CreateBindGroup`: `WgpuNoGroup`, `WgpuBindGroup` and the errors of matching resources to bindings that [New-WgpuBindGroup](Compute-Cmdlets.md#new-wgpubindgroup) lists. `CreateCommandEncoder`: `WgpuEncoder`. `CreateQuerySet`: `WgpuQuerySet`. `Submit` and `SubmitAsync`: `WgpuSubmitted`, `WgpuOtherDevice`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuStopped`, `WgpuDeviceLost` and `WgpuUncaptured`. `Wait` and `WaitAsync`: `WgpuSubmitThread`, `WgpuPoll`, `WgpuStopped`, `WgpuDeviceLost` and `WgpuUncaptured`. `Run` and `RunAsync`: the errors of binding and running that [Invoke-WgpuCompute](Compute-Cmdlets.md#invoke-wgpucompute) lists. `Download` and `DownloadAsync`: `WgpuRead`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuMap` and `WgpuStopped`, and `WgpuBufferTooLarge` for more elements than a .NET array holds. `Destroy`: `WgpuPoll`. A method throws them as the [Error Reference](Error-Reference.md) describes.

### Wgpu.DeviceEvent

What Wgpu.DeviceLost and Wgpu.UncapturedError carry as their MessageData.

| Property | Type | Holds |
|---|---|---|
| `Device` | string | The label of the device the event came from. |
| `Kind` | string | Destroyed or Unknown for a lost device; Validation, OutOfMemory or Internal for an error. |
| `Message` | string | What wgpu reported; empty for a destroyed device. |

### Wgpu.DeviceStatus

A device's state now: whether it was lost, and the errors it raised outside any call that have not been reported yet.

Returned by the `Status` method of [Wgpu.Device](#wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `IsLost` | bool | Whether the device was lost; a lost device refuses every call. |
| `LostReason` | string | Why it was lost; `$null` while it is not. |
| `PendingErrors` | string[] | Errors raised outside any call, which the device's next call reports. |
| `IsDefault` | bool | Whether this is the device cmdlets use when given no -Device. |

### Wgpu.Harmonics

The irradiance of an environment as nine spherical harmonics a color channel, bands 0 to 2, as cmgen's --sh-irradiance writes them: each is the environment's coefficient in the real orthonormal basis times the clamped cosine's, pi, 2 pi / 3 or pi / 4 by band, so the irradiance a surface facing the unit direction (x, y, z) receives is the sum of each coefficient times its basis function there. Index 0 holds the coefficient of 0.282095; 1, 2 and 3 of -0.488603 y, 0.488603 z and -0.488603 x; 4 to 8 of 1.092548 xy, -1.092548 yz, 0.315392 (3z^2 - 1), -1.092548 xz and 0.546274 (x^2 - y^2).

Held in [Wgpu.Texture](#wgputexture).Harmonics.

| Property | Type | Holds |
|---|---|---|
| `Red` | double[] | Red's nine coefficients. |
| `Green` | double[] | Green's nine coefficients. |
| `Blue` | double[] | Blue's nine coefficients. |

### Wgpu.Limits

Every limit of an adapter or a device, as wgpu reports it. A device opened by this module asks for every limit its adapter has, so a device's limits are its adapter's.

Held in [Wgpu.Adapter](#wgpuadapter).Limits and [Wgpu.Device](#wgpudevice).Limits.

| Property | Type | Holds |
|---|---|---|
| `MaxTextureDimension1d` | ulong | The largest width of a one-dimensional texture, in texels. |
| `MaxTextureDimension2d` | ulong | The largest width or height of a two-dimensional texture, in texels. |
| `MaxTextureDimension3d` | ulong | The largest width, height or depth of a three-dimensional texture, in texels. |
| `MaxTextureArrayLayers` | ulong | The most layers a two-dimensional texture array holds. |
| `MaxBindGroups` | ulong | The most bind groups a pipeline uses at once. |
| `MaxBindGroupsPlusVertexBuffers` | ulong | The most bind group and vertex buffer slots used at once, counting empty slots below the highest. |
| `MaxBindingsPerBindGroup` | ulong | The highest binding number a bind group takes, plus one. |
| `MaxDynamicUniformBuffersPerPipelineLayout` | ulong | The most uniform buffer bindings with dynamic offsets in one pipeline. |
| `MaxDynamicStorageBuffersPerPipelineLayout` | ulong | The most storage buffer bindings with dynamic offsets in one pipeline. |
| `MaxSampledTexturesPerShaderStage` | ulong | The most sampled textures one shader stage sees. |
| `MaxSamplersPerShaderStage` | ulong | The most samplers one shader stage sees. |
| `MaxStorageBuffersPerShaderStage` | ulong | The most storage buffers one shader stage sees. |
| `MaxStorageTexturesPerShaderStage` | ulong | The most storage textures one shader stage sees. |
| `MaxUniformBuffersPerShaderStage` | ulong | The most uniform buffers one shader stage sees. |
| `MaxBindingArrayElementsPerShaderStage` | ulong | The most resources other than samplers that binding arrays hold for one shader stage; 0 without binding arrays. |
| `MaxBindingArrayAccelerationStructureElementsPerShaderStage` | ulong | The most acceleration structures that binding arrays hold for one shader stage; 0 without them. |
| `MaxBindingArraySamplerElementsPerShaderStage` | ulong | The most samplers that binding arrays hold for one shader stage; 0 without binding arrays. |
| `MaxUniformBufferBindingSize` | ulong | The largest uniform buffer binding, in bytes. |
| `MaxStorageBufferBindingSize` | ulong | The largest storage buffer binding, in bytes: the most of an array one shader binding sees. |
| `MaxVertexBuffers` | ulong | The most vertex buffers a render pipeline reads. |
| `MaxBufferSize` | ulong | The largest buffer, in bytes; a buffer smaller than this can still fail for want of memory. |
| `MaxVertexAttributes` | ulong | The most vertex attributes across all of a render pipeline's vertex buffers. |
| `MaxVertexBufferArrayStride` | ulong | The largest distance between consecutive vertices in a vertex buffer, in bytes. |
| `MaxInterStageShaderVariables` | ulong | The most `@location` variables passed between shader stages, such as vertex outputs. |
| `MinUniformBufferOffsetAlignment` | ulong | The alignment, in bytes, of a uniform buffer binding's offset. |
| `MinStorageBufferOffsetAlignment` | ulong | The alignment, in bytes, of a storage buffer binding's offset. |
| `MaxColorAttachments` | ulong | The most color targets a render pipeline draws into. |
| `MaxColorAttachmentBytesPerSample` | ulong | The most bytes one sample takes across all color targets. |
| `MaxComputeWorkgroupStorageSize` | ulong | The most `var<workgroup>` memory a compute entry point uses, in bytes. |
| `MaxComputeInvocationsPerWorkgroup` | ulong | The largest product of a compute entry point's `@workgroup_size` dimensions. |
| `MaxComputeWorkgroupSizeX` | ulong | The largest X dimension of a compute entry point's `@workgroup_size`. |
| `MaxComputeWorkgroupSizeY` | ulong | The largest Y dimension of a compute entry point's `@workgroup_size`. |
| `MaxComputeWorkgroupSizeZ` | ulong | The largest Z dimension of a compute entry point's `@workgroup_size`. |
| `MaxComputeWorkgroupsPerDimension` | ulong | The most workgroups one dispatch runs in each dimension, the bound of `-Workgroups`. |
| `MaxImmediateSize` | ulong | The most immediate data a pipeline takes, in bytes; 0 without the `Immediates` feature. |
| `MaxNonSamplerBindings` | ulong | The most bindings other than samplers alive at once; it bounds only the Direct3D 12 backend. |
| `MaxTaskWorkgroupTotalCount` | ulong | The largest product of the workgroup counts of a mesh draw through a task shader. |
| `MaxTaskWorkgroupsPerDimension` | ulong | The largest workgroup count in each dimension of a mesh draw through a task shader. |
| `MaxMeshWorkgroupTotalCount` | ulong | The largest product of the workgroup counts of a mesh draw without a task shader, or of a task shader's output. |
| `MaxMeshWorkgroupsPerDimension` | ulong | The largest workgroup count in each dimension of a mesh draw without a task shader, or of a task shader's output. |
| `MaxTaskInvocationsPerWorkgroup` | ulong | The most invocations in one task shader workgroup. |
| `MaxTaskInvocationsPerDimension` | ulong | The largest dimension of a task shader's workgroup size. |
| `MaxMeshInvocationsPerWorkgroup` | ulong | The most invocations in one mesh shader workgroup. |
| `MaxMeshInvocationsPerDimension` | ulong | The largest dimension of a mesh shader's workgroup size. |
| `MaxTaskPayloadSize` | ulong | The largest payload a task shader passes to a mesh shader, in bytes. |
| `MaxMeshOutputVertices` | ulong | The most vertices a mesh shader outputs. |
| `MaxMeshOutputPrimitives` | ulong | The most primitives a mesh shader outputs. |
| `MaxMeshOutputLayers` | ulong | The most layers a mesh shader outputs. |
| `MaxMeshMultiviewViewCount` | ulong | The most views a mesh shader renders in multiview rendering. |
| `MaxBlasPrimitiveCount` | ulong | The most primitives in a bottom-level acceleration structure; 0 without ray queries. |
| `MaxBlasGeometryCount` | ulong | The most geometries in a bottom-level acceleration structure; 0 without ray queries. |
| `MaxTlasInstanceCount` | ulong | The most instances in a top-level acceleration structure; 0 without ray queries. |
| `MaxAccelerationStructuresPerShaderStage` | ulong | The most acceleration structures one shader stage uses; 0 without ray queries. |
| `MaxBuffersAndAccelerationStructuresPerShaderStage` | ulong | The most storage, uniform and vertex buffers and acceleration structures one shader stage binds, together. |
| `MaxMultiviewViewCount` | ulong | The most views in multiview rendering. |
| `MaxRayDispatchCount` | ulong | The most rays one trace call dispatches; 0 without ray tracing pipelines. |
| `MaxRayRecursionDepth` | ulong | The deepest ray recursion a ray tracing pipeline may ask for; 0 without ray tracing pipelines. |

### Wgpu.Measurement

What Measure-WgpuArray writes.

Written by [Measure-WgpuArray](Compute-Cmdlets.md#measure-wgpuarray).

| Property | Type | Holds |
|---|---|---|
| `Count` | ulong | The number of elements. |
| `Sum` | double | Their sum: for integers, added in at least 64 bits and exact up to 2^53, past which a double skips integers; for floats, added per workgroup in the element's own precision. |
| `Average` | double | Their mean. |
| `Minimum` | double | The smallest. |
| `Maximum` | double | The largest. |

### Wgpu.QuerySet

A set of queries the GPU writes results into.

Written by [New-WgpuQuerySet](Command-Cmdlets.md#new-wgpuqueryset). Returned by the `CreateQuerySet` method of [Wgpu.Device](#wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The query set's label. |
| `Kind` | [Wgpu.QueryKind](#wgpuquerykind) | What its queries record. |
| `Count` | uint | How many queries it holds. |

| Method | Returns | Does |
|---|---|---|
| `Read(uint first, uint count)` | ulong[] | The raw results of `count` queries from query `first` (all when omitted): one value per timestamp or occlusion query, five per pipeline-statistics query. |
| `ReadAsync(uint first, uint count, CancellationToken cancellationToken)` | Task[ulong[]] | Read on a thread of its own: returns at once with a task whose result is the values. |
| `ReadNanoseconds(uint first, uint count)` | double[] | Timestamp results in nanoseconds. |
| `ReadNanosecondsAsync(uint first, uint count, CancellationToken cancellationToken)` | Task[double[]] | ReadNanoseconds on a thread of its own: returns at once with a task whose result is the nanoseconds. |
| `Destroy()` | void | Frees the query set now. |

#### Errors

Its reads: `WgpuRange`, `WgpuNotTimestamps`, `WgpuBuffer`, `WgpuQueryResolve`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`. A method throws them as the [Error Reference](Error-Reference.md) describes.

### Wgpu.RenderPass

A render pass being recorded into an encoder.

Returned by the `BeginRenderPass` method of [Wgpu.CommandEncoder](#wgpucommandencoder).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The pass's label. |
| `IsEnded` | bool | Whether End was called; the encoder takes no other command until it is. |

| Method | Returns | Does |
|---|---|---|
| `SetPipeline(Wgpu.RenderPipeline pipeline)` | void | Sets the pipeline the next draws use. |
| `SetBindGroup(uint index, Wgpu.BindGroup bindGroup)` | void | Sets bind group `index`. |
| `SetVertexBuffer(uint slot, Wgpu.Buffer buffer)` | void | Feeds vertex input `slot` from a buffer. |
| `SetIndexBuffer(Wgpu.Buffer buffer)` | void | Sets the index buffer: UInt16 or UInt32 elements. |
| `Draw(uint vertexCount, uint instanceCount, uint firstVertex, uint firstInstance)` | void | Draws `vertexCount` vertices of `instanceCount` instances (1 when omitted). |
| `DrawIndexed(uint indexCount, uint instanceCount, uint firstIndex, int baseVertex, uint firstInstance)` | void | Draws `indexCount` indices of `instanceCount` instances (1 when omitted). |
| `DrawIndirect(Wgpu.Buffer buffer, ulong offset)` | void | Draws with the arguments a buffer holds at byte `offset`. |
| `DrawIndexedIndirect(Wgpu.Buffer buffer, ulong offset)` | void | Draws indexed with the arguments a buffer holds at byte `offset`. |
| `SetViewport(float x, float y, float width, float height, float minDepth, float maxDepth)` | void | Sets the viewport, with depths 0 to 1 when omitted. |
| `SetScissorRect(uint x, uint y, uint width, uint height)` | void | Limits drawing to a rectangle. |
| `SetBlendConstant(double r, double g, double b, double a)` | void | Sets the constant color blend factors read. |
| `SetStencilReference(uint reference)` | void | Sets the stencil reference value. |
| `BeginOcclusionQuery(uint index)` | void | Starts counting samples into occlusion query `index` of the pass's query set. |
| `EndOcclusionQuery()` | void | Stops counting samples. |
| `PushDebugGroup(string label)` | void | Opens a named group of commands for graphics debuggers. |
| `PopDebugGroup()` | void | Closes the innermost debug group. |
| `InsertDebugMarker(string label)` | void | Marks a point in the pass for graphics debuggers. |
| `End()` | void | Ends the pass, so its encoder takes commands again. |

#### Errors

`WgpuPassEnded`, and `WgpuIndexType` for an index buffer of neither UInt16 nor UInt32. A method throws them as the [Error Reference](Error-Reference.md) describes. wgpu checks the pass's commands when its encoder finishes, so an invalid command surfaces there as `WgpuFinish`.

### Wgpu.RenderPipeline

A render pipeline, built for one device: a vertex and a fragment entry point with their vertex inputs, bindings and target formats.

Written by [New-WgpuRenderPipeline](Render-Cmdlets.md#new-wgpurenderpipeline).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The pipeline's label. |
| `VertexEntryPoint` | string | The vertex entry point. |
| `FragmentEntryPoint` | string | The fragment entry point. |
| `VertexInputs` | string[] | The vertex inputs, in location order, each fed from a buffer of its own. |
| `Bindings` | [Wgpu.ShaderBinding](#wgpushaderbinding)[] | The bindings both stages use, in group and binding order. |
| `TargetFormats` | string[] | The formats of its color targets. |
| `DepthFormat` | string | The format of its depth target, or `$null`. |
| `SampleCount` | uint | Samples per pixel. |
| `Adapter` | string | The adapter of the device it was built on. |

| Method | Returns | Does |
|---|---|---|
| `BindGroup(uint group, object resources)` | [Wgpu.BindGroup](#wgpubindgroup) | The bind group `group` of this pipeline, binding `resources`. |

### Wgpu.Sampler

How a shader samples textures.

Written by [New-WgpuSampler](Texture-Cmdlets.md#new-wgpusampler).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The sampler's label. |
| `MagFilter` | [Wgpu.FilterMode](#wgpufiltermode) | The filter when a texel covers more than a pixel. |
| `MinFilter` | [Wgpu.FilterMode](#wgpufiltermode) | The filter when a texel covers less than a pixel. |
| `MipmapFilter` | [Wgpu.FilterMode](#wgpufiltermode) | The filter between mip levels. |
| `AddressModeU` | [Wgpu.AddressMode](#wgpuaddressmode) | What it reads past the left and right edges. |
| `AddressModeV` | [Wgpu.AddressMode](#wgpuaddressmode) | What it reads past the top and bottom edges. |
| `AddressModeW` | [Wgpu.AddressMode](#wgpuaddressmode) | What it reads past the front and back of a 3D texture. |
| `Compare` | [Wgpu.CompareFunction](#wgpucomparefunction)? | The comparison of a comparison sampler; `$null` for a filtering one. |
| `Anisotropy` | ushort | The anisotropic filtering clamp, 1 to 16. |

### Wgpu.Shader

A shader parsed and validated by naga, independent of any device: it compiles on whichever device a pipeline is made on.

Written by [New-WgpuShader](Shader-Cmdlets.md#new-wgpushader). Returned by the `CreateShader` method of [Wgpu.Device](#wgpudevice).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The shader's label. |
| `Language` | [Wgpu.ShaderLanguage](#wgpushaderlanguage) | The language it was written in. |
| `EntryPoints` | [Wgpu.ShaderEntryPoint](#wgpushaderentrypoint)[] | Its entry points. |
| `Bindings` | [Wgpu.ShaderBinding](#wgpushaderbinding)[] | Its resource bindings, in group and binding order. |
| `Overrides` | [Wgpu.ShaderOverride](#wgpushaderoverride)[] | Its pipeline-overridable constants. |
| `Source` | string | Its source text; `$null` for SPIR-V. |

| Method | Returns | Does |
|---|---|---|
| `[Wgpu.Shader]::new(string source, Wgpu.ShaderLanguage language, Wgpu.ShaderStage stage)` | [Wgpu.Shader](#wgpushader) | Parses and validates `source`: WGSL, or GLSL of `stage` (Compute when omitted) when `language` says so or the text starts with `#version`. |
| `[Wgpu.Shader]::FromFile(string path, Wgpu.ShaderLanguage language, Wgpu.ShaderStage stage)` | [Wgpu.Shader](#wgpushader) | Parses and validates a shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl, .mesh, .task). |
| `[Wgpu.Shader]::FromSpirV(object words)` | [Wgpu.Shader](#wgpushader) | Parses and validates SPIR-V given as `byte[]` or `uint[]`. |
| `ConvertTo(Wgpu.ShaderTarget target, object entry)` | object | The shader written in `target`, for `entry` when given: text, or `uint[]` for SPIR-V. |
| `Info()` | [Wgpu.ShaderInfo](#wgpushaderinfo) | Everything the shader declares. |

#### Errors

The constructor: `WgpuSpirV`, `WgpuStage` and `WgpuShaderInvalid`. `FromFile`: `WgpuShaderFile`, `WgpuSpirV`, `WgpuStage` and `WgpuShaderInvalid`. `FromSpirV`: `WgpuSpirV` and `WgpuShaderInvalid`, and PWRS's `PwrsConversionError` for words that are not numbers. `ConvertTo`: `WgpuNoEntryPoint`, `WgpuEntryPointAmbiguous`, `WgpuOverrides`, `WgpuVersion` and `WgpuTranslate`. A method throws them as the [Error Reference](Error-Reference.md) describes.

### Wgpu.ShaderBinding

A resource binding a shader declares.

Held in [Wgpu.ComputePipeline](#wgpucomputepipeline).Bindings, [Wgpu.RenderPipeline](#wgpurenderpipeline).Bindings, [Wgpu.Shader](#wgpushader).Bindings and [Wgpu.ShaderInfo](#wgpushaderinfo).Bindings.

| Property | Type | Holds |
|---|---|---|
| `Group` | uint | The `@group`. |
| `Binding` | uint | The `@binding`. |
| `Name` | string | The variable's name. |
| `Kind` | [Wgpu.BindingKind](#wgpubindingkind) | What it holds. |
| `WgslType` | string | Its type, written as WGSL. |
| `ElementType` | [Wgpu.ElementType](#wgpuelementtype)? | For a buffer of numbers, the type of each number. |
| `Size` | ulong | Bytes of the fixed part of a buffer binding; a runtime-sized array adds `Stride` per element. |
| `Stride` | ulong | Bytes per element of a runtime-sized or fixed array. |
| `Count` | uint | The number of resources in a binding array; 0 for a single resource. |

### Wgpu.ShaderDiagnostic

A problem naga or a device found in a shader.

Written by [Test-WgpuShader](Shader-Cmdlets.md#test-wgpushader).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The shader it is in: the path of the file it was read from, or Wgpu shader for text and SPIR-V. |
| `Severity` | string | Error. |
| `Message` | string | What is wrong. |
| `Line` | uint | The line it is on, from 1; 0 where the source has no lines, as with SPIR-V. |
| `Column` | uint | The column it starts at, from 1. |
| `Length` | uint | How many characters it covers. |
| `SourceLine` | string | The text of that line; `$null` where the source has no lines. |
| `Detail` | string | The full report, with every labeled span and note. |

### Wgpu.ShaderEntryPoint

An entry point of a shader.

Held in [Wgpu.Shader](#wgpushader).EntryPoints and [Wgpu.ShaderInfo](#wgpushaderinfo).EntryPoints.

| Property | Type | Holds |
|---|---|---|
| `Name` | string | Its name. |
| `Stage` | [Wgpu.ShaderStage](#wgpushaderstage) | The stage it runs in. |
| `WorkgroupSize` | uint[] | Invocations per workgroup in x, y and z, for compute, task and mesh stages. |
| `Bindings` | string[] | The names of the bindings it uses, in group and binding order. |

### Wgpu.ShaderInfo

What a shader declares: entry points, bindings and override constants.

Written by [Get-WgpuShaderInfo](Shader-Cmdlets.md#get-wgpushaderinfo). Returned by the `Info` method of [Wgpu.Shader](#wgpushader).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The shader's label: the path of the file it was read from, the label it was made with, or Wgpu shader for text and SPIR-V. |
| `Language` | [Wgpu.ShaderLanguage](#wgpushaderlanguage) | The language it was written in. |
| `EntryPoints` | [Wgpu.ShaderEntryPoint](#wgpushaderentrypoint)[] | Its entry points. |
| `Bindings` | [Wgpu.ShaderBinding](#wgpushaderbinding)[] | Its resource bindings, in group and binding order. |
| `Overrides` | [Wgpu.ShaderOverride](#wgpushaderoverride)[] | Its pipeline-overridable constants. |

### Wgpu.ShaderOverride

A pipeline-overridable constant a shader declares.

Held in [Wgpu.Shader](#wgpushader).Overrides and [Wgpu.ShaderInfo](#wgpushaderinfo).Overrides.

| Property | Type | Holds |
|---|---|---|
| `Name` | string | Its name; `$null` for one known only by its `@id`. |
| `Id` | uint? | Its `@id`, where it has one. |
| `WgslType` | string | Its type, written as WGSL. |
| `Default` | double? | Its default value, where it has one that is a literal. |

### Wgpu.Texture

A texture held in Rust, on one device.

Written by [Compare-WgpuTexture](Texture-Cmdlets.md#compare-wgputexture), [Convert-WgpuTexture](Texture-Cmdlets.md#convert-wgputexture), [ConvertFrom-WgpuCubeMap](Texture-Cmdlets.md#convertfrom-wgpucubemap), [ConvertTo-WgpuCubeMap](Texture-Cmdlets.md#convertto-wgpucubemap), [ConvertTo-WgpuIrradianceMap](Texture-Cmdlets.md#convertto-wgpuirradiancemap), [ConvertTo-WgpuNormalMap](Texture-Cmdlets.md#convertto-wgpunormalmap), [ConvertTo-WgpuSpecularMap](Texture-Cmdlets.md#convertto-wgpuspecularmap), [Import-WgpuImage](Texture-Cmdlets.md#import-wgpuimage), [Invoke-WgpuRender](Render-Cmdlets.md#invoke-wgpurender), [Join-WgpuTexture](Texture-Cmdlets.md#join-wgputexture), [New-WgpuBrdfTable](Texture-Cmdlets.md#new-wgpubrdftable), [New-WgpuTexture](Texture-Cmdlets.md#new-wgputexture) and [Resize-WgpuTexture](Texture-Cmdlets.md#resize-wgputexture).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The texture's label. |
| `Width` | uint | Width in texels. |
| `Height` | uint | Height in texels. |
| `DepthOrArrayLayers` | uint | Depth of a 3D texture, or the number of array layers of a 2D one. |
| `MipLevelCount` | uint | Mip levels. |
| `SampleCount` | uint | Samples per texel. |
| `Dimension` | [Wgpu.TextureDimension](#wgputexturedimension) | D1, D2 or D3. |
| `Format` | string | The format's name. |
| `Usage` | [Wgpu.TextureUsage](#wgputextureusage)[] | What it may be used for. |
| `ElementType` | [Wgpu.ElementType](#wgpuelementtype) | The type texels read and write as. |
| `Adapter` | string | The adapter of the device it lives on. |
| `IsCube` | bool | Whether it is a cube map: square layers, six to a cube. |
| `Notes` | string[] | What making it involved: how a file was read, decoded, converted or encoded, and what Basis Universal said. |
| `Harmonics` | [Wgpu.Harmonics](#wgpuharmonics) | An irradiance map's irradiance as spherical harmonics, which ConvertTo-WgpuIrradianceMap -Harmonics gives it; empty on any other texture. |

| Method | Returns | Does |
|---|---|---|
| `CreateView(object format, object dimension, uint baseMipLevel, uint mipLevelCount, uint baseArrayLayer, uint arrayLayerCount, Wgpu.TextureAspect aspect)` | [Wgpu.TextureView](#wgputextureview) | A view of the texture: another compatible format, a dimension (D1, D2, D2Array, Cube, CubeArray, D3), a range of mip levels and array layers, and an aspect. Everything omitted is the whole texture, and a cube map's whole cubes view as a cube, or as a cube array for more than one, when no dimension is given. |
| `Read(uint mipLevel, uint layer, Wgpu.TextureAspect aspect)` | object | The texels of mip level `mipLevel` of array layer `layer` (both 0 when omitted), rows packed, as the format's number type. |
| `ReadAsync(uint mipLevel, uint layer, Wgpu.TextureAspect aspect, CancellationToken cancellationToken)` | Task[object] | Read on a thread of its own: returns at once with a task whose result is the texels. |
| `Write(object data, uint mipLevel, uint layer, Wgpu.TextureAspect aspect)` | void | Writes texels into mip level `mipLevel` of array layer `layer` (both 0 when omitted), converted to the format's number type. |
| `Save(string path, Wgpu.Decoder decoder)` | void | Saves the texture to a file: a .dds or .ktx2 file whole, with every mip level, layer and cube face, and any other file as an image of mip level 0 of layer 0, in the format its extension names. A block-compressed texture is decoded for an image file by `decoder`, Cpu or Gpu, on the CPU when omitted. A relative path is relative to the process's working directory, as for any .NET method; Export-WgpuImage resolves PowerShell paths. |
| `SaveAsync(string path, Wgpu.Decoder decoder, CancellationToken cancellationToken)` | Task | Save on a thread of its own: returns at once with a task that ends when the file is written. |
| `GenerateMips(uint levels, Wgpu.MipFilter filter, object edge, double coverage, Wgpu.NormalMips normals)` | [Wgpu.Texture](#wgputexture) | Without `levels`, renders the texture's own mip levels 1 and on and returns nothing. With `levels`, returns a new texture with that many mip levels, 0 for the full chain down to 1x1: the texture's own levels kept and the rest rendered. `filter` makes each level: Box when omitted, from the level above; Triangle, Kaiser, Mitchell and Lanczos from level 0; Min and Max, the smallest or largest value of each channel among the texels under it. `edge` is what Triangle, Kaiser, Mitchell and Lanczos read past an edge of the image: Clamp when omitted, Wrap or Mirror, or a sampler's ClampToEdge, Repeat or MirrorRepeat. `coverage` scales each rendered level's alpha so the share of its texels with alpha at or above it, the alpha an alpha test passes at, stays level 0's: above 0 and at most 1. `normals` keeps a normal map's normals unit length: Renormalize scales each texel's normal back to length 1, and SlopeSpace filters their slopes. Levels are filtered as floating-point colors, in linear light for an sRGB format, and rounded once; a BC texture's levels are encoded on the GPU. |
| `GenerateMipsAsync(uint levels, Wgpu.MipFilter filter, object edge, double coverage, Wgpu.NormalMips normals, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | GenerateMips on a thread of its own: returns at once with a task whose result is the new texture, or `$null` without `levels`. |
| `Compress(string format, uint quality, uint effort)` | [Wgpu.Texture](#wgputexture) | A new texture with every mip level and layer encoded in `format`: a BC format, encoded on the GPU, or a Basis Universal format, such as BasisEtc1s or BasisUastc4x4, which Basis Universal encodes and the module transcodes to the best format the device holds. `quality`, 1 to 100, and `effort`, 0 to 10, are Import-WgpuImage's -Quality and -Effort. The new texture's Notes say what the encoding involved, Basis Universal's warnings among it. |
| `CompressAsync(string format, uint quality, uint effort, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Compress on a thread of its own: returns at once with a task whose result is the new texture. |
| `Resize(uint width, uint height, Wgpu.ResizeFilter filter)` | [Wgpu.Texture](#wgputexture) | A new texture of `width` by `height` texels, resized with `filter`: Box where a side shrinks and Bicubic where it grows when omitted. A side given as 0, or omitted, keeps the aspect ratio. The new texture keeps the format and as many mip levels as the new size's chain has, rendered again from level 0; its Notes say what was done. |
| `ResizeAsync(uint width, uint height, Wgpu.ResizeFilter filter, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Resize on a thread of its own: returns at once with a task whose result is the new texture. |
| `Crop(uint x, uint y, uint width, uint height)` | [Wgpu.Texture](#wgputexture) | A new texture of the `width` by `height` texels whose top left is at (`x`, `y`). |
| `CropAsync(uint x, uint y, uint width, uint height, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Crop on a thread of its own: returns at once with a task whose result is the new texture. |
| `Pad(uint width, uint height, Wgpu.Anchor anchor, double[] color)` | [Wgpu.Texture](#wgputexture) | A new texture on a `width` by `height` canvas: the texture placed by `anchor`, Center when omitted, and the rest filled with `color`, red, green, blue and alpha from 0 to 1, transparent black when omitted. |
| `PadAsync(uint width, uint height, Wgpu.Anchor anchor, double[] color, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Pad on a thread of its own: returns at once with a task whose result is the new texture. |
| `Flip(Wgpu.FlipDirection direction)` | [Wgpu.Texture](#wgputexture) | A new texture mirrored: Horizontal swaps left and right, and Vertical top and bottom. |
| `FlipAsync(Wgpu.FlipDirection direction, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Flip on a thread of its own: returns at once with a task whose result is the new texture. |
| `Rotate(uint degrees)` | [Wgpu.Texture](#wgputexture) | A new texture turned `degrees` clockwise: 90, 180 or 270. |
| `RotateAsync(uint degrees, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Rotate on a thread of its own: returns at once with a task whose result is the new texture. |
| `Swizzle(string mask)` | [Wgpu.Texture](#wgputexture) | A new texture with its channels rearranged by `mask`: four letters of r, g, b, a, 0 and 1, one for each channel, as 'bgra' or 'rrr1'. An sRGB texture's channels move as the values it stores. |
| `SwizzleAsync(string mask, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Swizzle on a thread of its own: returns at once with a task whose result is the new texture. |
| `Premultiply()` | [Wgpu.Texture](#wgputexture) | A new texture with red, green and blue multiplied by alpha, in linear light for an sRGB texture. |
| `PremultiplyAsync(CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Premultiply on a thread of its own: returns at once with a task whose result is the new texture. |
| `Unpremultiply()` | [Wgpu.Texture](#wgputexture) | A new texture with red, green and blue divided by alpha where alpha is above 0, in linear light for an sRGB texture. |
| `UnpremultiplyAsync(CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Unpremultiply on a thread of its own: returns at once with a task whose result is the new texture. |
| `ConvertColorSpace(Wgpu.ColorSpace space)` | [Wgpu.Texture](#wgputexture) | A new texture with its texels converted to `space`, Linear or Srgb, so their colors look the same: a format with an sRGB form switches to that form, and any other keeps its format with its values encoded or decoded. |
| `ConvertColorSpaceAsync(Wgpu.ColorSpace space, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ConvertColorSpace on a thread of its own: returns at once with a task whose result is the new texture. |
| `AssignColorSpace(Wgpu.ColorSpace space)` | [Wgpu.Texture](#wgputexture) | A new texture with its texels relabeled as `space`, Linear or Srgb, without changing them: the format switches to that form. |
| `AssignColorSpaceAsync(Wgpu.ColorSpace space, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | AssignColorSpace on a thread of its own: returns at once with a task whose result is the new texture. |
| `Grayscale(object weights)` | [Wgpu.Texture](#wgputexture) | A new grayscale texture, weighed by `weights`: Rec709 in linear light when omitted, Rec601 on the stored values, or three numbers for red, green and blue in linear light. |
| `GrayscaleAsync(object weights, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Grayscale on a thread of its own: returns at once with a task whose result is the new texture. |
| `Invert()` | [Wgpu.Texture](#wgputexture) | A new texture with red, green and blue turned into 1 minus their stored values, as ImageMagick's -negate; alpha is kept. |
| `InvertAsync(CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Invert on a thread of its own: returns at once with a task whose result is the new texture. |
| `Adjust(double brightness, double contrast, double gamma, double exposure)` | [Wgpu.Texture](#wgputexture) | A new texture with `brightness` and `contrast`, from -100 to 100 as ImageMagick's -brightness-contrast, and `gamma`, as its -gamma, applied to the stored values, and `exposure` in stops to linear values; each is left out when omitted. |
| `AdjustAsync(double brightness, double contrast, double gamma, double exposure, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Adjust on a thread of its own: returns at once with a task whose result is the new texture. |
| `ToneMap(Wgpu.ToneMapCurve curve)` | [Wgpu.Texture](#wgputexture) | A new texture with its HDR colors mapped into the range from 0 to 1 by `curve`: Reinhard, AcesFilmic, AgX or Neutral. |
| `ToneMapAsync(Wgpu.ToneMapCurve curve, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ToneMap on a thread of its own: returns at once with a task whose result is the new texture. |
| `ColorKey(object key, double tolerance)` | [Wgpu.Texture](#wgputexture) | A new texture whose texels of the color `key` get alpha 0: red, green and blue from 0 to 1 as the texture stores them, rounded to its precision, or '#RRGGBB', each channel within `tolerance` (0, the same stored value, when omitted). Other texels keep their alpha. |
| `ColorKeyAsync(object key, double tolerance, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ColorKey on a thread of its own: returns at once with a task whose result is the new texture. |
| `AlphaThreshold(double threshold)` | [Wgpu.Texture](#wgputexture) | A new texture whose texels with alpha below `threshold` get alpha 0 and the rest alpha 1, as an alpha test would: above 0 and at most 1. |
| `AlphaThresholdAsync(double threshold, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | AlphaThreshold on a thread of its own: returns at once with a task whose result is the new texture. |
| `FixAlphaBorder(uint reach)` | [Wgpu.Texture](#wgputexture) | A new texture whose texels with alpha 0 take the color of the nearest texel with alpha above 0, their alpha left at 0, so filtering and mip levels blend no dark or keyed color in at a cutout's edge: every such texel, or those within `reach` texels of a texel with alpha above 0. |
| `FixAlphaBorderAsync(uint reach, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | FixAlphaBorder on a thread of its own: returns at once with a task whose result is the new texture. |
| `Blur(double[] sigma, Wgpu.FilterEdge edge)` | [Wgpu.Texture](#wgputexture) | A new texture blurred by a Gaussian of standard deviation `sigma` in texels, or of two, across and down, in linear values weighed by alpha, reading past the edge as `edge` says, Clamp when omitted. |
| `BlurAsync(double[] sigma, Wgpu.FilterEdge edge, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Blur on a thread of its own: returns at once with a task whose result is the new texture. |
| `Sharpen(double amount, double radius, double threshold, Wgpu.FilterEdge edge)` | [Wgpu.Texture](#wgputexture) | A new texture sharpened by an unsharp mask: `amount` times the difference between each texel and a Gaussian blur of it, of standard deviation `radius` texels, 1 when omitted, added back in each channel where it is at least `threshold`, 0 when omitted, in linear values weighed by alpha, reading past the edge as `edge` says, Clamp when omitted. |
| `SharpenAsync(double amount, double radius, double threshold, Wgpu.FilterEdge edge, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Sharpen on a thread of its own: returns at once with a task whose result is the new texture. |
| `FindEdges(Wgpu.EdgeOperator method, double[] threshold, double sigma, Wgpu.FilterEdge edge)` | [Wgpu.Texture](#wgputexture) | A new texture of the texture's edges, as `method` finds them: the size of each channel's Sobel, Scharr or Prewitt gradient, where a step from 0 to 1 reads 1, or of its Laplacian, in the values the texture stores; or Canny's edges of the luminance, 1 on an edge and 0 elsewhere, kept above the low of `threshold`'s two numbers where they link to one above the high, 0.1 and 0.2 when omitted, after a Gaussian of standard deviation `sigma` texels, 1 when omitted. Alpha is kept, and the texels past the edge are read as `edge` says, Clamp when omitted; for Canny by the smoothing, its gradient after repeating the edge texel. |
| `FindEdgesAsync(Wgpu.EdgeOperator method, double[] threshold, double sigma, Wgpu.FilterEdge edge, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | FindEdges on a thread of its own: returns at once with a task whose result is the new texture. |
| `ReconstructZ()` | [Wgpu.Texture](#wgputexture) | A new texture with each normal's Z, in blue, rebuilt from its X and Y, in red and green, as for a normal map stored in two channels; the texture needs a blue channel. A format that holds no negative values stores a normal n as n / 2 + 1 / 2, and any other stores n. |
| `ReconstructZAsync(CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ReconstructZ on a thread of its own: returns at once with a task whose result is the new texture. |
| `Normalize()` | [Wgpu.Texture](#wgputexture) | A new texture with each normal scaled to length 1. |
| `NormalizeAsync(CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Normalize on a thread of its own: returns at once with a task whose result is the new texture. |
| `FlipGreen()` | [Wgpu.Texture](#wgputexture) | A new texture with each normal's green flipped, between the convention glTF reads, green up, and DirectX's, green down. |
| `FlipGreenAsync(CancellationToken cancellationToken)` | Task[Wgpu.Texture] | FlipGreen on a thread of its own: returns at once with a task whose result is the new texture. |
| `ToNormalMap(double strength, Wgpu.HeightChannel channel, Wgpu.NormalKernel kernel, Wgpu.HeightEdge edge, Wgpu.NormalAlpha alpha)` | [Wgpu.Texture](#wgputexture) | A normal map made from the texture's heights, as ConvertTo-WgpuNormalMap makes it: `strength`, 1 when omitted, is the slope a height step of 1 between neighboring texels gives; `channel` is where the heights are read, Red when omitted; `kernel` takes the slopes, Prewitt when omitted; `edge` is what is read past the edges, Continue when omitted; and `alpha` is what alpha holds, One when omitted. The normal map is Rgba8Unorm with green up, and has the height map's mip levels, each texel's normal scaled back to length 1; its Notes say what was done. |
| `ToNormalMapAsync(double strength, Wgpu.HeightChannel channel, Wgpu.NormalKernel kernel, Wgpu.HeightEdge edge, Wgpu.NormalAlpha alpha, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ToNormalMap on a thread of its own: returns at once with a task whose result is the normal map. |
| `ToCubeMap(Wgpu.CubeLayout layout, double yaw, uint faceSize, object format)` | [Wgpu.Texture](#wgputexture) | A cube map made from the texture, one image holding its faces, as ConvertTo-WgpuCubeMap makes it: `layout` names the layout or projection the image's shape does not tell apart; `yaw` turns a panorama, an angular map or an octahedral map about the vertical axis by degrees, +X toward -Z; and `faceSize` sets the faces' size for one of those three. The cube keeps the texture's format, or takes `format`, and has mip level 0 alone; its Notes say what was done. |
| `ToCubeMapAsync(Wgpu.CubeLayout layout, double yaw, uint faceSize, object format, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ToCubeMap on a thread of its own: returns at once with a task whose result is the cube map. |
| `[Wgpu.Texture]::CubeFromFaces(Wgpu.Texture[] faces, object format)` | [Wgpu.Texture](#wgputexture) | A cube map of six textures, its faces in the order +X, -X, +Y, -Y, +Z, -Z, as ConvertTo-WgpuCubeMap makes it: square 2D textures of one size and format, their texels and the mip levels all six have kept as they are, in their format or `format`. |
| `[Wgpu.Texture]::CubeFromFacesAsync(Wgpu.Texture[] faces, object format, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | CubeFromFaces on a thread of its own: returns at once with a task whose result is the cube map. |
| `ToLayout(Wgpu.CubeLayout layout, double yaw, uint faceSize, uint mipLevel, object format)` | [Wgpu.Texture](#wgputexture) | The cube map written into one image in `layout`, as ConvertFrom-WgpuCubeMap writes it: a layout's cells take the faces as they are, and an AngularMap or an Octahedral map is resampled, its faces `faceSize` texels across, the cube's when omitted, turned by `yaw` degrees about the vertical axis, +X toward -Z. The image comes from mip level `mipLevel`, 0 when omitted, in the cube's format or `format`. |
| `ToLayoutAsync(Wgpu.CubeLayout layout, double yaw, uint faceSize, uint mipLevel, object format, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ToLayout on a thread of its own: returns at once with a task whose result is the image. |
| `ToPanorama(double yaw, uint faceSize, uint mipLevel, object format)` | [Wgpu.Texture](#wgputexture) | The cube map resampled into a panorama, as ConvertFrom-WgpuCubeMap -Panorama makes it: center +X and left quarter -Z, 4 faces wide and 2 high, the faces `faceSize` texels across, the cube's when omitted, turned by `yaw` degrees about the vertical axis, +X toward -Z. The panorama comes from mip level `mipLevel`, 0 when omitted, in the cube's format or `format`. |
| `ToPanoramaAsync(double yaw, uint faceSize, uint mipLevel, object format, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ToPanorama on a thread of its own: returns at once with a task whose result is the panorama. |
| `ToIrradianceMap(uint faceSize, bool harmonics, object format)` | [Wgpu.Texture](#wgputexture) | The irradiance cube of the texture, an environment cube map or a panorama or layout its shape tells apart, as ConvertTo-WgpuIrradianceMap makes it: faces `faceSize` texels across, 32 when omitted, in Rgba16Float or `format`, and with `harmonics` the irradiance's spherical harmonics as its Harmonics. |
| `ToIrradianceMapAsync(uint faceSize, bool harmonics, object format, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ToIrradianceMap on a thread of its own: returns at once with a task whose result is the irradiance map. |
| `ToSpecularMap(Wgpu.RoughnessMapping mapping, uint samples, uint mipLevels, object format)` | [Wgpu.Texture](#wgputexture) | The prefiltered specular cube of the texture, an environment cube map or a panorama or layout its shape tells apart, as ConvertTo-WgpuSpecularMap makes it: `mapping` steps the mip levels through roughness, Linear when omitted, by `samples` samples a texel, 1024 when omitted, into `mipLevels` levels, down to 2x2 for Linear and 16x16 for Quadratic when omitted, in Rgba16Float or `format`. |
| `ToSpecularMapAsync(Wgpu.RoughnessMapping mapping, uint samples, uint mipLevels, object format, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | ToSpecularMap on a thread of its own: returns at once with a task whose result is the specular map. |
| `[Wgpu.Texture]::NewBrdfTable(uint size, uint samples, object format, Wgpu.Device device)` | [Wgpu.Texture](#wgputexture) | The BRDF table split-sum image-based lighting reads, as New-WgpuBrdfTable makes it: `size` texels a side, 128 when omitted, by `samples` samples a texel, 1024 when omitted, in Rg16Float or `format`, on `device`, the default device when omitted. |
| `[Wgpu.Texture]::NewBrdfTableAsync(uint size, uint samples, object format, Wgpu.Device device, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | NewBrdfTable on a thread of its own: returns at once with a task whose result is the table. |
| `Compare(Wgpu.Texture reference)` | [Wgpu.TextureComparison](#wgputexturecomparison)[] | How the texture differs from `reference`, as Compare-WgpuTexture writes it: for each mip level both have, each layer and each channel both store, the mean squared error, PSNR and SSIM. The textures need the same dimension, width, height and layers, and an NV12 or P010 texture compares only with another NV12 or P010 texture. |
| `CompareAsync(Wgpu.Texture reference, CancellationToken cancellationToken)` | Task[Wgpu.TextureComparison[]] | Compare on a thread of its own: returns at once with a task whose result is the rows. |
| `Difference(Wgpu.Texture reference, Wgpu.TextureChannel channel)` | [Wgpu.Texture](#wgputexture) | A picture of how the texture differs from `reference`, as Compare-WgpuTexture -Difference makes it: an Rgba8Unorm texture whose texels show the error on a log scale, black at 2^-12 or less and full at 1 or more, red, green and blue in their own colors, or `channel` as gray. Its Notes say what it shows. |
| `DifferenceAsync(Wgpu.Texture reference, Wgpu.TextureChannel channel, CancellationToken cancellationToken)` | Task[Wgpu.Texture] | Difference on a thread of its own: returns at once with a task whose result is the picture. |
| `Measure(uint bins, double[] range)` | [Wgpu.TextureMeasurement](#wgputexturemeasurement)[] | The texture's values, as Measure-WgpuTexture measures them: for each mip level, layer and channel, the minimum, maximum, mean, population standard deviation and a histogram of `bins` bins splitting `range`, the low end and the high end; 256 bins from 0 to 1 when omitted. |
| `MeasureAsync(uint bins, double[] range, CancellationToken cancellationToken)` | Task[Wgpu.TextureMeasurement[]] | Measure on a thread of its own: returns at once with a task whose result is the rows. |
| `[Wgpu.Texture]::Join(object red, object green, object blue, object alpha, object format)` | [Wgpu.Texture](#wgputexture) | A new texture with its channels from `red`, `green`, `blue` and `alpha`, as Join-WgpuTexture takes them: each a texture, whose red channel it reads, a texture and a channel letter as a pair, or a number; 0 for an omitted color channel and 1 for an omitted alpha. It is Rgba8Unorm, or `format`: a color format, a BC format or a Basis Universal format. |
| `Destroy()` | void | Frees the texture's GPU memory now; the object stops working. |

#### Errors

`CreateView`: `WgpuUnknownFormat`, `WgpuViewDimension` and `WgpuTextureView`. `Read` and `ReadAsync`: `WgpuAspect`, `WgpuBuffer`, `WgpuTextureRead`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuRead`, `WgpuMap` and `WgpuStopped`. `Write`: `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuAspect`, `WgpuTexelCount` and `WgpuTextureWrite`. `Save` and `SaveAsync`: `WgpuImageExtension`, `WgpuImageFormat`, `WgpuDecoder`, `WgpuTextureDecode`, `WgpuConvert`, `WgpuImageShape`, `WgpuTextureFile`, `WgpuImageWrite` and the errors of reading the texture back. `GenerateMips` and `GenerateMipsAsync`: `WgpuMipLevels`, `WgpuTextureUsage`, `WgpuConvert`, `WgpuCompress` and `WgpuNormalMap`, the errors of reading the texture back, and from writing its levels or making the new texture, `WgpuTexture`, `WgpuTexelCount` and `WgpuTextureWrite`. `Compress` and `CompressAsync`: `WgpuUnknownFormat`, `WgpuCompressFormat`, `WgpuQuality`, `WgpuEffort`, `WgpuTextureFormat`, `WgpuConvert`, `WgpuCompress`, `WgpuBasisEncode`, `WgpuKtx2` and `WgpuTextureFile`, and the errors of reading the texture back and making the new one. `Resize`, `Crop`, `Pad`, `Flip`, `Rotate` and their Async forms: `WgpuResize`, `WgpuCrop`, `WgpuPad`, `WgpuRotate` and `WgpuTransform`, as [Resize-WgpuTexture](Texture-Cmdlets.md#resize-wgputexture) and [Convert-WgpuTexture](Texture-Cmdlets.md#convert-wgputexture) raise them for the same change, and the errors of reading the texture back, decoding its texels and encoding them again, and making the new texture that [Resize-WgpuTexture](Texture-Cmdlets.md#resize-wgputexture) lists. `Swizzle`, `Premultiply`, `Unpremultiply`, `ConvertColorSpace`, `AssignColorSpace`, `Grayscale`, `Invert`, `Adjust`, `ToneMap` and their Async forms: `WgpuSwizzle`, `WgpuColorSpace`, `WgpuGrayscale`, `WgpuAdjust` and `WgpuTransform`, as Convert-WgpuTexture raises them for the same change, and the same errors of reading, decoding, encoding and making. `ReconstructZ`, `Normalize`, `FlipGreen` and their Async forms: `WgpuNormalMap`, as Convert-WgpuTexture raises it for the same change, and the same errors of reading, decoding, encoding and making. `Blur`, `Sharpen`, `FindEdges` and their Async forms: `WgpuBlur`, `WgpuSharpen`, `WgpuEdges` and `WgpuFilter`, as Convert-WgpuTexture raises them for the same filter, and the same errors of reading, decoding, encoding and making. `ToNormalMap` and `ToNormalMapAsync`: the errors [ConvertTo-WgpuNormalMap](Texture-Cmdlets.md#convertto-wgpunormalmap) lists. `ToCubeMap`, `CubeFromFaces` and their Async forms: the errors [ConvertTo-WgpuCubeMap](Texture-Cmdlets.md#convertto-wgpucubemap) lists. `ToLayout`, `ToPanorama` and their Async forms: the errors [ConvertFrom-WgpuCubeMap](Texture-Cmdlets.md#convertfrom-wgpucubemap) lists. `ToIrradianceMap`, `ToSpecularMap` and their Async forms: the errors [ConvertTo-WgpuIrradianceMap](Texture-Cmdlets.md#convertto-wgpuirradiancemap) and [ConvertTo-WgpuSpecularMap](Texture-Cmdlets.md#convertto-wgpuspecularmap) list. `NewBrdfTable` and `NewBrdfTableAsync`: the errors [New-WgpuBrdfTable](Texture-Cmdlets.md#new-wgpubrdftable) lists. `Join`: the errors [Join-WgpuTexture](Texture-Cmdlets.md#join-wgputexture) lists. `Compare`, `Difference` and their Async forms: the errors [Compare-WgpuTexture](Texture-Cmdlets.md#compare-wgputexture) lists. `Measure` and `MeasureAsync`: the errors [Measure-WgpuTexture](Texture-Cmdlets.md#measure-wgputexture) lists, and PWRS's `PwrsConversionError` for a range that is not numbers. A method throws them as the [Error Reference](Error-Reference.md) describes.

### Wgpu.TextureComparison

What Compare-WgpuTexture writes: how one channel of one mip level and layer of a texture differs from the reference's.

Written by [Compare-WgpuTexture](Texture-Cmdlets.md#compare-wgputexture). Returned by the `Compare` method of [Wgpu.Texture](#wgputexture).

| Property | Type | Holds |
|---|---|---|
| `Level` | uint | The mip level. |
| `Layer` | uint | The array layer or cube face, or the depth slice of a 3D texture. |
| `Channel` | [Wgpu.TextureChannel](#wgputexturechannel) | The channel. |
| `MSE` | double | The mean squared error: the mean of the squared differences between the texture's values and the reference's. |
| `PSNR` | double | The peak signal-to-noise ratio in decibels, 10 log10(peak^2 / MSE): the peak is 1 for values from 0 to 1, 2 for values from -1 to 1, and for a float texture the reference channel's largest value, or 1 when that is 0 or below; +Infinity when the values match. |
| `SSIM` | double? | The structural similarity, Wang et al.'s SSIM with a Gaussian window of sigma 1.5 and the peak as its data range: 1 when the values match; $null for a level under 11 texels on a side. |

### Wgpu.TextureFormatInfo

A format and what it can do on one adapter.

Written by [Get-WgpuTextureFormat](Texture-Cmdlets.md#get-wgputextureformat).

| Property | Type | Holds |
|---|---|---|
| `Name` | string | The format's name, as -Format takes it. |
| `BlockSize` | uint? | Bytes per block (per texel for an uncompressed format); `$null` for a combined depth-stencil format. |
| `BlockWidth` | uint | Texels across a block: 1 for uncompressed formats. |
| `BlockHeight` | uint | Texels down a block. |
| `Components` | uint | Channels per texel. |
| `IsCompressed` | bool | Whether it is block-compressed. |
| `IsDepthStencil` | bool | Whether it holds depth or stencil. |
| `IsSrgb` | bool | Whether it stores sRGB-encoded color. |
| `ElementType` | [Wgpu.ElementType](#wgpuelementtype) | The type its texels read and write as. |
| `RequiredFeatures` | string[] | The features a device needs to use it. |
| `AllowedUsages` | [Wgpu.TextureUsage](#wgputextureusage)[] | What a texture of this format may be used for on the adapter. |
| `Capabilities` | string[] | What the adapter can do with it beyond its usages: Filterable, Blendable, MultisampleX2..X16, MultisampleResolve, StorageReadOnly, StorageWriteOnly, StorageReadWrite, StorageAtomic. |

### Wgpu.TextureMeasurement

What Measure-WgpuTexture writes: the values of one channel of one mip level and layer of a texture.

Written by [Measure-WgpuTexture](Texture-Cmdlets.md#measure-wgputexture). Returned by the `Measure` method of [Wgpu.Texture](#wgputexture).

| Property | Type | Holds |
|---|---|---|
| `Level` | uint | The mip level. |
| `Layer` | uint | The array layer or cube face, or the depth slice of a 3D texture. |
| `Channel` | [Wgpu.TextureChannel](#wgputexturechannel) | The channel. |
| `Minimum` | double | The smallest value; NaN when a value is NaN. |
| `Maximum` | double | The largest value; NaN when a value is NaN. |
| `Average` | double | The mean; NaN when a value is NaN. |
| `StandardDeviation` | double | The population standard deviation, the square root of the mean squared distance from the mean; NaN when a value is NaN. |
| `Histogram` | ulong[] | How many values each bin counts, the bins splitting the range evenly; values below it count in the first bin, values above it in the last, and NaN in none. |

### Wgpu.TextureView

A view of a texture, bound in place of the whole texture.

Returned by the `CreateView` method of [Wgpu.Texture](#wgputexture).

| Property | Type | Holds |
|---|---|---|
| `Label` | string | The texture's label. |
| `Format` | string | The format the view reads the texels as. |

## Enumerations

### Wgpu.AddressMode

What a sampler reads past a texture's edge.

| Value | Number | Means |
|---|---|---|
| `ClampToEdge` | 0 | The edge texel. |
| `Repeat` | 1 | The texture again. |
| `MirrorRepeat` | 2 | The texture mirrored. |
| `ClampToBorder` | 3 | The border color (feature AddressModeClampToBorder). |

### Wgpu.Anchor

Where a crop to a size, or a texture on a larger canvas, is placed.

| Value | Number | Means |
|---|---|---|
| `TopLeft` | 0 | The top left corner. |
| `Top` | 1 | The middle of the top edge. |
| `TopRight` | 2 | The top right corner. |
| `Left` | 3 | The middle of the left edge. |
| `Center` | 4 | The middle. |
| `Right` | 5 | The middle of the right edge. |
| `BottomLeft` | 6 | The bottom left corner. |
| `Bottom` | 7 | The middle of the bottom edge. |
| `BottomRight` | 8 | The bottom right corner. |

### Wgpu.Backend

A graphics API wgpu drives.

| Value | Number | Means |
|---|---|---|
| `Vulkan` | 0 | Vulkan on Windows, Linux and FreeBSD, and on macOS through MoltenVK. |
| `Metal` | 1 | Metal on macOS. |
| `Dx12` | 2 | Direct3D 12 on Windows. |
| `Gl` | 3 | OpenGL and OpenGL ES; on Windows only when named, never by default. |
| `Noop` | 4 | A backend that runs nothing, for exercising a script without a GPU. |

### Wgpu.BindingKind

What a shader binding holds.

| Value | Number | Means |
|---|---|---|
| `UniformBuffer` | 0 | `var<uniform>`. |
| `StorageBuffer` | 1 | `var<storage, read_write>`. |
| `ReadOnlyStorageBuffer` | 2 | `var<storage, read>`. |
| `Sampler` | 3 | `sampler`. |
| `ComparisonSampler` | 4 | `sampler_comparison`. |
| `Texture` | 5 | A sampled or depth texture. |
| `StorageTexture` | 6 | A storage texture. |
| `AccelerationStructure` | 7 | A ray-tracing acceleration structure. |
| `ExternalTexture` | 8 | An external texture. |

### Wgpu.BlendFactor

What a blend multiplies the fragment's color (source) or the color under it (destination) by.

| Value | Number | Means |
|---|---|---|
| `Zero` | 0 | 0. |
| `One` | 1 | 1. |
| `Src` | 2 | The fragment's color. |
| `OneMinusSrc` | 3 | 1 minus the fragment's color. |
| `SrcAlpha` | 4 | The fragment's alpha. |
| `OneMinusSrcAlpha` | 5 | 1 minus the fragment's alpha. |
| `Dst` | 6 | The color under the fragment. |
| `OneMinusDst` | 7 | 1 minus the color under the fragment. |
| `DstAlpha` | 8 | The alpha under the fragment. |
| `OneMinusDstAlpha` | 9 | 1 minus the alpha under the fragment. |
| `SrcAlphaSaturated` | 10 | The smaller of the fragment's alpha and 1 minus the alpha under it; 1 for alpha. |
| `Constant` | 11 | The blend constant a draw sets, by -BlendConstant or SetBlendConstant. |
| `OneMinusConstant` | 12 | 1 minus the blend constant. |
| `Src1` | 13 | The fragment's second color, `@blend_src(1)`; needs the DualSourceBlending feature. |
| `OneMinusSrc1` | 14 | 1 minus the fragment's second color. |
| `Src1Alpha` | 15 | The fragment's second color's alpha. |
| `OneMinusSrc1Alpha` | 16 | 1 minus the fragment's second color's alpha. |

### Wgpu.BlendMode

How a fragment's color combines with the color under it.

| Value | Number | Means |
|---|---|---|
| `Replace` | 0 | The fragment replaces the color. |
| `Alpha` | 1 | Alpha blending: the fragment over the color by its alpha. |
| `Additive` | 2 | The fragment adds to the color. |
| `Premultiplied` | 3 | Alpha blending of a fragment whose color is multiplied by its alpha. |
| `Constant` | 4 | The fragment and the color mixed by the blend constant a draw sets, by -BlendConstant or SetBlendConstant: the fragment times the constant plus the color times one minus it. |
| `Custom` | 5 | The factors and operations a custom blend names: -BlendSource, -BlendDestination and -BlendOperation, and their alpha forms. |

### Wgpu.BlendOperation

How a blend combines the fragment's weighted color with the weighted color under it.

| Value | Number | Means |
|---|---|---|
| `Add` | 0 | Source plus destination. |
| `Subtract` | 1 | Source minus destination. |
| `ReverseSubtract` | 2 | Destination minus source. |
| `Min` | 3 | The smaller of source and destination; both factors must be One. |
| `Max` | 4 | The larger of source and destination; both factors must be One. |

### Wgpu.BorderColor

The color a ClampToBorder sampler reads past the edge.

| Value | Number | Means |
|---|---|---|
| `TransparentBlack` | 0 | Black with zero alpha. |
| `OpaqueBlack` | 1 | Black. |
| `OpaqueWhite` | 2 | White. |
| `Zero` | 3 | Zero in every channel, alpha included, for any format. |

### Wgpu.BufferUsage

What a buffer may be used for.

| Value | Number | Means |
|---|---|---|
| `MapRead` | 0 | Mapped for reading on the CPU. |
| `MapWrite` | 1 | Mapped for writing on the CPU. |
| `CopySrc` | 2 | The source of a copy. |
| `CopyDst` | 3 | The destination of a copy or a write. |
| `Index` | 4 | An index buffer of a draw. |
| `Vertex` | 5 | A vertex buffer of a draw. |
| `Uniform` | 6 | A uniform buffer binding. |
| `Storage` | 7 | A storage buffer binding. |
| `Indirect` | 8 | The arguments of an indirect dispatch or draw. |
| `QueryResolve` | 9 | The destination of resolved query results. |
| `BlasInput` | 10 | Input to a bottom-level acceleration structure build. |
| `TlasInput` | 11 | Input to a top-level acceleration structure build. |

### Wgpu.ColorSpace

How a texture's texels encode color.

| Value | Number | Means |
|---|---|---|
| `Linear` | 0 | Linear values, which a shader reads as they are stored. |
| `Srgb` | 1 | sRGB-encoded values, which a shader reads decoded into linear ones. |

### Wgpu.CompareFunction

A comparison, for comparison samplers and depth and stencil tests.

| Value | Number | Means |
|---|---|---|
| `Never` | 0 | Never passes. |
| `Less` | 1 | Passes when the new value is less. |
| `Equal` | 2 | Passes when the values are equal. |
| `LessEqual` | 3 | Passes when the new value is less or equal. |
| `Greater` | 4 | Passes when the new value is greater. |
| `NotEqual` | 5 | Passes when the values differ. |
| `GreaterEqual` | 6 | Passes when the new value is greater or equal. |
| `Always` | 7 | Always passes. |

### Wgpu.CubeLayout

How one image holds a cube map's six faces: in square cells, as texassemble and Godot lay them out, or in a projection of every direction.

| Value | Number | Means |
|---|---|---|
| `HorizontalCross` | 0 | Four cells across and three down: -X, +Z, +X and -Z across the middle row, +Y above +Z and -Y below it. |
| `VerticalCross` | 1 | Three cells across and four down: -X, +Z and +X across the second row, +Y above +Z, and -Y and then -Z below it. |
| `VerticalCrossTurned` | 2 | The vertical cross with -Z turned half a turn, so it joins -Y above it, as HDRShop's light probes and texassemble's v-cross-fnz lay it out. |
| `HorizontalTee` | 3 | Four cells across and three down: +Z, +X, -Z and -X across the middle row, +Y above +Z and -Y below it. |
| `HorizontalStrip` | 4 | Six cells in a row, in face order. |
| `VerticalStrip` | 5 | Six cells in a column, in face order. |
| `Grid2x3` | 6 | Two cells across and three down, in face order row by row, as Godot reads a 2x3 cube map. |
| `Grid3x2` | 7 | Three cells across and two down, in face order row by row, as Godot reads a 3x2 cube map. |
| `AngularMap` | 8 | Debevec's angular map, HDRShop's light probe: a disc whose center faces -Z and whose rim faces +Z, the angle from -Z growing evenly with the distance from the center, +X to the right and +Y up. |
| `Octahedral` | 9 | Filament's octahedral map: +Y at the center, +X to the right and +Z down, the lower half of the sphere folded out to the corners. |

### Wgpu.CullMode

Which faces a render pipeline discards. A triangle whose corners turn counterclockwise on the target faces the viewer.

| Value | Number | Means |
|---|---|---|
| `None` | 0 | Draws both faces. |
| `Front` | 1 | Discards faces wound counterclockwise, which face the viewer. |
| `Back` | 2 | Discards faces wound clockwise, which face away. |

### Wgpu.Decoder

Where a block-compressed texture is decoded when it is saved as an image file.

| Value | Number | Means |
|---|---|---|
| `Cpu` | 0 | On the CPU, by the module's decoders, which give the same pixels on every device. |
| `Gpu` | 1 | On the GPU, by its own decoder, as a shader reads the texels. |

### Wgpu.DeviceType

The kind of adapter.

| Value | Number | Means |
|---|---|---|
| `DiscreteGpu` | 0 | A GPU of its own. |
| `IntegratedGpu` | 1 | A GPU sharing the CPU's package and memory. |
| `VirtualGpu` | 2 | A GPU of a virtual machine. |
| `Cpu` | 3 | Software running on the CPU, such as WARP or lavapipe. |
| `Other` | 4 | Anything else. |

### Wgpu.EdgeOperator

How -Edges finds edges.

| Value | Number | Means |
|---|---|---|
| `Sobel` | 0 | The size of the gradient by Sobel's kernel, which weighs the three rows or columns across it 1, 2 and 1. |
| `Scharr` | 1 | The size of the gradient by Scharr's kernel, which weighs them 3, 10 and 3. |
| `Prewitt` | 2 | The size of the gradient by Prewitt's kernel, which weighs them alike. |
| `Laplacian` | 3 | The size of the second derivative, by the Laplacian of the four nearest texels. |
| `Canny` | 4 | Canny's thin, linked edges in the luminance: 1 on an edge and 0 elsewhere. |

### Wgpu.ElementType

The scalar type of a buffer's elements.

| Value | Number | Means |
|---|---|---|
| `Float32` | 0 | 32-bit float: WGSL `f32`, .NET `float`. |
| `Float64` | 1 | 64-bit float: WGSL `f64` (feature ShaderF64), .NET `double`. |
| `Float16` | 2 | 16-bit float: WGSL `f16` (feature ShaderF16), read and written as .NET `float`. |
| `Int8` | 3 | 8-bit signed integer, .NET `sbyte`. |
| `UInt8` | 4 | 8-bit unsigned integer, .NET `byte`. |
| `Int16` | 5 | 16-bit signed integer, .NET `short`. |
| `UInt16` | 6 | 16-bit unsigned integer, .NET `ushort`. |
| `Int32` | 7 | 32-bit signed integer: WGSL `i32`, .NET `int`. |
| `UInt32` | 8 | 32-bit unsigned integer: WGSL `u32`, .NET `uint`. |
| `Int64` | 9 | 64-bit signed integer: WGSL `i64` (feature ShaderInt64), .NET `long`. |
| `UInt64` | 10 | 64-bit unsigned integer: WGSL `u64` (feature ShaderInt64), .NET `ulong`. |

### Wgpu.FilterEdge

What a filter reads past the edge of the texture.

| Value | Number | Means |
|---|---|---|
| `Clamp` | 0 | The edge texel repeated, as scikit-image's gaussian and ImageMagick's Edge virtual pixels read past it. |
| `Wrap` | 1 | The other side, for a texture that tiles. |
| `Mirror` | 2 | The texture mirrored at its edge, the edge texel repeated, as a MirrorRepeat sampler reads it. |

### Wgpu.FilterMode

How a sampler filters between texels.

| Value | Number | Means |
|---|---|---|
| `Nearest` | 0 | The nearest texel. |
| `Linear` | 1 | A blend of the nearest texels. |

### Wgpu.FlipDirection

Which way a flip mirrors a texture.

| Value | Number | Means |
|---|---|---|
| `Horizontal` | 0 | Left and right swap. |
| `Vertical` | 1 | Top and bottom swap. |

### Wgpu.HeightChannel

The channel of a height map its heights are read from.

| Value | Number | Means |
|---|---|---|
| `Red` | 0 | Red, which a grayscale height map stores its heights in, as it does in every channel. |
| `Green` | 1 | Green. |
| `Blue` | 2 | Blue. |
| `Alpha` | 3 | Alpha. |
| `Luminance` | 4 | The luminance of red, green and blue by texconv's weights: 0.2125, 0.7154 and 0.0721. |

### Wgpu.HeightEdge

What a height map is read as past its edges.

| Value | Number | Means |
|---|---|---|
| `Continue` | 0 | The slope at the edge carried on: a texel past the edge is twice the edge texel's height less the height of the texel as far inside. |
| `Wrap` | 1 | The other side, for a height map that tiles. |
| `Clamp` | 2 | The edge texel repeated. |
| `Mirror` | 3 | The map mirrored at its edge, the edge texel repeated, as a MirrorRepeat sampler reads it. |

### Wgpu.MemoryHint

The memory strategy a device asks its backend for.

| Value | Number | Means |
|---|---|---|
| `Performance` | 0 | Favor speed over memory use, the backend's own default. |
| `MemoryUsage` | 1 | Favor memory use over speed. |

### Wgpu.MipEdge

What the filters of level 0 read past an edge of the image.

| Value | Number | Means |
|---|---|---|
| `Clamp` | 0 | Nothing: a texel past the edge is left out and the rest weigh the new texel, as Resize does. |
| `Wrap` | 1 | The other side of the image, for a texture that tiles. |
| `Mirror` | 2 | The image mirrored at its edge, the edge texel repeated, as a MirrorRepeat sampler reads it. |

### Wgpu.MipFilter

How each mip level is filtered from the image above it.

| Value | Number | Means |
|---|---|---|
| `Box` | 0 | The average of the texels each new texel covers, each weighed by how much of it the new texel covers, from the level above. |
| `Triangle` | 1 | The triangle, from level 0. |
| `Kaiser` | 2 | A sinc under Kaiser's window 3 texels wide with alpha 4, from level 0. |
| `Mitchell` | 3 | Mitchell and Netravali's cubic with B and C a third, from level 0. |
| `Lanczos` | 4 | Lanczos with three lobes, as Resize's, from level 0. |
| `Min` | 5 | The smallest value of each channel among the texels each new texel covers, from the level above. |
| `Max` | 6 | The largest value of each channel among the texels each new texel covers, from the level above. |

### Wgpu.NormalAlpha

What a normal map's alpha holds.

| Value | Number | Means |
|---|---|---|
| `One` | 0 | 1. |
| `Height` | 1 | The height the texel's normal came from, which parallax mapping reads. |
| `Occlusion` | 2 | texconv's occlusion term: darker where the eight neighbors rise above the texel. |

### Wgpu.NormalKernel

How a height map's slopes are taken from the heights around each texel; each kernel is scaled so a steady ramp gives its own slope.

| Value | Number | Means |
|---|---|---|
| `Prewitt` | 0 | texconv's: across, the left-right differences in the texel's row and the rows above and below it, averaged; down, the up-down differences in its column and the columns beside it, averaged. |
| `Central` | 1 | The difference of the two neighbors alone. |
| `Sobel` | 2 | The three differences of Prewitt weighted 1, 2 and 1. |
| `Sobel5x5` | 3 | NVIDIA Texture Tools' 5 by 5 kernel, which reaches two texels each way. |

### Wgpu.NormalMips

How the mip levels of a texture of normals keep them unit length.

| Value | Number | Means |
|---|---|---|
| `Renormalize` | 0 | Each level filtered as -MipFilter says, then each texel's normal scaled back to length 1. |
| `SlopeSpace` | 1 | Each normal's slopes, x / z and y / z, filtered as -MipFilter says and made a unit normal again, as NVIDIA's exporter filters normal maps; a z under 1/255 counts as 1/255, so no slope passes 255. |

### Wgpu.PowerOfTwo

How each side of a size rounds to a power of two.

| Value | Number | Means |
|---|---|---|
| `Nearest` | 0 | The nearer power of two; the larger when both are as near. |
| `Up` | 1 | The power of two at or above the side. |
| `Down` | 2 | The power of two at or below the side. |

### Wgpu.PrimitiveTopology

How vertices make primitives.

| Value | Number | Means |
|---|---|---|
| `PointList` | 0 | Each vertex is a point. |
| `LineList` | 1 | Each pair of vertices is a line. |
| `LineStrip` | 2 | Each vertex after the first ends a line from the one before. |
| `TriangleList` | 3 | Each three vertices are a triangle. |
| `TriangleStrip` | 4 | Each vertex after the second ends a triangle with the two before. |

### Wgpu.QueryKind

What a query set records.

| Value | Number | Means |
|---|---|---|
| `Timestamp` | 0 | GPU timestamps, in ticks of the queue's timestamp period (feature TimestampQuery). |
| `Occlusion` | 1 | Samples that passed the depth and stencil tests during a render pass. |
| `PipelineStatistics` | 2 | Five pipeline statistics per query (feature PipelineStatisticsQuery): vertex shader invocations, clipper invocations, clipper primitives out, fragment shader invocations, compute shader invocations. |

### Wgpu.RenderingIntent

How a conversion from an image's color profile to sRGB maps colors, above all those sRGB lacks.

| Value | Number | Means |
|---|---|---|
| `Perceptual` | 0 | The profile's mapping for photographs, which fits colors sRGB lacks in among the rest. |
| `RelativeColorimetric` | 1 | Colors sRGB holds keep their values, with the profile's white made sRGB's white, and the others go to the nearest sRGB holds. |
| `Saturation` | 2 | The profile's mapping for graphics, which keeps colors vivid. |
| `AbsoluteColorimetric` | 3 | Colors sRGB holds keep their values, the profile's white included. |

### Wgpu.ResizeFilter

How a resize computes each new texel from the texels around where it lands.

| Value | Number | Means |
|---|---|---|
| `Nearest` | 0 | The texel the new texel's center lands in. |
| `Box` | 1 | The average of the texels the new texel covers, each weighed by how much of it the new texel covers, as mip levels are made. |
| `Bilinear` | 2 | A triangle filter: two texels wide when enlarging, wider when shrinking. |
| `Bicubic` | 3 | Catmull-Rom, the cubic that passes through the texels' values: four texels wide when enlarging, wider when shrinking. |
| `Lanczos` | 4 | Lanczos with three lobes: six texels wide when enlarging, wider when shrinking. |

### Wgpu.RoughnessMapping

How a prefiltered specular map's mip levels step through roughness.

| Value | Number | Means |
|---|---|---|
| `Linear` | 0 | Level n of N holds perceptual roughness n / (N - 1), down to 2x2, as Khronos's glTF IBL sampler writes the levels and glTF viewers read them. |
| `Quadratic` | 1 | Level n of N holds perceptual roughness 1 - sqrt(1 - n / (N - 1)), down to 16x16, as Filament's cmgen writes the levels. |

### Wgpu.ShaderLanguage

The language a shader is written in.

| Value | Number | Means |
|---|---|---|
| `Wgsl` | 0 | WebGPU Shading Language. |
| `Glsl` | 1 | OpenGL Shading Language, one stage per shader. |
| `SpirV` | 2 | SPIR-V words. |

### Wgpu.ShaderStage

A pipeline stage.

| Value | Number | Means |
|---|---|---|
| `Compute` | 0 | A compute shader. |
| `Vertex` | 1 | A vertex shader. |
| `Fragment` | 2 | A fragment shader. |
| `Task` | 3 | A task shader, which launches mesh shaders. |
| `Mesh` | 4 | A mesh shader. |
| `RayTracing` | 5 | A ray-tracing stage. |

### Wgpu.ShaderTarget

A language ConvertTo-WgpuShader writes.

| Value | Number | Means |
|---|---|---|
| `Wgsl` | 0 | WGSL text. |
| `SpirV` | 1 | SPIR-V words, as `uint[]`. |
| `Glsl` | 2 | GLSL text for one entry point (OpenGL 4.50, or OpenGL ES 3.10 and up). |
| `Hlsl` | 3 | HLSL text for Direct3D, Shader Model 5.0 to 6.9. |
| `Msl` | 4 | Metal Shading Language text. |
| `Dot` | 5 | The module's intermediate form as a Graphviz graph. |

### Wgpu.StencilOperation

What a stencil test does to the stencil value under a fragment.

| Value | Number | Means |
|---|---|---|
| `Keep` | 0 | Leaves the value as it is. |
| `Zero` | 1 | Sets the value to 0. |
| `Replace` | 2 | Sets the value to the stencil reference. |
| `Invert` | 3 | Inverts the value's bits. |
| `IncrementClamp` | 4 | Adds 1, staying at the largest value. |
| `DecrementClamp` | 5 | Subtracts 1, staying at 0. |
| `IncrementWrap` | 6 | Adds 1, wrapping from the largest value to 0. |
| `DecrementWrap` | 7 | Subtracts 1, wrapping from 0 to the largest value. |

### Wgpu.TextureAspect

Which part of a depth-stencil texture, or which plane of a planar one, an operation touches.

| Value | Number | Means |
|---|---|---|
| `All` | 0 | Every part: the color of a color texture. |
| `DepthOnly` | 1 | The depth of a depth-stencil texture. |
| `StencilOnly` | 2 | The stencil of a depth-stencil texture. |
| `Plane0` | 3 | The first plane of an NV12 or P010 texture: luma, one channel at the texture's size. |
| `Plane1` | 4 | The second plane of an NV12 or P010 texture: chroma, two channels at half the texture's width and height. |

### Wgpu.TextureChannel

A channel of a texture's texels.

| Value | Number | Means |
|---|---|---|
| `Red` | 0 | The first channel: red, the depth of a texture with depth, or the stencil of Stencil8. |
| `Green` | 1 | The second channel: green, or the stencil of a texture with depth and stencil. |
| `Blue` | 2 | The third channel: blue. |
| `Alpha` | 3 | The fourth channel: alpha. |

### Wgpu.TextureDimension

A texture's shape.

| Value | Number | Means |
|---|---|---|
| `D1` | 0 | One dimension: a row of texels. |
| `D2` | 1 | Two dimensions, with array layers. |
| `D3` | 2 | Three dimensions. |

### Wgpu.TextureUsage

What a texture may be used for.

| Value | Number | Means |
|---|---|---|
| `CopySrc` | 0 | The source of a copy. |
| `CopyDst` | 1 | The destination of a copy or a write. |
| `TextureBinding` | 2 | Sampled or loaded in a shader. |
| `StorageBinding` | 3 | A storage texture in a shader. |
| `RenderAttachment` | 4 | A color or depth attachment of a render pass. |
| `StorageAtomic` | 5 | Atomic operations on a storage texture. |
| `TransientAttachment` | 6 | An attachment whose contents never leave the tile memory of the pass. |

### Wgpu.ToneMapCurve

A curve that maps HDR colors into the range from 0 to 1.

| Value | Number | Means |
|---|---|---|
| `Reinhard` | 0 | Reinhard's, with the brightest texel's luminance as the white point, as texconv applies it. |
| `AcesFilmic` | 1 | ACES Filmic, as three.js fits it. |
| `AgX` | 2 | AgX, as three.js computes it. |
| `Neutral` | 3 | Khronos PBR Neutral, which keeps hues and changes colors below its compression point little. |
