---
title: Arrays And Buffers
weight: 2
---

A GPU computes on numbers held in its own memory. This chapter sends an array there and gets the results back, keeps numbers there for several steps in a buffer, and works on them where they are, all without writing a shader.

## Send an array, get the results

`Invoke-WgpuMap` runs a WGSL expression over every element of an array on the GPU. The element is named `x`:

```powershell
PS> Invoke-WgpuMap 'x * x' ([float[]](1, 2, 3, 4))
1
4
9
16
```

The module copied the four numbers to the GPU, ran the expression once for each of them at the same time, and copied the four results back. The numbers went as `Float32`, 32-bit floats, because the array is a `[float[]]`: the .NET type of an array decides the type its numbers have on the GPU, which WGSL calls `f32` for these. An `[int[]]` goes as `Int32`, WGSL's `i32`, and an expression over it computes in whole numbers:

```powershell
PS> Invoke-WgpuMap 'x / 2' ([int[]](7, 8, 9))
3
4
4
```

## Keep the numbers on the GPU

Sending numbers to the GPU and back often takes longer than computing on them. A buffer keeps an array in the GPU's memory between calls, so a chain of steps sends the numbers once and brings back only what the last step wrote:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $values = New-WgpuBuffer ([float[]](1..8))
PS> $values
Label       ElementType Length Size Adapter
-----       ----------- ------ ---- -------
8 x Float32 Float32          8   32 NVIDIA GeForce RTX 5070

PS> Read-WgpuBuffer $values
1
2
3
4
5
6
7
8
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $values = $gpu.Upload([float[]](1..8))
PS> $values
Label       ElementType Length Size Adapter
-----       ----------- ------ ---- -------
8 x Float32 Float32          8   32 NVIDIA GeForce RTX 5070

PS> $values.Read()
1
2
3
4
5
6
7
8
```
{{< /tab >}}
{{< /tabs >}}

The buffer shows its element type, its length in elements, its size in bytes, and the adapter whose memory holds it. Its label says the same in a few words, and graphics debuggers show it.

## Change it in place

`Write-WgpuBuffer` overwrites elements from `-Offset`, counted in elements, `Copy-WgpuBuffer` copies from one buffer into another on the GPU, and `Clear-WgpuBuffer` sets every element to zero, or to `-Value`:

```powershell
PS> Write-WgpuBuffer $values ([float[]](100, 200)) -Offset 2
PS> Read-WgpuBuffer $values -Count 4
1
2
100
200

PS> $saved = New-WgpuBuffer -Length 8
PS> Copy-WgpuBuffer $values $saved
PS> Clear-WgpuBuffer $values -Value 1
PS> Read-WgpuBuffer $values -Count 4
1
1
1
1

PS> Read-WgpuBuffer $saved -Count 4
1
2
100
200
```

## Work on it where it is

`Invoke-WgpuMap -AsBuffer` leaves its results on the GPU as a new buffer, and `Measure-WgpuArray` reduces a buffer on the GPU, so only its count, sum, average, minimum and maximum come back:

```powershell
PS> $squares = Invoke-WgpuMap 'x * x' ([float[]](1..1000)) -AsBuffer
PS> Measure-WgpuArray $squares
Count       Sum    Average Minimum Maximum
-----       ---    ------- ------- -------
 1000 333833504 333833.504       1 1000000
```

The thousand squares never left the GPU; five numbers did. The sum is 333833504 where the exact sum is 333833500: a Float32 keeps about seven significant digits, so a sum this large is rounded to the nearest number one can hold.

## Where next

[Compute Shaders](Compute-Shaders.md) writes the shader that `Invoke-WgpuMap` wrote for you, to do what one expression cannot.
