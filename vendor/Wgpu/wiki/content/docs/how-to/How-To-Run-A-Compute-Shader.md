---
title: How To Run A Compute Shader
weight: 2
---

Count a large array of numbers into a histogram on the GPU: a compute shader with the counts it adds to as atomics, its range given as a struct and its number of bins as an override, run once over an array, and then built once and run over a stream of batches whose counts add up on the GPU. [Compute](Compute.md) has each part on its own, and [Compute Shaders](Compute-Shaders.md) teaches the shader language from the start.

## The shader

Each run reads one number, works out which bin of the range it belongs to, and adds one to that bin's count. Many runs add to the same bins at once, so the counts are `atomic<u32>`, which the GPU adds to without losing a count:

```powershell
PS> Get-Content ./histogram.wgsl
override bins: u32 = 16u;
struct Range { low: f32, high: f32 }
@group(0) @binding(0) var<storage, read> values: array<f32>;
@group(0) @binding(1) var<uniform> range: Range;
@group(0) @binding(2) var<storage, read_write> counts: array<atomic<u32>>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= arrayLength(&values)) { return; }
    let t = (values[id.x] - range.low) / (range.high - range.low);
    let bin = min(u32(max(t, 0.0) * f32(bins)), bins - 1u);
    atomicAdd(&counts[bin], 1u);
}
PS> $histogram = Get-Content ./histogram.wgsl -Raw
```

A number below the range counts in the first bin and one above it in the last.

## Count an array

The numbers here are the sums of two random numbers from 0 to 1, so they run from 0 to 2 and gather around 1. A seeded `Random` makes the same ones every time:

```powershell
PS> $random = [Random]::new(7)
PS> $values = [float[]]::new(100000)
PS> for ($i = 0; $i -lt $values.Length; $i++) { $values[$i] = $random.NextDouble() + $random.NextDouble() }
```

`Invoke-WgpuCompute` binds each resource by its name in the shader: the array, the struct as a hashtable of its members, which the module lays out as WGSL does, and an array of zeros for the counts, the one binding the shader writes, which comes back. With no size given, it runs once for each number of the first array bound:

```powershell
PS> $counts = Invoke-WgpuCompute $histogram @{ values = $values; range = @{ low = 0; high = 2 }; counts = [uint32[]]::new(16) }
PS> ($counts | Measure-Object -Sum).Sum
100000

PS> $counts | ForEach-Object { '{0,5} {1}' -f $_, ('#' * [math]::Round($_ / 500)) }
  762 ##
 2412 #####
 3982 ########
 5540 ###########
 7087 ##############
 8641 #################
10116 ####################
11882 ########################
11593 #######################
10158 ####################
 8509 #################
 6916 ##############
 5483 ###########
 3929 ########
 2253 #####
  737 #
```

Every number was counted once, and the counts rise to the middle and fall, as sums of two random numbers do.

## Choose the number of bins

The number of bins is an override, fixed as the shader compiles, so `-Constant` gives another; the counts array needs as many elements:

```powershell
PS> $eight = Invoke-WgpuCompute $histogram @{ values = $values; range = @{ low = 0; high = 2 }; counts = [uint32[]]::new(8) } -Constant @{ bins = 8 }
PS> $eight -join ','
3174,9522,15728,21998,21751,15425,9412,2990
```

`-Invocations` asks for that many runs instead, rounded up to whole workgroups of 256, so 1000 runs it 1024 times:

```powershell
PS> $first = Invoke-WgpuCompute $histogram @{ values = $values; range = @{ low = 0; high = 2 }; counts = [uint32[]]::new(8) } -Constant @{ bins = 8 } -Invocations 1000
PS> ($first | Measure-Object -Sum).Sum
1024
```

Every one of the 1024 runs counted a number, since the test at the top of the shader checks the array's length, 100,000, and not the 1000 runs asked for. A shader that must stop at a count of its own reads it from a uniform or an override.

## Count a stream in batches

Numbers that arrive in batches, or that are too many to hold at once, count into one buffer that stays on the GPU: each batch adds to the counts already there, and they are read once at the end. A pipeline compiles the shader once for all the batches:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $pipeline = New-WgpuComputePipeline $histogram -Constant @{ bins = 16 }
PS> $total = New-WgpuBuffer -Length 16 -ElementType UInt32
PS> for ($start = 0; $start -lt $values.Length; $start += 25000) {
>>     $batch = $values[$start..($start + 24999)]
>>     Invoke-WgpuCompute $pipeline @{ values = [float[]]$batch; range = @{ low = 0; high = 2 }; counts = $total }
>> }
PS> (Read-WgpuBuffer $total) -join ',' -eq ($counts -join ',')
True
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $pipeline = $gpu.CreateComputePipeline($histogram, $null, @{ bins = 16 }, $null)
PS> $total = $gpu.Zeros(16, 'UInt32')
PS> for ($start = 0; $start -lt $values.Length; $start += 25000) {
>>     $batch = [float[]]$values[$start..($start + 24999)]
>>     $pipeline.Dispatch(@{ values = $batch; range = @{ low = 0; high = 2 }; counts = $total }, [math]::Ceiling($batch.Length / 256))
>> }
PS> $total.Read() -join ',' -eq ($counts -join ',')
True
```
{{< /tab >}}
{{< /tabs >}}

Four batches of 25,000 counted the same as the whole array at once. A `Wgpu.Buffer` is bound where it is, so nothing came back after each batch; `Clear-WgpuBuffer` zeroes it to start counting again. `Read-WgpuBuffer` writes what it reads as one array, and `-Enumerate` writes each element to the pipeline instead, for `Measure-Object` to add up:

```powershell
PS> Clear-WgpuBuffer $total
PS> (Read-WgpuBuffer $total -Enumerate | Measure-Object -Sum).Sum
0
```

[Buffers](Buffers.md) has more on keeping data on the GPU, and [How To Map And Reduce Arrays](How-To-Map-And-Reduce-Arrays.md) sums, averages and maps arrays with no shader to write.
