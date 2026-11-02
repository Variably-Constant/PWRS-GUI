---
title: How To Debug GPU Work
weight: 20
---

Track down GPU work that fails, or that holds more memory than it should: name everything so wgpu's messages say which object is at fault, find where a mistake in a recording is reported, mark a recording and capture it for a graphics debugger, check a device for errors raised outside any call, watch the memory it holds, and turn on the backend's own checks. [Errors](Errors.md) shows how a script catches and tells apart the errors this finds.

## Name what you make

`-Label` names a device, buffer, texture, pipeline, query set or encoder, and wgpu's messages and graphics debuggers show the name. A mistake in a recording is reported when the encoder finishes, so the names say where it was. This pass dispatches without setting a pipeline:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $encoder = New-WgpuCommandEncoder -Label 'blur frame'
PS> $pass = $encoder.BeginComputePass('horizontal blur')
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> $encoder.Finish()
MethodInvocationException: Exception calling "Finish" with "0" argument(s): "[WgpuFinish] the recorded commands are invalid: Validation Error

Caused by:
  In a CommandEncoder, label = 'blur frame'
    In a dispatch command, indirect:false
      Compute pipeline must be set
"
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $encoder = [Wgpu.Device]::GetDefault().CreateCommandEncoder('blur frame')
PS> $pass = $encoder.BeginComputePass('horizontal blur')
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> $encoder.Finish()
MethodInvocationException: Exception calling "Finish" with "0" argument(s): "[WgpuFinish] the recorded commands are invalid: Validation Error

Caused by:
  In a CommandEncoder, label = 'blur frame'
    In a dispatch command, indirect:false
      Compute pipeline must be set
"
```
{{< /tab >}}
{{< /tabs >}}

A command is refused at the call itself only by a pass that has ended or an encoder that has finished; [Find where a mistake is reported](Recording.md#find-where-a-mistake-is-reported) has both.

## Mark a recording for a graphics debugger

A graphics debugger, such as RenderDoc, PIX or Xcode, shows the commands of a captured frame. Debug groups gather commands under a name, and markers name a point between them, on an encoder or in a pass; they change nothing else:

```powershell
PS> $values = New-WgpuBuffer ([float[]](1, 2, 3, 4)) -Label values
PS> $marked = New-WgpuCommandEncoder -Label 'reset values'
PS> $marked.PushDebugGroup('clear and refill')
PS> $marked.ClearBuffer($values)
PS> $marked.InsertDebugMarker('cleared')
PS> $marked.CopyBufferToBuffer((New-WgpuBuffer ([float[]](9, 9))), 0, $values, 0)
PS> $marked.PopDebugGroup()
PS> Submit-WgpuCommand $marked.Finish()
PS> (Read-WgpuBuffer $values) -join ','
9,9,0,0
```

`StartCapture` and `StopCapture` capture the work between them in a RenderDoc or Xcode attached to the process, as when PowerShell is started from RenderDoc; with neither attached they do nothing:

```powershell
PS> $gpu = Get-WgpuDevice
PS> $gpu.StartCapture()
PS> Invoke-WgpuMap 'x * 2.0' ([float[]](1, 2)) -Device $gpu
2
4

PS> $gpu.StopCapture()
```

## Check a device for errors

wgpu can report an error that no call was waiting for. The device then holds it in its status, raises the `Wgpu.UncapturedError` engine event for a device `Open-WgpuDevice` opened, and fails its next call with `WgpuUncaptured`. The status also says whether the device was lost, and why:

```powershell
PS> $gpu.Status()
IsLost LostReason PendingErrors IsDefault
------ ---------- ------------- ---------
False             {}            True
```

An action registered for the event hears of each error as it comes:

```powershell
PS> $null = Register-EngineEvent -SourceIdentifier Wgpu.UncapturedError -Action {
>>     Write-Warning "$($Event.MessageData.Device): $($Event.MessageData.Message)"
>> }
```

## Watch the memory a device holds

`Counters` counts the objects a device holds and the bytes they take, as the backend counts them, so a count that keeps growing shows objects kept by mistake. A buffer's memory is freed when nothing holds the buffer any longer; `Remove-WgpuBuffer` frees it at once. On a device of its own, four buffers of 4 MB each come and go:

```powershell
PS> $watched = Open-WgpuDevice -Label watched
PS> $before = $watched.Counters()
PS> $big = 1..4 | ForEach-Object { New-WgpuBuffer -Length 1048576 -ElementType Float32 -Device $watched -Label "big $_" }
PS> $during = $watched.Counters()
PS> $during.Buffers - $before.Buffers
4

PS> [math]::Round(($during.BufferMemory - $before.BufferMemory) / 1MB)
16

PS> $big | Remove-WgpuBuffer
PS> $watched.Counters().Buffers - $before.Buffers
0
```

`AllocatorReport` gives the backend's own report of the memory it has allocated and reserved, where the backend keeps one:

```powershell
PS> $watched.AllocatorReport() | Select-Object TotalAllocatedBytes, TotalReservedBytes
TotalAllocatedBytes TotalReservedBytes
------------------- ------------------
             524784          201326592
```

## Turn on the backend's checks

`-Validation` turns on the backend's validation layers, which check far more than wgpu does and report misuse the driver would let through: the Vulkan SDK's layers, or Direct3D 12's debug layer, which comes with Windows' optional Graphics Tools. `-GpuBasedValidation` adds checks that run on the GPU, which are slower. Where the layers are not installed, as on the machine these pages ran on, the device opens without them:

```powershell
PS> (Open-WgpuDevice -Validation -GpuBasedValidation -Label checked).Adapter
NVIDIA GeForce RTX 5070
```

`-TracePath` writes a trace of every call made on the device into a folder, which wgpu's trace player replays to reproduce a problem elsewhere:

```powershell
PS> $null = New-Item -ItemType Directory -Path ./trace
PS> $traced = Open-WgpuDevice -TracePath ./trace -Label traced
PS> Invoke-WgpuMap 'x + 1.0' ([float[]](1, 2)) -Device $traced
2
3

PS> Get-ChildItem ./trace | Select-Object -ExpandProperty Name
data1.bin
data2.wgsl
trace.ron
```

A shader's own mistakes are found before any of this, with no device: [How To Check And Translate Shaders](How-To-Check-And-Translate-Shaders.md).
