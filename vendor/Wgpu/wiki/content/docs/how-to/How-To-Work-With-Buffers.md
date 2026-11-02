---
title: How To Work With Buffers
weight: 4
---

Run a computation that takes many steps without moving its data off the GPU between them: heat spreading along a rod, one step at a time, between two buffers that take turns. Partway through, read a few cells to see how it goes, heat one end by writing into part of a buffer, keep a copy, and at the end clear the buffers or free them. [Buffers](Buffers.md) has each part on its own.

## The step

Each step reads every cell of one buffer and writes the next state into another: a cell moves toward the average of its two neighbors, and the cells at the ends keep their values. A step cannot write the buffer it reads, since a cell would then read a neighbor's new value as well as an old one:

```powershell
PS> Get-Content ./diffuse.wgsl
override rate: f32 = 0.25;
@group(0) @binding(0) var<storage, read> before: array<f32>;
@group(0) @binding(1) var<storage, read_write> after: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = arrayLength(&before);
    let i = id.x;
    if (i >= n) { return; }
    if (i == 0u || i == n - 1u) { after[i] = before[i]; return; }
    after[i] = before[i] + rate * (before[i - 1u] - 2.0 * before[i] + before[i + 1u]);
}
PS> $step = New-WgpuComputePipeline (Get-Content ./diffuse.wgsl -Raw)
```

## Two buffers that take turns

A rod of 64 cells, cold but for four hot ones in the middle, goes to the GPU once. The second buffer starts as 64 zeros:

```powershell
PS> $start = [float[]]::new(64)
PS> foreach ($cell in 30..33) { $start[$cell] = 100 }
PS> $current = New-WgpuBuffer $start
PS> $next = New-WgpuBuffer -Length 64 -ElementType Float32
PS> $current, $next | Select-Object Label, ElementType, Length, Size
Label        ElementType Length Size
-----        ----------- ------ ----
64 x Float32     Float32     64  256
64 x Float32     Float32     64  256
```

Each step reads `$current` and writes `$next`, and then the two swap, so after an even number of steps the newest state is in `$current` again. A buffer is bound where it is, so nothing travels back between steps:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> for ($n = 0; $n -lt 100; $n++) {
>>     Invoke-WgpuCompute $step @{ before = $current; after = $next }
>>     $current, $next = $next, $current
>> }
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> for ($n = 0; $n -lt 100; $n++) {
>>     $step.Dispatch(@{ before = $current; after = $next }, 1)
>>     $current, $next = $next, $current
>> }
```
{{< /tab >}}
{{< /tabs >}}

## Look at a few cells

`-Offset` and `-Count` read part of a buffer, counted in elements, so a check on the middle of the rod copies 8 floats, not 64. `Read-WgpuBuffer` writes what it reads as one array, which is fastest for a large buffer, and `-Enumerate` writes the elements one by one, for the pipeline. The heat has spread out from the four hot cells:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuBuffer $current -Offset 28 -Count 8 -Enumerate | ForEach-Object { [math]::Round($_, 1) }) -join ', '
19.8, 21, 21.8, 22.2, 22.2, 21.8, 21, 19.8
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> ($current.ReadRange(28, 8) | ForEach-Object { [math]::Round($_, 1) }) -join ', '
19.8, 21, 21.8, 22.2, 22.2, 21.8, 21, 19.8
```
{{< /tab >}}
{{< /tabs >}}

Measured on the GPU, nearly all the heat of the four cells of 100 is still in the rod. The little that reached the ends is gone, since the end cells keep their values and so take in heat without warming:

```powershell
PS> [math]::Round((Measure-WgpuArray $current).Sum, 2)
399.99
```

## Keep a copy, then heat one end

`Copy-WgpuBuffer` copies between buffers on the GPU, here to keep the state after 100 steps. `Write-WgpuBuffer` with `-Offset` writes part of a buffer, here the first cell, which the step keeps as it is, so from now on the left end stays at 100 and heats the rod:

```powershell
PS> $kept = New-WgpuBuffer -Length 64 -ElementType Float32
PS> Copy-WgpuBuffer $current $kept
PS> Write-WgpuBuffer $current ([float[]](100)) -Offset 0
PS> for ($n = 0; $n -lt 100; $n++) {
>>     Invoke-WgpuCompute $step @{ before = $current; after = $next }
>>     $current, $next = $next, $current
>> }
PS> (Read-WgpuBuffer $current -Count 6 -Enumerate | ForEach-Object { [math]::Round($_, 1) }) -join ', '
100, 88.9, 77.9, 67.5, 57.6, 48.5

PS> (Read-WgpuBuffer $kept -Count 6 -Enumerate | ForEach-Object { [math]::Round($_, 1) }) -join ', '
0, 0, 0, 0, 0, 0
```

The cells next to the left end warmed, and the copy kept the rod as it was after 100 steps, its left end still cold.

## Start again, or free the buffers

`Clear-WgpuBuffer` sets every element to 0, or to the number `-Value` gives, without a copy from PowerShell:

```powershell
PS> Clear-WgpuBuffer $current
PS> Clear-WgpuBuffer $next -Value 20
PS> (Measure-WgpuArray $current).Maximum, (Measure-WgpuArray $next).Minimum
0
20
```

A buffer's GPU memory is freed when nothing holds the buffer any longer, or at once with `Remove-WgpuBuffer`, after which wgpu refuses every use of it:

```powershell
PS> $current, $next, $kept | Remove-WgpuBuffer
PS> Read-WgpuBuffer $kept
Read-WgpuBuffer: the buffer cannot be read back: Validation Error

Caused by:
  In Queue::submit
    Buffer with '64 x Float32' label has been destroyed
```

A buffer holds numbers of one element type, Float32 here; [Buffers](Buffers.md) lists the eleven and what each converts to, and [How To Record Commands By Hand](How-To-Record-Commands-By-Hand.md) puts many steps in one submission.
