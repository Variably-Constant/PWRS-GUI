---
title: Compute
weight: 30
---

A compute shader is a program the GPU runs many times at once. It declares its resources as bindings, each numbered by a group and a binding, and an entry point that runs once for each invocation; invocations run in workgroups of the size the shader gives. `Invoke-WgpuCompute` reads a shader's bindings from the shader itself, so it knows what each one takes and which ones the shader may write. An array given to it is copied to the GPU, and copied back when the shader may have written it; a buffer stays on the GPU. `Invoke-WgpuMap` writes the shader around an expression, and `Measure-WgpuArray` sums and finds the extremes of an array, both with no shader to write. The parameters are on [Compute](Compute-Cmdlets.md) under Cmdlets and types, and the `Wgpu.ComputePipeline` class on [Types](Types.md#wgpucomputepipeline).

## Run a shader over an array

A shader that doubles every element of the array it binds. The `arrayLength` test keeps the invocations of the last workgroup that run past the end of the array from writing:

```powershell
PS> Get-Content ./double.wgsl
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
PS> $double = Get-Content ./double.wgsl -Raw
```

`Invoke-WgpuCompute` copies the array to the GPU as the element type the shader declares, runs one invocation per element, and writes the array back. The device's `Run` method binds the same way but counts workgroups and leaves its results in a buffer:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> Invoke-WgpuCompute $double ([float[]](1..4))
2
4
6
8

PS> (Invoke-WgpuCompute $double ([int[]](1, 2, 3))).GetType().Name
Single[]
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $data = $gpu.Upload([float[]](1..4))
PS> $gpu.Run($double, $data, 1)
PS> $gpu.Download($data)
2
4
6
8
```
{{< /tab >}}
{{< /tabs >}}

## Give a shader its resources

A shader with two bindings, of which it writes only the first:

```powershell
PS> Get-Content ./add.wgsl
@group(0) @binding(0) var<storage, read_write> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@compute @workgroup_size(64)
fn sum(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&a)) { a[id.x] = a[id.x] + b[id.x]; }
}
PS> $add = Get-Content ./add.wgsl -Raw
```

Its resources are an array in the order the shader declares its bindings, a hashtable by binding name, or a hashtable by `group:binding`; a shader with one binding also takes the resource alone. Only the binding the shader may write comes back:

```powershell
PS> Invoke-WgpuCompute $add @([float[]](1, 2), [float[]](10, 20))
11
22

PS> Invoke-WgpuCompute $add @{ b = [float[]](10, 20); a = [float[]](1, 2) }
11
22

PS> Invoke-WgpuCompute $add @{ '0:0' = [float[]](1, 2); '0:1' = [float[]](5, 5) }
6
7

PS> Invoke-WgpuCompute $add @{ a = [float[]](1) }
Invoke-WgpuCompute: no resource was given for b (0:1)
```

The `Run` and `Dispatch` methods, `New-WgpuBindGroup` and the `BindGroup` methods take resources in the same forms.

## Pack a struct, set a constant

A uniform or storage binding of a struct takes a hashtable of its members, packed as WGSL lays the struct out, padding included. A member left out is zero, and one the struct does not have is refused. An override constant takes its value from `-Constant`, by name or by its `@id`:

```powershell
PS> Get-Content ./affine.wgsl
struct Params { scale: f32, offset: f32 }
override bias: f32 = 0.0;
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * p.scale + p.offset + bias; }
}
PS> $affine = Get-Content ./affine.wgsl -Raw
PS> Invoke-WgpuCompute $affine @{ v = [float[]](1, 2, 3); p = @{ scale = 10; offset = 0.5 } }
10.5
20.5
30.5

PS> Invoke-WgpuCompute $affine @{ v = [float[]](1, 2, 3); p = @{ scale = 10; offset = 0.5 } } -Constant @{ bias = 100 }
110.5
120.5
130.5

PS> Invoke-WgpuCompute $affine @{ v = [float[]](1); p = @{ sacle = 1 } }
Invoke-WgpuCompute: Params has no member sacle; its members are scale, offset
```

## Size the run

Without a size, the run has one invocation per element of the first buffer it binds. `-Workgroups` counts workgroups in x, y and z, so one workgroup of 64 over 100 elements reaches the first 64; `-Invocations` counts invocations and rounds each dimension up to whole workgroups:

```powershell
PS> $out = Invoke-WgpuCompute $double ([float[]](1..100)) -Workgroups 1
PS> $out[63], $out[64]
128
65

PS> (Invoke-WgpuCompute $double ([float[]](1..100)) -Invocations 100)[99]
200
```

A run over a grid sizes both dimensions. This shader fills a 4 by 3 grid, one invocation per cell, with the cell's column times 10 plus its row:

```powershell
PS> Get-Content ./grid.wgsl
struct Size { width: u32, height: u32 }
@group(0) @binding(0) var<uniform> size: Size;
@group(0) @binding(1) var<storage, read_write> grid: array<u32>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < size.width && id.y < size.height) { grid[id.y * size.width + id.x] = id.x * 10u + id.y; }
}
PS> $grid = Invoke-WgpuCompute (Get-Content ./grid.wgsl -Raw) @{ size = @{ width = 4; height = 3 }; grid = [uint32[]]::new(12) } -Invocations 4, 3
PS> $grid -join ','
0,10,20,30,1,11,21,31,2,12,22,32
```

`-Indirect` reads the three workgroup counts from a buffer on the GPU, which another shader can have written:

```powershell
PS> $counts = New-WgpuBuffer ([uint32[]](1, 1, 1))
PS> $out = Invoke-WgpuCompute $double ([float[]](1..100)) -Indirect $counts
PS> $out[63], $out[64]
128
65
```

A dispatch is limited to the device's `MaxComputeWorkgroupsPerDimension` workgroups in each of x, y and z, so a run in one dimension larger than that is refused; it has to spread over y, with the shader working out its index from both. `Invoke-WgpuMap` does that itself.

## Keep data on the GPU between runs

A buffer stays on the GPU: nothing comes back from a run over it, and the next run reads what the last one wrote:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $buffer = New-WgpuBuffer ([float[]](1..4))
PS> Invoke-WgpuCompute $double $buffer
PS> Invoke-WgpuCompute $double $buffer
PS> Read-WgpuBuffer $buffer
4
8
12
16
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $buffer = $gpu.Upload([float[]](1..4))
PS> $gpu.Run($double, $buffer, 1)
PS> $gpu.Run($double, $buffer, 1)
PS> $buffer.Read()
4
8
12
16
```
{{< /tab >}}
{{< /tabs >}}

## Run GLSL or SPIR-V

A GLSL compute shader runs as it is, recognized by its `#version` line or named by `-Language Glsl`:

```powershell
PS> Get-Content ./increment.comp
#version 450
layout(local_size_x = 32) in;
layout(set = 0, binding = 0) buffer Data { float values[]; };
void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i < values.length()) { values[i] += 1.0; }
}
PS> Invoke-WgpuCompute (Get-Content ./increment.comp -Raw) ([float[]](1, 2, 3))
2
3
4
```

SPIR-V runs as a shader object, which `New-WgpuShader -SpirV` makes from its words:

```powershell
PS> $spirv = New-WgpuShader -SpirV (ConvertTo-WgpuShader (Get-Content ./increment.comp -Raw) -To SpirV)
PS> Invoke-WgpuCompute $spirv ([float[]](4, 5))
5
6
```

## Build a pipeline once

`Invoke-WgpuCompute` compiles shader text for the device on every call. A pipeline is the shader compiled once for one entry point, with its override constants fixed, and runs any number of times:

```powershell
PS> Get-Content ./scale.wgsl
override factor: f32 = 2.0;
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * factor; }
}
PS> $scale = Get-Content ./scale.wgsl -Raw
```

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $pipeline = New-WgpuComputePipeline $scale -Constant @{ factor = 10 } -Label tenfold
PS> $pipeline
Label   EntryPoint WorkgroupSize Bindings
-----   ---------- ------------- --------
tenfold main       {64, 1, 1}    {v}

PS> Invoke-WgpuCompute $pipeline ([float[]](1, 2))
10
20
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $pipeline = $gpu.CreateComputePipeline($scale, $null, @{ factor = 10 }, 'tenfold')
PS> $pipeline
Label   EntryPoint WorkgroupSize Bindings
-----   ---------- ------------- --------
tenfold main       {64, 1, 1}    {v}

PS> $values = $gpu.Upload([float[]](1, 2))
PS> $pipeline.Dispatch($values, 1)
PS> $values.Read()
10
20
```
{{< /tab >}}
{{< /tabs >}}

A pipeline lists the bindings its entry point uses:

```powershell
PS> $pipeline.Bindings | Select-Object Name, Kind, ElementType
Name          Kind ElementType
----          ---- -----------
v    StorageBuffer     Float32
```

A shader with more than one compute entry point needs the one to build named:

```powershell
PS> Get-Content ./two.wgsl
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn add_one(@builtin(global_invocation_id) id: vec3<u32>) { if (id.x < arrayLength(&v)) { v[id.x] += 1.0; } }
@compute @workgroup_size(64)
fn negate(@builtin(global_invocation_id) id: vec3<u32>) { if (id.x < arrayLength(&v)) { v[id.x] = -v[id.x]; } }
PS> New-WgpuComputePipeline (Get-Content ./two.wgsl -Raw)
New-WgpuComputePipeline: the shader has several Compute entry points; name one with -Entry

PS> $negate = New-WgpuComputePipeline (Get-Content ./two.wgsl -Raw) -Entry negate
PS> Invoke-WgpuCompute $negate ([float[]](1, -2))
-1
2
```

## Bind resources ahead of a pass

A bind group fixes the resources of one group of a pipeline's bindings, for a compute pass recorded by hand to set; [Recording](Recording.md) shows the pass. A pipeline's `BindGroup` method and the device's `CreateBindGroup` make one as `New-WgpuBindGroup` does.

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $sum = New-WgpuComputePipeline $add
PS> $left = New-WgpuBuffer ([float[]](1, 2, 3))
PS> $right = New-WgpuBuffer ([float[]](10, 20, 30))
PS> $group = New-WgpuBindGroup $sum @{ a = $left; b = $right }
PS> $group
Label                   Group Bindings
-----                   ----- --------
Wgpu shader sum group 0     0 {a, b}

PS> New-WgpuBindGroup $sum $left, $right -Group 1
New-WgpuBindGroup: the pipeline has no bind group 1; it has 1
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $sum = $gpu.CreateComputePipeline($add, $null, $null, $null)
PS> $left = $gpu.Upload([float[]](1, 2, 3))
PS> $right = $gpu.Upload([float[]](10, 20, 30))
PS> $group = $sum.BindGroup(0, @{ a = $left; b = $right })
PS> $group
Label                   Group Bindings
-----                   ----- --------
Wgpu shader sum group 0     0 {a, b}

PS> $gpu.CreateBindGroup($sum, 0, @($left, $right)).Bindings
a
b

PS> $sum.BindGroup(1, @($left, $right))
MethodInvocationException: Exception calling "BindGroup" with "2" argument(s): "[WgpuNoGroup] the pipeline has no bind group 1; it has 1"
```
{{< /tab >}}
{{< /tabs >}}

## Time a run

`-PassThru` writes a `Wgpu.ComputeResult` with every array by binding name, the workgroups dispatched, the time the whole call took on the CPU, and the time the pass took on the GPU where the device has timestamp queries. The times differ from run to run, so this compares them: the GPU time covers the pass alone, and the CPU time the whole call, the copies to and from the GPU included.

```powershell
PS> $result = Invoke-WgpuCompute $add @{ a = [float[]](1, 2); b = [float[]](3, 4) } -PassThru
PS> $result.Outputs['a']
4
6

PS> $result.Workgroups -join ','
1,1,1

PS> $result.GpuTimeNs -gt 0
True

PS> $result.GpuTimeNs -lt $result.CpuTimeMs * 1e6
True
```

## Map an expression over arrays

`Invoke-WgpuMap` writes a shader around a WGSL expression in `x`, the element, and `i`, its index as a `u32`, and runs one invocation per element:

```powershell
PS> Invoke-WgpuMap 'x * x + 1.0' ([float[]](1, 2, 3))
2
5
10

PS> Invoke-WgpuMap 'f32(i) / 4.0' ([float[]]::new(5))
0
0.25
0.5
0.75
1

PS> 1..4 | Invoke-WgpuMap 'x + 0.5'
1.5
2.5
3.5
4.5
```

The expression is WGSL, so its built-in functions are there, and so are its strict types: a float array's `x` is an `f32` and `i` a `u32`, and WGSL converts a literal to the type it meets but never one variable's type to another's. An expression that does not compile is reported at its line in the shader written around it:

```powershell
PS> Invoke-WgpuMap 'clamp(x, 0.0, 1.0)' ([float[]](-0.5, 0.25, 3))
0
0.25
1

PS> Invoke-WgpuMap 'select(0.0, 1.0, x > 2.0)' ([float[]](1, 2, 3, 4))
0
0
1
1

PS> Invoke-WgpuMap 'x * 2' ([float[]](1.5))
3

PS> Invoke-WgpuMap 'x * f32(i)' ([float[]](1.5, 1.5))
0
1.5

PS> Invoke-WgpuMap 'x * i' ([float[]](1.5))
Invoke-WgpuMap: the shader is invalid: line 8, column 25: Entry point main at Compute is invalid: Expression [17] is invalid: Operation Multiply can't work with [14] (of type Scalar(Scalar { kind: Float, width: 4 })) and [8] (of type Scalar(Scalar { kind: Uint, width: 4 }))
```

`-Variable` gives more names: an array is read element by element like `x`, and must be at least as long, and a number is a constant written into the shader:

```powershell
PS> $prices = [float[]](10, 20, 30)
PS> $amounts = [int[]](3, 1, 2)
PS> Invoke-WgpuMap 'x * f32(amount) * (1.0 + tax)' $prices -Variable @{ amount = $amounts; tax = 0.25 }
37.5
25
75
```

The results have the type of `x` unless `-ElementType` names another, and an array of 8-bit or 16-bit integers, which has no WGSL type, is refused:

```powershell
PS> Invoke-WgpuMap 'u32(x) % 3u' ([float[]](3, 4, 5)) -ElementType UInt32
0
1
2

PS> Invoke-WgpuMap 'x / 2' ([int[]](7, 8))
3
4

PS> Invoke-WgpuMap 'x' ([byte[]](1, 2))
Invoke-WgpuMap: x is UInt8, which WGSL has no type for; convert it to Int32, UInt32 or Float32 first
```

`-AsBuffer` leaves the results on the GPU as a buffer, for the next cmdlet to read:

```powershell
PS> $squares = Invoke-WgpuMap 'x * x' ([float[]](1..100)) -AsBuffer
PS> $squares
Label          ElementType Length Size Adapter
-----          ----------- ------ ---- -------
Invoke-WgpuMap Float32        100  400 NVIDIA GeForce RTX 5070
```

## Sum, average and find the extremes

`Measure-WgpuArray` reduces an array or a buffer on the GPU to its count, sum, average, minimum and maximum, as `Measure-Object` does on the CPU:

```powershell
PS> Measure-WgpuArray $squares
Count    Sum Average Minimum Maximum
-----    --- ------- ------- -------
  100 338350  3383.5       1   10000

PS> Measure-WgpuArray ([int[]](4, -7, 12, 0)) | Format-List
Count   : 4
Sum     : 9
Average : 2.25
Minimum : -7
Maximum : 12

PS> (1..1000 | Measure-WgpuArray).Average
500.5
```

A sum of 32-bit integers does not overflow: they are added in 64 bits, and the workgroups' sums in 128 bits. 64-bit integers are added in 64 bits within each workgroup, which overflows only when a workgroup's sum leaves their range. `Sum` is a double, which holds every integer up to 2^53:

```powershell
PS> (Measure-WgpuArray ([uint32[]](@(4000000000) * 1000))).Sum
4000000000000
```

A float sum adds a workgroup's elements in their own precision and the workgroups' sums in double, so a Float32 sum agrees with the one `Measure-Object` adds in double to about seven significant digits, as far as Float32 itself holds:

```powershell
PS> $harmonic = [float[]](1..10000 | ForEach-Object { 1 / $_ })
PS> (Measure-WgpuArray $harmonic).Sum
9.78760586376302

PS> ($harmonic | Measure-Object -Sum).Sum
9.78760610157042
```

8-bit and 16-bit integers have no WGSL type, so `-ElementType` measures them as another element type, and without it they are refused:

```powershell
PS> (Measure-WgpuArray ([byte[]](250, 251, 252)) -ElementType UInt32).Sum
753

PS> Measure-WgpuArray ([byte[]](1, 2))
Measure-WgpuArray: the array is UInt8, which WGSL has no type for; convert it to Int32, UInt32 or Float32 first
```

An expression sees one element of each array at a time. Work that reads other elements, writes elsewhere than its own element, or shares memory across a workgroup needs a shader of its own.
