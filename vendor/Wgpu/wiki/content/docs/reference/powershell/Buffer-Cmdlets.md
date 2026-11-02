---
title: Buffers
weight: 20
---

The syntax, the parameters and the errors of `New-WgpuBuffer`, `Write-WgpuBuffer`, `Read-WgpuBuffer`, `Copy-WgpuBuffer`, `Clear-WgpuBuffer` and `Remove-WgpuBuffer`. The examples are on [Buffers](Buffers.md).

### New-WgpuBuffer

Creates a GPU buffer from an array of any numeric type, or of a length.

The buffer stays on the device until something reads it. Without -Type it takes the array's own element type (Float32 for an untyped array); with -Type the values are converted.

Alias: `New-GpuBuffer`

```powershell
New-WgpuBuffer [[-Data] <Object>] [-ElementType <ElementType>] [-Usage <BufferUsage[]>] [-Label <string>] [-Device <Device>] [<CommonParameters>]
New-WgpuBuffer -Length <ulong> [-ElementType <ElementType>] [-Usage <BufferUsage[]>] [-Label <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Data` | object | by value | The values: an array of any numeric type, a number, or numbers piped in. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to create it on; the default device when omitted. |
| `-ElementType` | [Wgpu.ElementType](Types.md#wgpuelementtype) |  | The element type; the array's own type, or Float32, when omitted. Alias: `-Type`. |
| `-Label` | string |  | The buffer's label. |
| `-Length` | ulong |  | The number of zeroed elements. |
| `-Usage` | [Wgpu.BufferUsage](Types.md#wgpubufferusage)[] |  | What the buffer may be used for; every usage but mapping when omitted. |

Writes [Wgpu.Buffer](Types.md#wgpubuffer).

#### Errors

`WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuEmpty`, `WgpuBufferTooLarge` and `WgpuBuffer`, and without `-Device` the errors of opening the default device, `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`; each described in the [Error Reference](Error-Reference.md). A value that does not convert to a number raises PWRS's `PwrsConversionError`, described there too.

### Write-WgpuBuffer

Overwrites elements of a buffer with new values.

Alias: `Write-GpuBuffer`

```powershell
Write-WgpuBuffer [-Buffer] <Buffer> [-Data] <Object> [-Offset <ulong>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Buffer` | [Wgpu.Buffer](Types.md#wgpubuffer) |  | The buffer to write. |
| `-Data` | object |  | The values, converted to the buffer's element type. |
| `-Offset` | ulong |  | The first element to overwrite; 0 when omitted. |

Writes nothing to the pipeline.

#### Errors

`WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuRange` and `WgpuWrite`; and for a write that starts or ends inside a 4-byte word, which reads that word back first, `WgpuRead`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuMap` and `WgpuStopped`. Each is described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a value that does not convert to a number.

### Read-WgpuBuffer

Copies a buffer back from the GPU and writes its elements as one array.

With -Destination it copies them into an array the caller holds instead, from its index 0, and writes nothing unless -PassThru is given.

Alias: `Read-GpuBuffer`

```powershell
Read-WgpuBuffer [-Buffer] <Buffer> [-Offset <ulong>] [-Count <ulong>] [-ElementType <ElementType>] [-AsMemory] [-Enumerate] [<CommonParameters>]
Read-WgpuBuffer [-Buffer] <Buffer> -Destination <Object> [-Offset <ulong>] [-Count <ulong>] [-ElementType <ElementType>] [-PassThru] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AsMemory` | switch |  | Write a Memory&lt;T&gt; over the values instead of an array (an array on Windows PowerShell). |
| `-Buffer` | [Wgpu.Buffer](Types.md#wgpubuffer) | by value | The buffer to read. |
| `-Count` | ulong |  | The number of elements to read; to the end when omitted. |
| `-Destination` | object |  | The array to copy the elements into, from its index 0: of the type the read gives, float[] for Float16, holding at least the elements read. The elements past them keep their values. |
| `-ElementType` | [Wgpu.ElementType](Types.md#wgpuelementtype) |  | Read the bytes as this element type instead of the buffer's own. Alias: `-Type`. |
| `-Enumerate` | switch |  | Write each element to the pipeline rather than one array. |
| `-Offset` | ulong |  | The first element to read; 0 when omitted. |
| `-PassThru` | switch |  | With -Destination, write the filled array as one object. |

Writes `array`.

#### Errors

`WgpuNoBuffer`, `WgpuRange`, `WgpuRead`, `WgpuSubmitThread`, `WgpuPoll`, `WgpuMap` and `WgpuStopped`; `WgpuBufferTooLarge` for more elements than a .NET array holds; and with `-Destination`, `WgpuDestination`. Each is described in the [Error Reference](Error-Reference.md).

### Copy-WgpuBuffer

Copies elements from one buffer to another on the GPU.

Offsets count elements of each buffer's own type; the copy moves the source's bytes, in whole 4-byte words.

Alias: `Copy-GpuBuffer`

```powershell
Copy-WgpuBuffer [-Source] <Buffer> [-Destination] <Buffer> [-SourceOffset <ulong>] [-DestinationOffset <ulong>] [-Count <ulong>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Count` | ulong |  | The number of source elements; to the end when omitted. |
| `-Destination` | [Wgpu.Buffer](Types.md#wgpubuffer) |  | The buffer to copy to. |
| `-DestinationOffset` | ulong |  | The first destination element; 0 when omitted. |
| `-Source` | [Wgpu.Buffer](Types.md#wgpubuffer) |  | The buffer to copy from. |
| `-SourceOffset` | ulong |  | The first source element; 0 when omitted. |

Writes nothing to the pipeline.

#### Errors

`WgpuNoBuffer`, `WgpuRange`, `WgpuOtherDevice`, `WgpuCopyAlignment`, `WgpuCopy`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll` and `WgpuStopped`, each described in the [Error Reference](Error-Reference.md).

### Clear-WgpuBuffer

Sets every element of a buffer to zero, or to -Value.

Alias: `Clear-GpuBuffer`

```powershell
Clear-WgpuBuffer [-Buffer] <Buffer> [-Value <Object>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Buffer` | [Wgpu.Buffer](Types.md#wgpubuffer) | by value | The buffer to clear. |
| `-Value` | object |  | The number every element takes, converted to the buffer's element type as each number of untyped data is; 0 when omitted. |

Writes nothing to the pipeline.

#### Errors

`WgpuNoBuffer`, `WgpuOutOfRange`, `WgpuClear`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuWrite`, `WgpuPoll` and `WgpuStopped`; and for a buffer whose bytes do not fill whole 4-byte words, whose last word is read back first, `WgpuRead` and `WgpuMap`. Each is described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a `-Value` that does not convert to a number.

### Remove-WgpuBuffer

Frees a buffer's GPU memory now, rather than when the object is collected.

Alias: `Remove-GpuBuffer`

```powershell
Remove-WgpuBuffer [-Buffer] <Buffer> [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Buffer` | [Wgpu.Buffer](Types.md#wgpubuffer) | by value | The buffer to free. |

Writes nothing to the pipeline.

#### Errors

`WgpuNoBuffer`, described in the [Error Reference](Error-Reference.md). A removed buffer's later use fails with the error of that use, such as `WgpuRead`.
