---
title: How To Handle Errors
weight: 19
---

Make a script that copes when GPU work fails: let a bad input pass and carry on with the rest, stop where the work cannot go on, tell one error from another by its id, take another way for an error it expects, replace a device that is lost, and hear about an error no call was waiting for. The [Error Reference](Error-Reference.md) lists every id with its category and messages.

## Carry on, or stop

A cmdlet that fails writes an error record and goes on with its next input, as PowerShell's own cmdlets do. Here the first command buffer was submitted already, and the second still runs:

```powershell
PS> $done = (New-WgpuCommandEncoder 'done').Finish()
PS> Submit-WgpuCommand $done
PS> $fresh = (New-WgpuCommandEncoder 'fresh').Finish()
PS> $done, $fresh | Submit-WgpuCommand
Submit-WgpuCommand: the command buffer of encoder 'done' was already submitted; record the work again

PS> $fresh.IsSubmitted
True
```

A method that fails throws, which ends the statement that called it, and the script goes on with the next:

```powershell
PS> $buffer = New-WgpuBuffer -Data ([float[]](1, 2, 3))
PS> $buffer.ReadRange(5, 1); 'the next statement runs'
MethodInvocationException: Exception calling "ReadRange" with "2" argument(s): "[WgpuRange] element 5 is past the end of a buffer of 3 elements"
the next statement runs
```

`-ErrorAction Stop` on a cmdlet, or `$ErrorActionPreference = 'Stop'` for a whole script, makes a cmdlet's error stop the script as well. Inside `try`, either kind of error goes to `catch` instead:

```powershell
PS> try { Open-WgpuDevice -Adapter nosuchadapter -ErrorAction Stop; 'not reached' } catch { "caught: $($_.Exception.Message)" }
caught: no adapter's name contains nosuchadapter; the adapters are: NVIDIA GeForce RTX 5070 (Vulkan); AMD Radeon(TM) Graphics (Vulkan); NVIDIA GeForce RTX 5070 (Dx12); AMD Radeon(TM) Graphics (Dx12); Microsoft Basic Render Driver (Dx12)
```

## Tell one error from another

Every error the module raises carries an id that says what went wrong. A cmdlet's record leads its FullyQualifiedErrorId with it, and a method's exception leads its message with it, in brackets. This function reads it from either, and gives the id of an error PowerShell raised itself, such as a parameter it could not bind, the same way:

```powershell
PS> function Get-WgpuErrorId($record) {
>>     if ($record.Exception.GetBaseException().Message -match '^\[(\w+)\]') { $Matches[1] }
>>     else { ($record.FullyQualifiedErrorId -split ',')[0] }
>> }
PS> try { $buffer.ReadRange(5, 1) } catch { Get-WgpuErrorId $_ }
WgpuRange

PS> try { Open-WgpuDevice -Adapter nosuchadapter -ErrorAction Stop } catch { Get-WgpuErrorId $_ }
WgpuNoAdapter

PS> try { Open-WgpuDevice -Backend Nope } catch { Get-WgpuErrorId $_ }
CannotConvertArgumentNoMessage
```

The exception behind the module's own errors is a `Pwrs.PwrsException`. Windows PowerShell can catch it by type, with `catch [Pwrs.PwrsException]`, but PowerShell 7 cannot find that type by name, so a script that runs in both reads the id.

## Try something else

With the id in hand, a script can take another way for the errors it expects and pass the rest on. Here an adapter that is not there falls back to the default device, and any other error is thrown again:

```powershell
PS> $gpu = try { Open-WgpuDevice -Adapter nosuchadapter -ErrorAction Stop } catch {
>>     if ((Get-WgpuErrorId $_) -ne 'WgpuNoAdapter') { throw }
>>     Get-WgpuDevice
>> }
PS> $gpu.Label
Wgpu device 1
```

## A device that is lost

A device is lost when it is closed or destroyed, or when the driver loses it. It raises the `Wgpu.DeviceLost` engine event, whose MessageData is a [Wgpu.DeviceEvent](Types.md#wgpudeviceevent) naming the device, and every later call that uses it fails with WgpuDeviceLost:

```powershell
PS> $gpu = Open-WgpuDevice -Label 'short-lived'
PS> $kept = New-WgpuBuffer -Data ([float[]](1, 2, 3)) -Device $gpu
PS> $gpu | Close-WgpuDevice
PS> (Wait-Event -SourceIdentifier Wgpu.DeviceLost -Timeout 10).MessageData
Device      Kind      Message
------      ----      -------
short-lived Destroyed

PS> Remove-Event -SourceIdentifier Wgpu.DeviceLost
PS> Read-WgpuBuffer $kept
Read-WgpuBuffer: the device was lost: Destroyed
```

A default device that is lost is replaced: the next cmdlet given no `-Device` opens a new one, or `Open-WgpuDevice -Default` opens the one wanted. Buffers, textures and pipelines belong to the device they were made on, so a script makes them again on the new one:

```powershell
PS> $first = Get-WgpuDevice
PS> $first | Close-WgpuDevice
PS> (Get-WgpuDevice).Label -eq $first.Label
False

PS> (New-WgpuBuffer -Data ([float[]](1, 2, 3))).Read() -join ','
1,2,3
```

## Errors outside any call

wgpu can report an error that no call of the module was waiting for. The device then raises the `Wgpu.UncapturedError` engine event, holds the error in [`Status().PendingErrors`](Types.md#wgpudevicestatus), and its next call fails with WgpuUncaptured, carrying it. An action registered for the event runs for each one:

```powershell
PS> $null = Register-EngineEvent -SourceIdentifier Wgpu.UncapturedError -Action {
>>     Write-Warning "$($Event.MessageData.Device): $($Event.MessageData.Kind): $($Event.MessageData.Message)"
>> }
```

A device opened by `[Wgpu.Device]::new` raises no engine events, since no runspace opened it; `Status()` still reports what it would have raised. For a task that fails, see [How To Wait Without Blocking](How-To-Wait-Without-Blocking.md).
