---
title: Buffers
weight: 20
---

A buffer is GPU memory holding an array of one element type. An array given to a cmdlet such as `Invoke-WgpuCompute` is copied to the GPU and back on every call, while a buffer keeps its data on the GPU between calls, so a chain of runs over it copies the data there once and back once. A buffer belongs to the device it was made on, the default device unless `-Device` names another. The cmdlets' parameters are on [Buffers](Buffer-Cmdlets.md) under Cmdlets and types, and the `Wgpu.Buffer` class on [Types](Types.md#wgpubuffer).

## Make a buffer

A buffer made from a typed array keeps the array's element type and length. Its size in bytes is rounded up to a multiple of 4, since wgpu moves buffer data in whole 4-byte words:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $buffer = New-WgpuBuffer ([float[]](1..1024))
PS> $buffer
Label          ElementType Length Size Adapter
-----          ----------- ------ ---- -------
1024 x Float32 Float32       1024 4096 NVIDIA GeForce RTX 5070

PS> New-WgpuBuffer -Length 16 -ElementType UInt32
Label       ElementType Length Size Adapter
-----       ----------- ------ ---- -------
16 x UInt32 UInt32          16   64 NVIDIA GeForce RTX 5070
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu = [Wgpu.Device]::GetDefault()
PS> $buffer = $gpu.Upload([float[]](1..1024))
PS> $buffer
Label          ElementType Length Size Adapter
-----          ----------- ------ ---- -------
1024 x Float32 Float32       1024 4096 NVIDIA GeForce RTX 5070

PS> $gpu.Zeros(16, 'UInt32')
Label       ElementType Length Size Adapter
-----       ----------- ------ ---- -------
16 x UInt32 UInt32          16   64 NVIDIA GeForce RTX 5070
```
{{< /tab >}}
{{< /tabs >}}

Three bytes take one 4-byte word:

```powershell
PS> New-WgpuBuffer ([byte[]](1, 2, 3)) | Select-Object ElementType, Length, Size
ElementType Length Size
----------- ------ ----
      UInt8      3    4
```

`New-WgpuBuffer` also takes numbers from the pipeline. A list of numbers with no element type of its own becomes Float32, unless `-ElementType` names another:

```powershell
PS> (1..5 | New-WgpuBuffer).ElementType
Float32

PS> 1..5 | New-WgpuBuffer -ElementType Int32 | Read-WgpuBuffer
1
2
3
4
5
```

Data that is not numbers is refused, as is an empty array:

```powershell
PS> New-WgpuBuffer 'not numbers'
New-WgpuBuffer: a string is not numeric data; give an array of numbers

PS> New-WgpuBuffer ([float[]]@())
New-WgpuBuffer: a buffer needs at least one element
```

## Choose an element type

A buffer holds one of eleven element types:

```powershell
PS> [enum]::GetNames([Wgpu.ElementType]) -join ', '
Float32, Float64, Float16, Int8, UInt8, Int16, UInt16, Int32, UInt32, Int64, UInt64

PS> (New-WgpuBuffer ([uint16[]](1, 2))).ElementType
UInt16

PS> (New-WgpuBuffer @(1, 2, 3) -ElementType Int64).Size
24
```

WGSL, the shading language, has no 8-bit or 16-bit integers, so a shader reads a buffer of them as packed 32-bit words, and the cmdlets that write a shader for you refuse them. A shader that reads Float16 needs the ShaderF16 feature, one that reads Float64 needs ShaderF64, and one that reads 64-bit integers needs ShaderInt64.

## Convert values to another element type

An array of a .NET numeric type, such as `[double[]]`, given another element type is converted value by value: a float is truncated toward zero and saturates at the ends of an integer type, and an integer wraps into a narrower one. Float16 keeps half precision in two bytes an element.

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> New-WgpuBuffer ([double[]](2.9, -1, 1000)) -ElementType UInt8 | Read-WgpuBuffer
2
0
255

PS> New-WgpuBuffer ([int[]](300, -1)) -ElementType UInt8 | Read-WgpuBuffer
44
255

PS> New-WgpuBuffer ([float[]](0.5, 1.5, -2)) -ElementType Float16 | Select-Object ElementType, Length, Size
ElementType Length Size
----------- ------ ----
    Float16      3    8
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu.Upload([double[]](2.9, -1, 1000), 'UInt8').Read()
2
0
255

PS> $gpu.Upload([int[]](300, -1), 'UInt8').Read()
44
255

PS> $gpu.Upload([float[]](0.5, 1.5, -2), 'Float16') | Select-Object ElementType, Length, Size
ElementType Length Size
----------- ------ ----
    Float16      3    8
```
{{< /tab >}}
{{< /tabs >}}

Numbers with no .NET array type of their own, as `@(1, 2.5)`, numbers piped in or one number, are converted as PowerShell converts them: an integer type takes each rounded half to even, as `[int]2.5` is 2, exactly up to 64 bits, and refuses one outside its range as `WgpuOutOfRange`:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> New-WgpuBuffer @(1.5, 2.5, -1.5) -ElementType Int32 | Read-WgpuBuffer
2
2
-2

PS> New-WgpuBuffer @(255, 300) -ElementType UInt8
New-WgpuBuffer: 300 does not fit in UInt8, which holds 0 to 255
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu.Upload(@(1.5, 2.5, -1.5), 'Int32').Read()
2
2
-2

PS> $gpu.Upload(@(255, 300), 'UInt8')
MethodInvocationException: Exception calling "Upload" with "2" argument(s): "[WgpuOutOfRange] 300 does not fit in UInt8, which holds 0 to 255"
```
{{< /tab >}}
{{< /tabs >}}

## Choose what a buffer may be used for

A buffer is made with every usage but mapping when none is named, so it can be bound to a shader as storage or as a uniform, read by a draw as vertices or indices, hold the arguments of an indirect dispatch or draw, take the results of queries, and be copied to and from. Naming fewer usages makes a buffer that refuses the others; one made without CopyDst cannot be written once it is made:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (New-WgpuBuffer -Length 4).Usage -join ', '
CopySrc, CopyDst, Index, Vertex, Uniform, Storage, Indirect, QueryResolve

PS> $sealed = New-WgpuBuffer ([float[]](1, 2)) -Usage Storage, CopySrc
PS> $sealed.Usage -join ', '
CopySrc, Storage

PS> Write-WgpuBuffer $sealed ([float[]](3, 4))
Write-WgpuBuffer: the buffer cannot be written: Validation Error

Caused by:
  In Queue::write_buffer
    Usage flags BufferUsages(COPY_SRC | STORAGE) of Buffer with '2 x Float32' label do not contain required usage flags BufferUsages(COPY_DST)
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $gpu.CreateBuffer(4, 'Float32', $null, $null).Usage -join ', '
CopySrc, CopyDst, Index, Vertex, Uniform, Storage, Indirect, QueryResolve

PS> $sealed = $gpu.CreateBuffer(2, 'Float32', @('Storage', 'CopySrc'), $null)
PS> $sealed.Usage -join ', '
CopySrc, Storage

PS> $sealed.Write([float[]](3, 4))
MethodInvocationException: Exception calling "Write" with "1" argument(s): "[WgpuWrite] the buffer cannot be written: Validation Error

Caused by:
  In Queue::write_buffer
    Usage flags BufferUsages(COPY_SRC | STORAGE) of Buffer with '2 x Float32' label do not contain required usage flags BufferUsages(COPY_DST)
"
```
{{< /tab >}}
{{< /tabs >}}

## Write elements

A write overwrites elements from an element offset, 0 when none is given, and leaves the rest as they were. Values of another type are converted to the buffer's:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $six = New-WgpuBuffer ([float[]](1..6))
PS> Write-WgpuBuffer $six ([float[]](10, 20)) -Offset 2
PS> Write-WgpuBuffer $six ([int[]](8, 9)) -Offset 4
PS> Read-WgpuBuffer $six
1
2
10
20
8
9
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $six = $gpu.Upload([float[]](1..6))
PS> $six.Write([float[]](10, 20), 2)
PS> $six.Write([int[]](8, 9), 4)
PS> $six.Read()
1
2
10
20
8
9
```
{{< /tab >}}
{{< /tabs >}}

A write that starts or ends inside a 4-byte word reads that word back first, so the bytes around it keep their values:

```powershell
PS> $bytes = New-WgpuBuffer ([byte[]](1..8))
PS> Write-WgpuBuffer $bytes ([byte[]](99)) -Offset 2
PS> (Read-WgpuBuffer $bytes) -join ','
1,2,99,4,5,6,7,8

PS> Write-WgpuBuffer $six ([float[]](1, 2, 3)) -Offset 5
Write-WgpuBuffer: elements 5 to 8 run past the end of a buffer of 6 elements
```

## Read a buffer back

A buffer comes back as one array of its element type, or a range of it from an element offset:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $ten = New-WgpuBuffer ([float[]](1..10))
PS> (Read-WgpuBuffer $ten).GetType().Name
Single[]

PS> Read-WgpuBuffer $ten -Offset 3 -Count 2
4
5

PS> Read-WgpuBuffer $ten -Offset 8 -Count 5
Read-WgpuBuffer: elements 8 to 13 run past the end of a buffer of 10 elements
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $ten = $gpu.Upload([float[]](1..10))
PS> $ten.Read().GetType().Name
Single[]

PS> $ten.ReadRange(3, 2)
4
5

PS> $ten.ReadRange(8, 5)
MethodInvocationException: Exception calling "ReadRange" with "2" argument(s): "[WgpuRange] elements 8 to 13 run past the end of a buffer of 10 elements"
```
{{< /tab >}}
{{< /tabs >}}

The same bytes read as another element type: the float 1 is the bits 0x3F800000.

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> Read-WgpuBuffer $ten -Count 1 -ElementType UInt32
1065353216
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $ten.Read('UInt32')[0]
1065353216
```
{{< /tab >}}
{{< /tabs >}}

One array is one object on the pipeline, and `-Enumerate` writes each element instead. `-AsMemory`, and the `ReadMemory` method, hand .NET a `Memory<T>` over the values without a second copy; Windows PowerShell gets an array.

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> (Read-WgpuBuffer $ten | Measure-Object).Count
1

PS> (Read-WgpuBuffer $ten -Enumerate | Measure-Object -Sum).Sum
55

PS> (Read-WgpuBuffer $ten -AsMemory).Length
10
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> ($ten.Read() | Measure-Object -Sum).Sum
55

PS> $ten.ReadMemory().Length
10
```
{{< /tab >}}
{{< /tabs >}}

`-Destination` copies the elements into an array the script already holds instead, from its index 0, and writes nothing unless `-PassThru` is given; the `ReadInto` method does the same with a whole buffer, and a slice's with its range. The array is of the type the read gives, `float[]` for Float16, and holds at least the elements read, and the elements past them keep their values. A loop that reads a buffer again and again can fill one array this way, which [How Data Moves](How-Data-Moves.md#what-that-costs) times:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $values = [float[]]::new(12)
PS> Read-WgpuBuffer $ten -Destination $values
PS> $values -join ','
1,2,3,4,5,6,7,8,9,10,0,0

PS> (Read-WgpuBuffer $ten -Offset 8 -Count 2 -Destination $values -PassThru) -join ','
9,10,3,4,5,6,7,8,9,10,0,0

PS> Read-WgpuBuffer $ten -Destination ([double[]]::new(10))
Read-WgpuBuffer: a read of Float32 elements fills a System.Single[], and the destination is a System.Double[]
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $values = [float[]]::new(12)
PS> $ten.ReadInto($values)
PS> $values -join ','
1,2,3,4,5,6,7,8,9,10,0,0

PS> $ten.ReadInto([float[]]::new(4))
MethodInvocationException: Exception calling "ReadInto" with "1" argument(s): "[WgpuDestination] the read gives 10 elements, and the destination holds 4"
```
{{< /tab >}}
{{< /tabs >}}

## Copy between buffers

A copy runs on the GPU, from an element offset of one buffer to an element offset of another, both on one device. It moves bytes, so between element types it reinterprets them:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $a = New-WgpuBuffer ([int[]](1..8))
PS> $b = New-WgpuBuffer -Length 8 -ElementType Int32
PS> Copy-WgpuBuffer $a $b -SourceOffset 4 -Count 4
PS> Copy-WgpuBuffer $a $b -SourceOffset 0 -DestinationOffset 6 -Count 2
PS> (Read-WgpuBuffer $b) -join ','
5,6,7,8,0,0,1,2

PS> $words = New-WgpuBuffer -Length 2 -ElementType UInt32
PS> Copy-WgpuBuffer (New-WgpuBuffer ([float[]](1, 2))) $words
PS> (Read-WgpuBuffer $words) -join ','
1065353216,1073741824
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $a = $gpu.Upload([int[]](1..8))
PS> $b = $gpu.Zeros(8, 'Int32')
PS> $a.CopyTo($b, 4, 0, 4)
PS> $a.CopyTo($b, 0, 6, 2)
PS> $b.Read() -join ','
5,6,7,8,0,0,1,2

PS> $words = $gpu.Zeros(2, 'UInt32')
PS> $gpu.Upload([float[]](1, 2)).CopyTo($words, 0, 0, 2)
PS> $words.Read() -join ','
1065353216,1073741824
```
{{< /tab >}}
{{< /tabs >}}

wgpu copies whole 4-byte words, so a copy of three bytes is refused:

```powershell
PS> Copy-WgpuBuffer $bytes (New-WgpuBuffer -Length 8 -ElementType UInt8) -Count 3
Copy-WgpuBuffer: a buffer copy moves whole 4-byte words; the size, 3 bytes, is not a multiple of 4
```

## Fill or clear a buffer

A fill sets every element to one number, converted to the buffer's element type as each number with no array type is, and a clear sets every byte to zero:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $four = New-WgpuBuffer -Length 4 -ElementType Int16
PS> Clear-WgpuBuffer $four -Value -7
PS> (Read-WgpuBuffer $four) -join ','
-7,-7,-7,-7

PS> Clear-WgpuBuffer $four -Value 2.5
PS> (Read-WgpuBuffer $four) -join ','
2,2,2,2

PS> Clear-WgpuBuffer $four -Value 70000
Clear-WgpuBuffer: 70000 does not fit in Int16, which holds -32768 to 32767

PS> $four | Clear-WgpuBuffer
PS> (Read-WgpuBuffer $four) -join ','
0,0,0,0
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $four = $gpu.Zeros(4, 'Int16')
PS> $four.Fill(-7)
PS> $four.Read() -join ','
-7,-7,-7,-7

PS> $four.Fill(2.5)
PS> $four.Read() -join ','
2,2,2,2

PS> $four.Fill(70000)
MethodInvocationException: Exception calling "Fill" with "1" argument(s): "[WgpuOutOfRange] 70000 does not fit in Int16, which holds -32768 to 32767"

PS> $four.Clear()
PS> $four.Read() -join ','
0,0,0,0
```
{{< /tab >}}
{{< /tabs >}}

`Clear-WgpuBuffer` takes `-WhatIf` and `-Confirm`.

## Bind part of a buffer

A slice is a range of a buffer's elements, bound in place of the whole buffer wherever a buffer is bound. A binding's offset in bytes must be a multiple of the device's `MinStorageBufferOffsetAlignment`, or of `MinUniformBufferOffsetAlignment` for a uniform; WebGPU allows those limits to be as large as 256 bytes, which is 64 floats.

```powershell
PS> $double = '@group(0) @binding(0) var<storage, read_write> v: array<f32>;
>> @compute @workgroup_size(64)
>> fn main(@builtin(global_invocation_id) id: vec3<u32>) {
>>     if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
>> }'
PS> (Get-WgpuDevice).Limits.MinStorageBufferOffsetAlignment
32

PS> $big = New-WgpuBuffer ([float[]](1..128))
PS> $tail = $big.Slice(64, 64)
PS> $tail
Label         Start Length ElementType
-----         ----- ------ -----------
128 x Float32    64     64 Float32

PS> Invoke-WgpuCompute $double $tail
PS> (Read-WgpuBuffer $big)[62..66] -join ','
63,64,130,132,134

PS> $tail.Read()[0..2] -join ','
130,132,134

PS> $kept = [float[]]::new(64)
PS> $tail.ReadInto($kept)
PS> $kept[0..2] -join ','
130,132,134
```

## See a buffer as another element type

`Reinterpret` gives the same GPU memory as elements of another type, with nothing copied:

```powershell
PS> $pair = New-WgpuBuffer ([float[]](1, 2))
PS> $pair.Reinterpret('UInt32').Read() -join ','
1065353216,1073741824
```

## Free a buffer

A buffer's GPU memory is freed when .NET collects the object, or at once with `Remove-WgpuBuffer` or the `Destroy` method. The object stays, and every later use of it fails:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $spent = New-WgpuBuffer ([float[]](1, 2)) -Label spent
PS> $spent | Remove-WgpuBuffer
PS> Read-WgpuBuffer $spent
Read-WgpuBuffer: the buffer cannot be read back: Validation Error

Caused by:
  In Queue::submit
    Buffer with 'spent' label has been destroyed
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $spent = $gpu.CreateBuffer(2, 'Float32', $null, 'spent')
PS> $spent.Destroy()
PS> $spent.Read()
MethodInvocationException: Exception calling "Read" with "0" argument(s): "[WgpuRead] the buffer cannot be read back: Validation Error

Caused by:
  In Queue::submit
    Buffer with 'spent' label has been destroyed
"
```
{{< /tab >}}
{{< /tabs >}}

`Remove-WgpuBuffer` takes `-WhatIf` and `-Confirm`, and removing a buffer a second time does nothing more. Every method that reads, writes, fills, clears or copies also has an `Async` form, which [Tasks](Tasks.md) shows.
