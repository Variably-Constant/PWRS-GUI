---
title: Tasks and threads
weight: 80
---

A cmdlet, and a method, waits until the GPU has done what it asked. Each method that waits has a form ending in `Async` that returns a .NET task at once and waits on a thread of its own, so a script can start GPU work and do something else while it runs. The module keeps its devices, and everything made on them, in the process rather than in a runspace, so every runspace and thread of a process shares them. The methods are on [Types](Types.md#wgpudevice), under each class.

## Wait for the GPU

Work submitted without waiting, here through the device's `SubmitAsync` method, is waited for by waiting for the device, which returns once the GPU has finished all the work it was given:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $buffer = New-WgpuBuffer ([float[]](1..4))
PS> $encoder = New-WgpuCommandEncoder
PS> $encoder.ClearBuffer($buffer)
PS> $submitted = (Get-WgpuDevice).SubmitAsync(@($encoder.Finish()))
PS> Wait-WgpuDevice
PS> (Read-WgpuBuffer $buffer) -join ','
0,0,0,0
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $buffer = $gpu.Upload([float[]](1..4))
PS> $encoder = $gpu.CreateCommandEncoder($null)
PS> $encoder.ClearBuffer($buffer)
PS> $submitted = $gpu.SubmitAsync(@($encoder.Finish()))
PS> $gpu.Wait()
PS> $buffer.Read() -join ','
0,0,0,0
```
{{< /tab >}}
{{< /tabs >}}

## Get a task in place of a wait

These methods return a task:

- `SubmitAsync`, `WaitAsync`, `RunAsync` and `DownloadAsync` of a device
- `ReadAsync`, `ReadRangeAsync`, `ReadMemoryAsync`, `WriteAsync`, `FillAsync`, `ClearAsync` and `CopyToAsync` of a buffer, and `ReadAsync` of a slice of one
- `DispatchAsync` of a compute pipeline
- `ReadAsync` and `ReadNanosecondsAsync` of a query set
- `ReadAsync` and `SaveAsync` of a texture, and the Async form of each method that makes a new texture from one: `GenerateMipsAsync`, `CompressAsync`, `ResizeAsync`, `CropAsync`, `PadAsync`, `FlipAsync`, `RotateAsync`, `SwizzleAsync`, `PremultiplyAsync`, `UnpremultiplyAsync`, `ConvertColorSpaceAsync`, `AssignColorSpaceAsync`, `GrayscaleAsync`, `InvertAsync`, `AdjustAsync`, `ToneMapAsync`, `ColorKeyAsync`, `AlphaThresholdAsync`, `FixAlphaBorderAsync` and `DifferenceAsync`; and `CompareAsync` and `MeasureAsync`, whose results are rows

A task's `GetAwaiter().GetResult()` waits for it to end, and returns what it holds:

```powershell
PS> $numbers = New-WgpuBuffer ([float[]](1..8))
PS> $reading = $numbers.ReadAsync()
PS> $reading.GetAwaiter().GetResult() -join ','
1,2,3,4,5,6,7,8
```

A buffer's tasks write, fill, copy and clear it, or hold what they read, and the device's `DownloadAsync` holds a buffer's values:

```powershell
PS> $values = New-WgpuBuffer ([int[]](1..8))
PS> $copy = New-WgpuBuffer ([int[]](@(7) * 8))
PS> $values.WriteAsync([int[]](5, 6)).GetAwaiter().GetResult()
PS> $values.ReadRangeAsync(0, 2).GetAwaiter().GetResult() -join ','
5,6

PS> $values.FillAsync(3).GetAwaiter().GetResult()
PS> $values.CopyToAsync($copy, 0, 0, 2).GetAwaiter().GetResult()
PS> $copy.Read() -join ','
3,3,7,7,7,7,7,7

PS> $values.ClearAsync().GetAwaiter().GetResult()
PS> $values.ReadMemoryAsync().GetAwaiter().GetResult().Length
8

PS> (Get-WgpuDevice).DownloadAsync($values).GetAwaiter().GetResult() -join ','
0,0,0,0,0,0,0,0
```

A texture's task holds the new texture its method makes, or for `SaveAsync`, nothing once the file is written:

```powershell
PS> $texel = New-WgpuTexture 1 1 -Data ([byte[]](200, 100, 50, 128))
PS> $texel.SwizzleAsync('gbra').GetAwaiter().GetResult().Read() -join ','
100,50,200,128

PS> $texel.PremultiplyAsync().GetAwaiter().GetResult().UnpremultiplyAsync().GetAwaiter().GetResult().Read() -join ','
199,100,50,128

PS> $texel.ConvertColorSpaceAsync('Srgb').GetAwaiter().GetResult().Format
Rgba8UnormSrgb

PS> $texel.AssignColorSpaceAsync('Srgb').GetAwaiter().GetResult().Format
Rgba8UnormSrgb

PS> $texel.GrayscaleAsync('Rec601').GetAwaiter().GetResult().Read() -join ','
124,124,124,128

PS> $texel.AdjustAsync($null, $null, $null, 1).GetAwaiter().GetResult().Read() -join ','
255,200,100,128

PS> $texel.ToneMapAsync('AgX').GetAwaiter().GetResult().Read() -join ','
139,94,65,128

PS> $texel.AlphaThresholdAsync(0.6).GetAwaiter().GetResult().Read() -join ','
200,100,50,0

PS> $tiles = New-WgpuTexture 2 2 -Data ([byte[]](1..16))
PS> $tiles.GenerateMipsAsync(0).GetAwaiter().GetResult().MipLevelCount
2

PS> $tiles.ResizeAsync(4, 4, 'Nearest').GetAwaiter().GetResult() | Select-Object Width, Height
Width Height
----- ------
    4      4

PS> $tiles.CropAsync(0, 0, 1, 2).GetAwaiter().GetResult().Read() -join ','
1,2,3,4,9,10,11,12

PS> $tiles.PadAsync(4, 2, 'Left', $null).GetAwaiter().GetResult().Width
4

PS> $tiles.FlipAsync('Vertical').GetAwaiter().GetResult().Read() -join ','
9,10,11,12,13,14,15,16,1,2,3,4,5,6,7,8

PS> (New-WgpuTexture 4 4).CompressAsync('Bc1RgbaUnorm', $null, $null).GetAwaiter().GetResult().Format
Bc1RgbaUnorm

PS> $tiles.SaveAsync("$PWD/tiles.png").GetAwaiter().GetResult()
PS> (Get-Item ./tiles.png).Length -gt 0
True
```

Each of these methods takes a `CancellationToken` last, which can be left out. `$null` does not convert to a token; leave it out, or give `[Threading.CancellationToken]::None`:

```powershell
PS> $numbers.ReadAsync($null, [Threading.CancellationToken]::None).GetAwaiter().GetResult()[0]
1

PS> $numbers.ReadAsync($null, $null)
MethodException: Cannot convert argument "cancellationToken", with value: "", for "ReadAsync" to type "System.Threading.CancellationToken": "Cannot convert null to type "System.Threading.CancellationToken"."
```

## Work while the GPU works

A shader that spins takes the place of real work here; one dispatch of it takes a few hundredths of a second on the GPU these pages ran on. The task is still running when the next command runs, and PowerShell is free in the meantime:

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
PS> $running = $spin.DispatchAsync($data, 1)
PS> $running.Status
WaitingForActivation

PS> (1..100000 | Measure-Object -Sum).Sum
5000050000

PS> $running.GetAwaiter().GetResult()
PS> $running.Status
RanToCompletion
```

## Wait for several tasks

`[Threading.Tasks.Task]::WaitAll` waits until every task given has ended. `WaitAny` returns the index of the first to end, and `WhenAll` and `WhenAny` return a task that ends with them. The `[Threading.Tasks.Task[]]` cast makes PowerShell's array of tasks the array those methods take:

```powershell
PS> $dispatches = 1..3 | ForEach-Object { $spin.DispatchAsync($data, 1) }
PS> [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]$dispatches)
PS> $dispatches.Status
RanToCompletion
RanToCompletion
RanToCompletion
```

## Read what a failed task holds

A method refuses at the call what it can check before the work starts, as its plain form would, so a shader with no compute entry point never becomes a task:

```powershell
PS> (Get-WgpuDevice).RunAsync('fn main() {}', $null, 1, 1, 1)
MethodInvocationException: Exception calling "RunAsync" with "5" argument(s): "[WgpuNoEntryPoint] the shader has no Compute entry point"
```

What fails once the work has started ends the task as Faulted. `GetAwaiter().GetResult()` throws the module's own error then, and the task's `Exception` holds it; `Wait()` throws it wrapped in an AggregateException, and `Result` gives `$null` with no error at all. To wait for a task without throwing, wait for `WhenAny` of it:

```powershell
PS> $both = New-WgpuTexture 2 2 -Format Depth24PlusStencil8
PS> $read = $both.ReadAsync(0, 0, 'DepthOnly')
PS> [Threading.Tasks.Task]::WhenAny($read).Wait()
PS> $read.Status
Faulted

PS> $read.Exception.InnerException.Message
[WgpuAspect] only the stencil of Depth24PlusStencil8 can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU

PS> $null -eq $read.Result
True

PS> $read.GetAwaiter().GetResult()
MethodInvocationException: Exception calling "GetResult" with "0" argument(s): "[WgpuAspect] only the stencil of Depth24PlusStencil8 can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU"
```

## Cancel a wait

A token from a `CancellationTokenSource` ends a wait when the source is canceled, within a tenth of a second, and the task ends as Canceled. It ends the wait only: the GPU still does the work it was given, and whatever waits for the device afterward waits for that work too. Ten dispatches queue some work, a wait for all of it is canceled, and the dispatches still finish:

```powershell
PS> $dispatches = 1..10 | ForEach-Object { $spin.DispatchAsync($data, 1) }
PS> $source = [Threading.CancellationTokenSource]::new()
PS> $waiting = (Get-WgpuDevice).WaitAsync($source.Token)
PS> $source.Cancel()
PS> [Threading.Tasks.Task]::WhenAny($waiting).Wait()
PS> $waiting.Status
Canceled

PS> [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]$dispatches)
PS> $dispatches.Status | Sort-Object -Unique
RanToCompletion
```

Ctrl+C stops a cmdlet that waits for the GPU within a tenth of a second, and the GPU finishes the work on its own. PowerShell cannot stop a method while it runs, though: a method that waits, a task's `Wait()` and `GetAwaiter().GetResult()` among them, returns only when the GPU is done. A loop of short waits gives Ctrl+C its chance between them:

```powershell
PS> $running = $spin.DispatchAsync($data, 1)
PS> while (-not $running.Wait(100)) { }
PS> $running.Status
RanToCompletion
```

## Share devices across runspaces

Every runspace of one process uses the same default device, and a buffer, texture or pipeline made in one works in all of them. A runspace made with `[powershell]::Create()` loads the module by name, as any session does, and reads a buffer the main runspace made, in Windows PowerShell as well:

```powershell
PS> $shared = New-WgpuBuffer ([float[]](1..8)) -Label shared
PS> $runspace = [powershell]::Create()
PS> $null = $runspace.AddScript({ param($buffer) "$($buffer.Label): $((Read-WgpuBuffer $buffer) -join ',')" }).AddArgument($shared)
PS> $runspace.Invoke()
shared: 1,2,3,4,5,6,7,8

PS> $runspace.Dispose()
```

In PowerShell 7, `ForEach-Object -Parallel` runs its script block for each input in runspaces of its own, at the same time. Each can run GPU work, and `$using:` hands them objects from the main runspace. The results come back as each runspace finishes, so they are sorted here:

<!-- pwsh 7 -->
```powershell
PS> 1..4 | ForEach-Object -Parallel { (Invoke-WgpuMap 'x * k' ([float[]](1, 2, 3)) -Variable @{ k = [float]$_ }) -join ',' } | Sort-Object
1,2,3
2,4,6
3,6,9
4,8,12

PS> 0, 4 | ForEach-Object -Parallel { ($using:shared).ReadRange($_, 4) -join ',' } | Sort-Object
1,2,3,4
5,6,7,8

PS> $made = 1..2 | ForEach-Object -Parallel { New-WgpuBuffer ([float[]]($_, $_)) }
PS> $made | ForEach-Object { (Read-WgpuBuffer $_) -join ',' } | Sort-Object
1,1
2,2
```

A thread job runs in a runspace of its own on a thread of its own, and hands back what it wrote when it is received. `Start-ThreadJob` comes with PowerShell 7; Windows PowerShell gets it from the ThreadJob module on the PowerShell Gallery:

<!-- pwsh 7 -->
```powershell
PS> $job = Start-ThreadJob { (Invoke-WgpuMap 'x + 1' ([float[]](1, 2))) -join ',' }
PS> Receive-Job $job -Wait -AutoRemoveJob
2,3
```

A device raises its engine events, `Wgpu.DeviceLost` and `Wgpu.UncapturedError`, in the runspace that opened it, and the default device in the runspace whose call opened it, so a script that calls `Get-WgpuDevice` in its main runspace before it starts any threads gets the default device's events there. A device opened with `[Wgpu.Device]::new` raises none. [Errors](Errors.md) shows what the events carry.

On Windows, NVIDIA's driver deadlocks the process when a thread that used OpenGL ends while another thread opens or closes a device, and wgpu uses OpenGL on every thread that looks for OpenGL adapters. So on Windows the module looks for adapters through Vulkan and Direct3D 12, and through OpenGL only when `-Backend Gl` asks for it; a script that asks for it there keeps OpenGL out of thread jobs and parallel runspaces, whose threads end with their work.
