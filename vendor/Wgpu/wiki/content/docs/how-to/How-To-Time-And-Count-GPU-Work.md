---
title: How To Time And Count GPU Work
weight: 16
---

Find out how long a shader takes on the GPU, apart from the time spent compiling it and copying data there and back, and count what a pass did: how many times a compute shader ran, and how many pixels a draw covered. Times on a GPU change from run to run, so the examples here compare times rather than print them; the counts are exact. Timestamps need the TimestampQuery feature and statistics the PipelineStatisticsQuery feature, which a device has whenever its adapter does.

This shader spins in a loop, so a dispatch takes long enough to time, and twice the rounds take about twice as long:

```powershell
PS> Get-Content ./spin.wgsl
override rounds: u32 = 1u;
@group(0) @binding(0) var<storage, read_write> data: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    var x = data[id.x];
    for (var i = 0u; i < rounds; i++) {
        x = fma(x, 0.9999999, 0.0000001);
    }
    data[id.x] = x;
}
PS> $spin = Get-Content ./spin.wgsl -Raw
```

## Time one call

`Invoke-WgpuCompute -PassThru` returns the arrays the shader wrote with the workgroups it ran, the nanoseconds the GPU spent in the pass, and the milliseconds the whole call took:

```powershell
PS> $result = Invoke-WgpuCompute $spin @{ data = [float[]]::new(64) } -Constant @{ rounds = 1000000 } -PassThru
PS> $result.Workgroups -join ','
1,1,1

PS> $result.GpuTimeNs -gt 0
True

PS> $result.CpuTimeMs * 1e6 -gt $result.GpuTimeNs
True
```

The call took longer than the GPU's pass, as it also compiled the shader, copied the array to the GPU and back, and waited for the GPU.

## Time passes in a recording

A timestamp query set holds a GPU clock reading in each query. A compute pass given one writes the time it begins and the time it ends into two of them, so two passes take four:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $stamps = New-WgpuQuerySet Timestamp 4 -Label passes
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $stamps = [Wgpu.Device]::GetDefault().CreateQuerySet('Timestamp', 4, 'passes')
```
{{< /tab >}}
{{< /tabs >}}

A pipeline of a million rounds and one of two million, each with a bind group of its own, since a bind group is made for the layout of one pipeline:

```powershell
PS> $data = New-WgpuBuffer -Length 64 -ElementType Float32
PS> $million = New-WgpuComputePipeline $spin -Constant @{ rounds = 1000000 }
PS> $twoMillion = New-WgpuComputePipeline $spin -Constant @{ rounds = 2000000 }
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginComputePass('a million', $stamps, 0, 1)
PS> $pass.SetPipeline($million)
PS> $pass.SetBindGroup(0, $million.BindGroup(0, $data))
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> $pass = $encoder.BeginComputePass('two million', $stamps, 2, 3)
PS> $pass.SetPipeline($twoMillion)
PS> $pass.SetBindGroup(0, $twoMillion.BindGroup(0, $data))
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> Submit-WgpuCommand $encoder.Finish()
```

`ReadNanoseconds` reads the four clock readings in nanoseconds, each the raw ticks times the queue's timestamp period. Each pass's time is its end minus its beginning:

```powershell
PS> $ns = $stamps.ReadNanoseconds()
PS> $first = $ns[1] - $ns[0]
PS> $second = $ns[3] - $ns[2]
PS> $ratio = $second / $first
PS> $ratio -gt 1.5 -and $ratio -lt 2.5
True
```

## Time what runs between passes

`WriteTimestamp` on an encoder writes a reading between passes and copies, where the TimestampQueryInsideEncoders feature allows it, and `ResolveQuerySet` copies readings into a buffer as raw ticks, for a shader to read or to keep. `TimestampPeriod` gives the nanoseconds of one tick:

```powershell
PS> $around = New-WgpuQuerySet Timestamp 2
PS> $ticks = New-WgpuBuffer -Length 2 -ElementType UInt64
PS> $encoder = New-WgpuCommandEncoder
PS> $encoder.WriteTimestamp($around, 0)
PS> $pass = $encoder.BeginComputePass()
PS> $pass.SetPipeline($million)
PS> $pass.SetBindGroup(0, $million.BindGroup(0, $data))
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> $encoder.WriteTimestamp($around, 1)
PS> $encoder.ResolveQuerySet($around, 0, 2, $ticks)
PS> Submit-WgpuCommand $encoder.Finish()
PS> $read = Read-WgpuBuffer $ticks
PS> $elapsed = ($read[1] - $read[0]) * (Get-WgpuDevice).TimestampPeriod()
PS> $elapsed -gt $first / 2
True
```

The two readings span the same million rounds as the first pass, so they take about as long.

## Count the shader's runs

A pipeline statistics query counts, for the part of a pass between its begin and its end, the vertex shader's runs, the triangles into and out of the clipper, the fragment shader's runs and the compute shader's runs. Four workgroups of 64 run the compute shader 256 times:

```powershell
PS> $statistics = New-WgpuQuerySet PipelineStatistics 1
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginComputePass()
PS> $pass.SetPipeline($million)
PS> $pass.SetBindGroup(0, $million.BindGroup(0, (New-WgpuBuffer -Length 256 -ElementType Float32)))
PS> $pass.BeginPipelineStatisticsQuery($statistics, 0)
PS> $pass.Dispatch(4)
PS> $pass.EndPipelineStatisticsQuery()
PS> $pass.End()
PS> Submit-WgpuCommand $encoder.Finish()
PS> $statistics.Read() -join ','
0,0,0,0,256
```

## Count the pixels a draw covers

An occlusion query counts the samples of a draw that pass the depth test, or with no depth target every sample it draws. A render pass given an occlusion query set counts what is drawn between `BeginOcclusionQuery` and `EndOcclusionQuery`. This draws the triangle below the diagonal of an 8 by 8 target, from its bottom left corner:

```powershell
PS> Get-Content ./half.wgsl
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 1.0, 1.0, 1.0); }
PS> $half = New-WgpuRenderPipeline (Get-Content ./half.wgsl -Raw)
PS> $covered = New-WgpuQuerySet Occlusion 1
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginRenderPass((New-WgpuTexture 8 8), $null, $null, $null, $covered)
PS> $pass.SetPipeline($half)
PS> $pass.BeginOcclusionQuery(0)
PS> $pass.Draw(3)
PS> $pass.EndOcclusionQuery()
PS> $pass.End()
PS> Submit-WgpuCommand $encoder.Finish()
PS> $covered.Read()[0]
28
```

Of the 64 pixels, those whose centers are inside the triangle were drawn: the eight on the diagonal are on its edge, and the rules that share an edge between two triangles give them to the triangle on the other side.

[Time and count with queries](Recording.md#time-and-count-with-queries) has every query method, and `-PassThru` is on [Running compute shaders](Compute-Cmdlets.md#invoke-wgpucompute).
