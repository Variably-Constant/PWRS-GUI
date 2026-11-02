---
title: Commands And Tasks
weight: 7
---

Each cmdlet so far did every step of its work and waited for the GPU to finish it. This chapter records steps by hand into a command encoder, so many of them go to the GPU in one submission, and starts GPU work with a method that returns at once, so PowerShell goes on while the GPU works.

## Record, then submit

A command encoder records commands in order, and nothing runs until the command buffer it finishes into is submitted. This one copies one buffer into another and then clears the first:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $source = New-WgpuBuffer ([int[]](1..4))
PS> $target = New-WgpuBuffer -Length 4 -ElementType Int32
PS> $encoder = New-WgpuCommandEncoder
PS> $encoder.CopyBufferToBuffer($source, 0, $target, 0)
PS> $encoder.ClearBuffer($source)
PS> (Read-WgpuBuffer $target) -join ','
0,0,0,0

PS> Submit-WgpuCommand $encoder.Finish()
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
PS> $encoder = $gpu.CreateCommandEncoder($null)
PS> $encoder.CopyBufferToBuffer($source, 0, $target, 0)
PS> $encoder.ClearBuffer($source)
PS> $target.Read() -join ','
0,0,0,0

PS> $gpu.Submit($encoder.Finish())
PS> $target.Read() -join ','
1,2,3,4

PS> $source.Read() -join ','
0,0,0,0
```
{{< /tab >}}
{{< /tabs >}}

Before the submission the target still held zeros. Once submitted, the GPU ran the copy and then the clear, in the order they were recorded.

## Many dispatches in one pass

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

A bind group fixes the resources of a pipeline's bindings once. A compute pass takes the pipeline and the bind group, then twenty dispatches, each reading what the one before wrote, and all twenty go to the GPU in one submission:

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

Every value is within a thousandth of 2: each step halves its distance from 2, and twenty steps halve it twenty times.

## Work while the GPU works

A method whose name ends in `Async` starts its work and returns a .NET task at once, instead of waiting. This shader spins in a loop so that one dispatch takes a noticeable time on the GPU:

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
PS> $spin = New-WgpuComputePipeline (Get-Content ./spin.wgsl -Raw) -Constant @{ rounds = 25000000 }
PS> $data = New-WgpuBuffer -Length 64 -ElementType Float32
```

`DispatchAsync` returns while the GPU is still working, PowerShell runs other commands meanwhile, and the task's `GetAwaiter().GetResult()` waits for it to end:

```powershell
PS> $running = $spin.DispatchAsync($data, 1)
PS> $running.Status
WaitingForActivation

PS> (1..100000 | Measure-Object -Sum).Sum
5000050000

PS> $running.GetAwaiter().GetResult()
PS> $running.Status
RanToCompletion
```

[Tasks and threads](Tasks.md) lists every method that returns a task, and shows how to wait for several at once, read the error of one that failed, and cancel a wait.

## Where next

The [how-to guides](/Wgpu/docs/how-to/) each take one task further, and the [reference](/Wgpu/docs/reference/) holds every cmdlet, class and error.
