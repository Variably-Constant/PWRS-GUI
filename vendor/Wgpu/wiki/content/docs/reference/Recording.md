---
title: Recording commands
weight: 70
---

`Invoke-WgpuCompute` and `Invoke-WgpuRender` each do every step of one dispatch or one draw: build the pipeline, bind the resources, run, submit, wait, and copy the results back. Recording by hand does each step once and keeps the data on the GPU in between. A command encoder records copies, clears and passes in order, and nothing runs until the command buffer it finishes into is submitted; a compute pass records dispatches, and a render pass draws. Many dispatches, draws and copies go to the GPU in one submission, and a pass can be timed or counted with queries. The parameters are on [Recording commands](Command-Cmdlets.md) under Cmdlets and types, and the `Wgpu.CommandEncoder`, `Wgpu.ComputePass`, `Wgpu.RenderPass` and `Wgpu.QuerySet` classes on [Types](Types.md#wgpucommandencoder).

## Record, finish, submit

An encoder records commands; `Finish` turns the recording into a command buffer, and submitting it runs it and waits for the GPU. Until then nothing has happened:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $source = New-WgpuBuffer ([int[]](1..4))
PS> $target = New-WgpuBuffer -Length 4 -ElementType Int32
PS> $encoder = New-WgpuCommandEncoder -Label work
PS> $encoder.CopyBufferToBuffer($source, 0, $target, 0)
PS> $encoder.ClearBuffer($source)
PS> (Read-WgpuBuffer $target) -join ','
0,0,0,0

PS> $commands = $encoder.Finish()
PS> $commands
Label IsSubmitted
----- -----------
work  False

PS> Submit-WgpuCommand $commands
PS> (Read-WgpuBuffer $target) -join ','
1,2,3,4

PS> (Read-WgpuBuffer $source) -join ','
0,0,0,0
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $source = $gpu.Upload([int[]](1..4))
PS> $target = $gpu.Zeros(4, 'Int32')
PS> $encoder = $gpu.CreateCommandEncoder('work')
PS> $encoder.CopyBufferToBuffer($source, 0, $target, 0)
PS> $encoder.ClearBuffer($source)
PS> $target.Read() -join ','
0,0,0,0

PS> $commands = $encoder.Finish()
PS> $commands
Label IsSubmitted
----- -----------
work  False

PS> $gpu.Submit($commands)
PS> $target.Read() -join ','
1,2,3,4

PS> $source.Read() -join ','
0,0,0,0
```
{{< /tab >}}
{{< /tabs >}}

A finished encoder records nothing more, and a command buffer runs once:

```powershell
PS> $encoder.IsFinished
True

PS> $encoder.ClearBuffer($source)
MethodInvocationException: Exception calling "ClearBuffer" with "1" argument(s): "[WgpuEncoderFinished] the encoder was finished; open another"

PS> $commands.IsSubmitted
True

PS> Submit-WgpuCommand $commands
Submit-WgpuCommand: the command buffer of encoder 'work' was already submitted; record the work again
```

Several command buffers run in the order given, and ones piped in are submitted together once the pipeline ends:

```powershell
PS> $buffer = New-WgpuBuffer ([int[]](1..4))
PS> $first = New-WgpuCommandEncoder
PS> $first.ClearBuffer($buffer)
PS> $second = New-WgpuCommandEncoder
PS> $second.CopyBufferToBuffer((New-WgpuBuffer ([int[]](7, 7))), 0, $buffer, 8)
PS> Submit-WgpuCommand $first.Finish(), $second.Finish()
PS> (Read-WgpuBuffer $buffer) -join ','
0,0,7,7

PS> 1..2 | ForEach-Object { $e = New-WgpuCommandEncoder; $e.ClearBuffer($buffer); $e.Finish() } | Submit-WgpuCommand
PS> (Read-WgpuBuffer $buffer) -join ','
0,0,0,0
```

## Dispatch many times from one binding

This shader moves each value halfway toward 2:

```powershell
PS> Get-Content ./step.wgsl
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] / 2.0 + 1.0; }
}
PS> $step = New-WgpuComputePipeline (Get-Content ./step.wgsl -Raw)
PS> $values = New-WgpuBuffer ([float[]](0, 10, 100, 1000))
PS> $group = New-WgpuBindGroup $step $values
```

A compute pass takes the pipeline and the bind group once, then twenty dispatches, each of which reads what the one before wrote:

```powershell
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginComputePass()
PS> $pass.SetPipeline($step)
PS> $pass.SetBindGroup(0, $group)
PS> 1..20 | ForEach-Object { $pass.Dispatch(1) }
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> Read-WgpuBuffer $values
1.999998
2.000008
2.000093
2.000952
```

## Mix passes and copies

An encoder takes passes and copies one after another, and the GPU runs them in that order. One step, a copy of what it left, and two more steps go in one command buffer:

```powershell
PS> $values = New-WgpuBuffer ([float[]](0, 10, 100, 1000))
PS> $group = New-WgpuBindGroup $step $values
PS> $snapshot = New-WgpuBuffer -Length 4 -ElementType Float32
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginComputePass()
PS> $pass.SetPipeline($step)
PS> $pass.SetBindGroup(0, $group)
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> $encoder.CopyBufferToBuffer($values, 0, $snapshot, 0)
PS> $pass = $encoder.BeginComputePass()
PS> $pass.SetPipeline($step)
PS> $pass.SetBindGroup(0, $group)
PS> $pass.Dispatch(1)
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> (Read-WgpuBuffer $snapshot) -join ','
1,6,51,501

PS> (Read-WgpuBuffer $values) -join ','
1.75,3,14.25,126.75
```

## Pass data straight to a dispatch

A shader can read a few bytes of immediate data, which a pass sets without a buffer, and a dispatch can read its workgroup counts from a buffer on the GPU, three `u32` that another shader can have written:

```powershell
PS> Get-Content ./scaled.wgsl
var<immediate> k: f32;
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * k; }
}
PS> $scaled = New-WgpuComputePipeline (Get-Content ./scaled.wgsl -Raw)
PS> $scaled.ImmediateSize
4

PS> $three = New-WgpuBuffer ([float[]](1, 2, 3))
PS> $counts = New-WgpuBuffer ([uint32[]](1, 1, 1))
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginComputePass()
PS> $pass.SetPipeline($scaled)
PS> $pass.SetBindGroup(0, $scaled.BindGroup(0, $three))
PS> $pass.SetImmediates(0, [float[]](3))
PS> $pass.DispatchIndirect($counts, 0)
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> Read-WgpuBuffer $three
3
6
9
```

`Invoke-WgpuCompute` takes immediate data as `-Immediate`, and indirect counts as `-Indirect`.

## Draw in a render pass

A render pass draws into a texture, cleared first to the color it is given, with vertex buffers fed one for each input of the vertex shader in location order, and an index buffer of UInt16 or UInt32. A scissor rectangle keeps the draw to part of the target. The shader:

```powershell
PS> Get-Content ./colored.wgsl
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@location(0) position: vec2<f32>, @location(1) color: vec3<f32>) -> Out {
    var out: Out;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
```

A quad that covers the target, kept by the scissor to its left half: the top-left pixel is drawn red, and the top-right one stays the black it was cleared to:

```powershell
PS> $pipeline = New-WgpuRenderPipeline (Get-Content ./colored.wgsl -Raw)
PS> $positions = New-WgpuBuffer ([float[]](-1, -1, 1, -1, -1, 1, 1, 1))
PS> $colors = New-WgpuBuffer ([float[]](1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0))
PS> $indices = New-WgpuBuffer ([uint32[]](0, 1, 2, 2, 1, 3))
PS> $canvas = New-WgpuTexture 4 4
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginRenderPass($canvas, @(0, 0, 0, 1))
PS> $pass.SetPipeline($pipeline)
PS> $pass.SetVertexBuffer(0, $positions)
PS> $pass.SetVertexBuffer(1, $colors)
PS> $pass.SetIndexBuffer($indices)
PS> $pass.SetScissorRect(0, 0, 2, 4)
PS> $pass.DrawIndexed(6)
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> $pixels = Read-WgpuTexture $canvas
PS> $pixels[0..3] -join ','
255,0,0,255

PS> $pixels[12..15] -join ','
0,0,0,255
```

A draw can read its arguments from a buffer on the GPU: `DrawIndexedIndirect` the index count, the instance count, the first index, the base vertex and the first instance, and `DrawIndirect` the vertex count, the instance count, the first vertex and the first instance. A viewport maps the draw onto part of the target, here its left half. `SetBlendConstant` sets the color a blend factor of Constant reads, which a pipeline built with `-Blend Constant`, or with a Custom blend naming that factor, mixes by; `SetStencilReference` sets the value a stencil test compares with. This pipeline's blend reads no constant and it tests no stencil, so neither changes this draw:

```powershell
PS> $canvas = New-WgpuTexture 4 4
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginRenderPass($canvas, @(0, 0, 0, 1))
PS> $pass.SetPipeline($pipeline)
PS> $pass.SetVertexBuffer(0, $positions)
PS> $pass.SetVertexBuffer(1, $colors)
PS> $pass.SetIndexBuffer($indices)
PS> $pass.SetViewport(0, 0, 2, 4)
PS> $pass.SetBlendConstant(1, 1, 1, 1)
PS> $pass.SetStencilReference(1)
PS> $pass.DrawIndexedIndirect((New-WgpuBuffer ([uint32[]](6, 1, 0, 0, 0))), 0)
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> $pixels = Read-WgpuTexture $canvas
PS> $pixels[0..3] -join ','
255,0,0,255

PS> $pixels[12..15] -join ','
0,0,0,255
```

Three vertices of the quad with no index buffer are its lower-left half:

```powershell
PS> $canvas = New-WgpuTexture 4 4
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginRenderPass($canvas, @(0, 0, 0, 1))
PS> $pass.SetPipeline($pipeline)
PS> $pass.SetVertexBuffer(0, $positions)
PS> $pass.SetVertexBuffer(1, $colors)
PS> $pass.DrawIndirect((New-WgpuBuffer ([uint32[]](3, 1, 0, 0))), 0)
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> $pixels = Read-WgpuTexture $canvas
PS> $pixels[48..51] -join ','
255,0,0,255

PS> $pixels[12..15] -join ','
0,0,0,255
```

A hashtable in place of `BeginRenderPass`'s arguments names them by key: `Color`, `ClearColor`, `Depth`, `ClearDepth`, `ClearStencil`, `ResolveTarget`, `OcclusionQuerySet` and `Label`. A pass clears its color to opaque black, its depth to 1 and its stencil to 0 unless the clear key gives another value, and keeps what the texture holds when it gives `'Load'`. With a stencil target, one draw marks where another may draw. The first writes the reference, 1, where the scissor lets it draw, the left half; the second passes only where the stencil equals the reference, so its green covers the left half alone:

```powershell
PS> $stencil = New-WgpuTexture 4 4 -Format Stencil8
PS> $marker = New-WgpuRenderPipeline (Get-Content ./colored.wgsl -Raw) -DepthFormat Stencil8 -StencilPass Replace
PS> $masked = New-WgpuRenderPipeline (Get-Content ./colored.wgsl -Raw) -DepthFormat Stencil8 -StencilCompare Equal
PS> $greens = New-WgpuBuffer ([float[]](0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0))
PS> $marked = New-WgpuTexture 4 4
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginRenderPass(@{ Color = $marked; Depth = $stencil })
PS> $pass.SetStencilReference(1)
PS> $pass.SetPipeline($marker)
PS> $pass.SetVertexBuffer(0, $positions)
PS> $pass.SetVertexBuffer(1, $colors)
PS> $pass.SetIndexBuffer($indices)
PS> $pass.SetScissorRect(0, 0, 2, 4)
PS> $pass.DrawIndexed(6)
PS> $pass.SetScissorRect(0, 0, 4, 4)
PS> $pass.SetPipeline($masked)
PS> $pass.SetVertexBuffer(1, $greens)
PS> $pass.DrawIndexed(6)
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> $pixels = Read-WgpuTexture $marked
PS> $pixels[0..3] -join ','
0,255,0,255

PS> $pixels[12..15] -join ','
0,0,0,255

PS> (Read-WgpuTexture $stencil -Aspect StencilOnly)[0..3] -join ','
1,1,0,0
```

A second pass that loads the color and the stencil draws on what the first left. With the reference at 0, the masked pipeline passes where the stencil is still 0, the right half, and leaves the green alone:

```powershell
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginRenderPass(@{ Color = $marked; ClearColor = 'Load'; Depth = $stencil; ClearStencil = 'Load' })
PS> $pass.SetPipeline($masked)
PS> $pass.SetVertexBuffer(0, $positions)
PS> $pass.SetVertexBuffer(1, $colors)
PS> $pass.SetIndexBuffer($indices)
PS> $pass.DrawIndexed(6)
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> $pixels = Read-WgpuTexture $marked
PS> $pixels[0..3] -join ','
0,255,0,255

PS> $pixels[12..15] -join ','
255,0,0,255
```

An encoder also copies a texture into another of the same size and format:

```powershell
PS> $copy = New-WgpuTexture 4 4
PS> $encoder = New-WgpuCommandEncoder
PS> $encoder.CopyTextureToTexture($canvas, $copy)
PS> $encoder.Finish() | Submit-WgpuCommand
PS> (Read-WgpuTexture $copy)[48..51] -join ','
255,0,0,255
```

## Time and count with queries

A query set holds queries of one kind: timestamps, occlusion counts, or pipeline statistics. Timestamp queries need the TimestampQuery feature and pipeline statistics the PipelineStatisticsQuery feature, which a device has whenever its adapter does:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $stamps = New-WgpuQuerySet Timestamp 2
PS> $stamps
Label         Kind      Count
-----         ----      -----
2 x Timestamp Timestamp     2
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $stamps = $gpu.CreateQuerySet('Timestamp', 2, $null)
PS> $stamps
Label         Kind      Count
-----         ----      -----
2 x Timestamp Timestamp     2
```
{{< /tab >}}
{{< /tabs >}}

A compute pass given a timestamp query set writes the GPU's clock into two of its queries, as the pass begins and as it ends, and `ReadNanoseconds` reads them back, each the raw ticks times the queue's timestamp period, as `ReadNanosecondsAsync` does in a task. The times differ from run to run, so this compares them:

```powershell
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginComputePass('timed', $stamps, 0, 1)
PS> $pass.SetPipeline($step)
PS> $pass.SetBindGroup(0, $group)
PS> 1..100 | ForEach-Object { $pass.Dispatch(1) }
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> $ns = $stamps.ReadNanoseconds()
PS> $ns[1] -gt $ns[0]
True

PS> ($stamps.ReadNanosecondsAsync().GetAwaiter().GetResult() -join ',') -eq ($ns -join ',')
True

PS> (New-WgpuQuerySet Occlusion 4).ReadNanoseconds()
MethodInvocationException: Exception calling "ReadNanoseconds" with "0" argument(s): "[WgpuNotTimestamps] only a timestamp query set reads as nanoseconds"
```

`WriteTimestamp` writes one between commands, `ResolveQuerySet` copies results into a buffer, and a pipeline-statistics query counts, for one pass, the invocations of the vertex shader, the triangles into and out of the clipper, and the invocations of the fragment and compute shaders:

```powershell
PS> $ticks = New-WgpuBuffer -Length 2 -ElementType UInt64
PS> $statistics = New-WgpuQuerySet PipelineStatistics 1
PS> $encoder = New-WgpuCommandEncoder
PS> $encoder.WriteTimestamp($stamps, 0)
PS> $pass = $encoder.BeginComputePass()
PS> $pass.SetPipeline($scaled)
PS> $pass.SetBindGroup(0, $scaled.BindGroup(0, $three))
PS> $pass.SetImmediates(0, [float[]](1))
PS> $pass.BeginPipelineStatisticsQuery($statistics, 0)
PS> $pass.Dispatch(1)
PS> $pass.EndPipelineStatisticsQuery()
PS> $pass.End()
PS> $encoder.WriteTimestamp($stamps, 1)
PS> $encoder.ResolveQuerySet($stamps, 0, 2, $ticks)
PS> $encoder.Finish() | Submit-WgpuCommand
PS> $resolved = Read-WgpuBuffer $ticks
PS> $resolved[1] -ge $resolved[0]
True

PS> $statistics.Read() -join ','
0,0,0,0,64
```

An occlusion query counts the samples of a draw that pass, in a render pass given the query set:

```powershell
PS> $occlusion = New-WgpuQuerySet Occlusion 1
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginRenderPass($canvas, $null, $null, $null, $occlusion)
PS> $pass.SetPipeline($pipeline)
PS> $pass.SetVertexBuffer(0, $positions)
PS> $pass.SetVertexBuffer(1, $colors)
PS> $pass.SetIndexBuffer($indices)
PS> $pass.BeginOcclusionQuery(0)
PS> $pass.DrawIndexed(6)
PS> $pass.EndOcclusionQuery()
PS> $pass.End()
PS> $encoder.Finish() | Submit-WgpuCommand
PS> $occlusion.Read()[0] -gt 0
True
```

## Mark the recording for a debugger

Debug groups and markers name parts of a recording for a graphics debugger such as RenderDoc, which shows them in a capture:

```powershell
PS> $marked = New-WgpuCommandEncoder
PS> $marked.PushDebugGroup('reset')
PS> $marked.ClearBuffer($buffer)
PS> $marked.InsertDebugMarker('cleared')
PS> $marked.PopDebugGroup()
PS> $marked.Finish() | Submit-WgpuCommand
```

## Find where a mistake is reported

A pass refuses commands once it has ended, and an encoder once it has finished. Anything else wgpu checks when the encoder finishes, so a mistake made in a pass is reported by `Finish`. A dispatch with no pipeline set:

```powershell
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginComputePass()
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> $pass.Dispatch(1)
MethodInvocationException: Exception calling "Dispatch" with "1" argument(s): "[WgpuPassEnded] the pass has ended"

PS> $encoder.Finish()
MethodInvocationException: Exception calling "Finish" with "0" argument(s): "[WgpuFinish] the recorded commands are invalid: Validation Error

Caused by:
  In a CommandEncoder, label = 'Wgpu encoder'
    In a dispatch command, indirect:false
      Compute pipeline must be set
"

PS> $encoder.Finish()
MethodInvocationException: Exception calling "Finish" with "0" argument(s): "[WgpuEncoderFinished] the encoder was finished already"
```
