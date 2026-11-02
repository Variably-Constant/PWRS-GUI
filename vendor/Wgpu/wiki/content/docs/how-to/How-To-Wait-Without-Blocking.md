---
title: How To Wait Without Blocking
weight: 17
---

Keep a script working while the GPU computes: start a long job as a task, do other work in the meantime, show progress while it runs and let Ctrl+C stop the wait, wait for several jobs at once, give up on a wait that takes too long, and find out why a job failed. Each method that waits for the GPU has a form ending in `Async` that returns a .NET task at once and waits on a thread of its own; [Tasks and threads](Tasks.md) has each part on its own.

## A job that takes a while

A shader that spins plays the part of real work here; one dispatch of it takes a few hundredths of a second on the GPU this page ran on:

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

## Start it and keep working

`DispatchAsync` returns at once with a task. The task is still waiting when the next command runs, and PowerShell is free in the meantime. `GetAwaiter().GetResult()` waits for the task to end and returns what it holds, nothing for a dispatch:

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

A read's task holds what it read:

```powershell
PS> $numbers = New-WgpuBuffer ([float[]](1..8))
PS> $numbers.ReadAsync().GetAwaiter().GetResult() -join ','
1,2,3,4,5,6,7,8
```

## Show progress, and let Ctrl+C in

Ctrl+C stops a cmdlet that waits for the GPU within a tenth of a second, but PowerShell cannot stop a method while it runs: `Wait()` and `GetAwaiter().GetResult()` return only when the GPU is done. A loop of short waits gives Ctrl+C its chance between them, and the time to report progress:

```powershell
PS> $running = $spin.DispatchAsync($data, 1)
PS> $started = [Diagnostics.Stopwatch]::StartNew()
PS> while (-not $running.Wait(100)) {
>>     Write-Progress -Activity 'Spinning on the GPU' -Status ('{0:n1} s' -f $started.Elapsed.TotalSeconds)
>> }
PS> Write-Progress -Activity 'Spinning on the GPU' -Completed
PS> $running.Status
RanToCompletion
```

Stopped by Ctrl+C, the loop leaves the GPU to finish the work on its own.

## Wait for several

`[Threading.Tasks.Task]::WaitAll` waits until every task given has ended, `WaitAny` returns the index of the first to end, and `WhenAll` and `WhenAny` return a task that ends with them. The `[Threading.Tasks.Task[]]` cast makes PowerShell's array of tasks the array those methods take:

```powershell
PS> $dispatches = 1..3 | ForEach-Object { $spin.DispatchAsync($data, 1) }
PS> [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]$dispatches)
PS> $dispatches.Status
RanToCompletion
RanToCompletion
RanToCompletion
```

A script that started work and kept no task for it waits for all of it with `Wait-WgpuDevice`, which returns once the GPU has done everything the device was given, and stops at Ctrl+C as any cmdlet does:

```powershell
PS> 1..3 | ForEach-Object { $null = $spin.DispatchAsync($data, 1) }
PS> Wait-WgpuDevice
```

## Give up on a wait

A token from a CancellationTokenSource ends the wait when the source is canceled, here after 50 milliseconds, within a tenth of a second, and the task ends as Canceled. It ends the wait only: the GPU still does the work it was given, and whatever waits for the device afterward waits for that work too. Ten dispatches queue about a third of a second of work, and a wait for all of it gives up first:

```powershell
PS> $dispatches = 1..10 | ForEach-Object { $spin.DispatchAsync($data, 1) }
PS> $deadline = [Threading.CancellationTokenSource]::new(50)
PS> $waiting = (Get-WgpuDevice).WaitAsync($deadline.Token)
PS> [Threading.Tasks.Task]::WhenAny($waiting).Wait()
PS> $waiting.Status
Canceled

PS> [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]$dispatches)
PS> $dispatches.Status | Sort-Object -Unique
RanToCompletion
```

Each `Async` method takes the token last, and it can be left out; `$null` does not convert to a token:

```powershell
PS> $numbers.ReadAsync($null, $null)
MethodException: Cannot convert argument "cancellationToken", with value: "", for "ReadAsync" to type "System.Threading.CancellationToken": "Cannot convert null to type "System.Threading.CancellationToken"."
```

## Find out why a job failed

What a method can check before the work starts, it refuses at the call, as its plain form would; a shader with no compute entry point never becomes a task:

```powershell
PS> (Get-WgpuDevice).RunAsync('fn main() {}', $null, 1, 1, 1)
MethodInvocationException: Exception calling "RunAsync" with "5" argument(s): "[WgpuNoEntryPoint] the shader has no Compute entry point"
```

What fails once the work has started ends the task as Faulted, as does a panic in wgpu or in the module while the work runs, as `WgpuPanic` with the panic's message. `GetAwaiter().GetResult()` throws the module's own error then, and the task's `Exception` holds it; `Wait()` throws it wrapped in an AggregateException, and `Result` gives `$null` with no error at all. Waiting for `WhenAny` of a task waits without throwing:

```powershell
PS> $both = New-WgpuTexture 2 2 -Format Depth24PlusStencil8
PS> $read = $both.ReadAsync(0, 0, 'DepthOnly')
PS> [Threading.Tasks.Task]::WhenAny($read).Wait()
PS> $read.Status
Faulted

PS> $read.Exception.InnerException.Message
[WgpuAspect] only the stencil of Depth24PlusStencil8 can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU

PS> $read.GetAwaiter().GetResult()
MethodInvocationException: Exception calling "GetResult" with "0" argument(s): "[WgpuAspect] only the stencil of Depth24PlusStencil8 can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU"
```

## Every method with a task

- `SubmitAsync`, `WaitAsync`, `RunAsync` and `DownloadAsync` on a [device](Types.md#wgpudevice)
- `ReadAsync`, `ReadRangeAsync`, `ReadMemoryAsync`, `WriteAsync`, `FillAsync`, `ClearAsync` and `CopyToAsync` on a [buffer](Types.md#wgpubuffer), and `ReadAsync` on a [slice](Types.md#wgpubufferslice) of one
- `DispatchAsync` on a [compute pipeline](Types.md#wgpucomputepipeline)
- `ReadAsync` and `ReadNanosecondsAsync` on a [query set](Types.md#wgpuqueryset)
- `ReadAsync` and `SaveAsync` on a [texture](Types.md#wgputexture), and the Async form of each texture method that makes a new texture: `GenerateMipsAsync`, `CompressAsync`, `ResizeAsync`, `CropAsync`, `PadAsync`, `FlipAsync`, `RotateAsync`, `SwizzleAsync`, `PremultiplyAsync`, `UnpremultiplyAsync`, `ConvertColorSpaceAsync`, `AssignColorSpaceAsync`, `GrayscaleAsync`, `InvertAsync`, `AdjustAsync`, `ToneMapAsync`, `ColorKeyAsync`, `AlphaThresholdAsync`, `FixAlphaBorderAsync` and `DifferenceAsync`; and `CompareAsync` and `MeasureAsync`, whose results are rows

[How To Use Threads And Runspaces](How-To-Use-Threads-And-Runspaces.md) runs GPU work from several threads at once.
