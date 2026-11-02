---
title: How To Map And Reduce Arrays
weight: 3
---

Clean up and standardize a series of readings on the GPU without writing a shader: measure it, replace the readings a faulty sensor spiked, rescale them to a mean of 0 and a spread of 1, weigh them by a second series, and store them as small integers. `Invoke-WgpuMap` runs a WGSL expression over every element, and `Measure-WgpuArray` reduces an array or a buffer to its count, sum, average and extremes. [Compute](Compute.md) has each on its own.

## Measure the readings

Ten thousand readings of a temperature that swings with the time of day, with three spikes where the sensor failed:

```powershell
PS> $readings = [float[]]::new(10000)
PS> for ($i = 0; $i -lt $readings.Length; $i++) { $readings[$i] = 20 + 5 * [math]::Sin($i / 100) + ($i % 7) / 7 }
PS> foreach ($spike in 1234, 5678, 9012) { $readings[$spike] = 1000 }
PS> Measure-WgpuArray $readings
Count              Sum          Average          Minimum Maximum
-----              ---          -------          ------- -------
10000 207289.912841797 20.7289912841797 15.0000228881836    1000
```

Three spikes out of ten thousand readings pulled the maximum to 1000 and the average up by nearly 0.3.

## Replace the spikes

An expression over `x`, each element, gives each new element. `select` takes its second value where the condition holds and its first elsewhere, so a reading above 40, which this sensor never measures, becomes 20, the temperature the swing centers on:

```powershell
PS> $cleaned = Invoke-WgpuMap 'select(x, 20.0, x > 40.0)' $readings
PS> $clean = Measure-WgpuArray $cleaned
PS> $clean | Select-Object Minimum, Maximum, Average
Minimum Maximum Average
------- ------- -------
  15.00   25.86   20.43
```

## Rescale to a mean of 0 and a spread of 1

The spread, the standard deviation, is the square root of the average squared distance from the mean. A variable gives the expression the mean, and `-AsBuffer` leaves the squared distances on the GPU for `Measure-WgpuArray`, so they never come back:

```powershell
PS> $variance = (Invoke-WgpuMap '(x - mean) * (x - mean)' $cleaned -Variable @{ mean = $clean.Average } -AsBuffer | Measure-WgpuArray).Average
PS> $spread = [math]::Sqrt($variance)
PS> [math]::Round($spread, 3)
3.554

PS> $standard = Invoke-WgpuMap '(x - mean) / spread' $cleaned -Variable @{ mean = $clean.Average; spread = $spread }
PS> $check = Measure-WgpuArray $standard
PS> [math]::Abs($check.Average) -lt 0.0001
True

PS> [math]::Round($check.Minimum, 2), [math]::Round($check.Maximum, 2)
-1.53
1.53
```

The rescaled readings average 0, to within a ten-thousandth, and run about one and a half spreads either side of it, as a swing like this one does.

## Weigh by a second series

A variable that is an array is read element by element, as `x` is. Readings taken at night, the last third of each day here, count half, and the weighted average is the sum of the weighted readings over the sum of the weights:

```powershell
PS> $weights = [float[]]::new($cleaned.Length)
PS> for ($i = 0; $i -lt $weights.Length; $i++) { $weights[$i] = if ($i % 24 -ge 16) { 0.5 } else { 1.0 } }
PS> $weighted = Invoke-WgpuMap 'x * weight' $cleaned -Variable @{ weight = $weights } -AsBuffer | Measure-WgpuArray
PS> [math]::Round($weighted.Sum / (Measure-WgpuArray $weights).Sum, 3)
20.434
```

`i`, each element's index as `u32`, is in the expression too; `f32(i)` converts it, as WGSL converts no variable's type to another's by itself.

## Store them as small integers

`-ElementType` gives the results another type, and the expression must produce it. Each rescaled reading becomes a level from 0 to 255, three spreads either side of the mean:

```powershell
PS> $levels = Invoke-WgpuMap 'u32(clamp((x + 3.0) / 6.0 * 255.0, 0.0, 255.0))' $standard -ElementType UInt32
PS> $levels.GetType().Name
UInt32[]

PS> $levels[0..5] -join ','
122,124,126,129,131,133
```

A float that goes into an integer type is cut toward zero.

## How exact the sums are

`Measure-WgpuArray` adds 32-bit integers in 64 bits, and the workgroups' sums in 128 bits, so a sum of 32-bit integers does not overflow; 64-bit integers add in 64 bits within each workgroup, which overflows only when a workgroup's sum leaves their range. Sum is a double, which holds every integer up to 2^53. Floats are added in their own precision within each workgroup of 256 invocations and the workgroups' sums in double, so a Float32 sum is as exact as Float32 itself, about seven significant digits:

```powershell
PS> (Measure-WgpuArray ([uint32[]](@(4000000000) * 1000))).Sum
4000000000000

PS> $harmonic = [float[]](1..10000 | ForEach-Object { 1 / $_ })
PS> (Measure-WgpuArray $harmonic).Sum
9.78760586376302

PS> ($harmonic | Measure-Object -Sum).Sum
9.78760610157042
```

An expression sees one element of each array at a time. Work that reads an element's neighbors or writes elsewhere than its own element takes a shader of its own: [How To Run A Compute Shader](How-To-Run-A-Compute-Shader.md).
