---
title: Errors
weight: 90
---

Every error the module raises carries an id that says what went wrong, such as `WgpuRange` for a range past the end of a buffer. A cmdlet that fails writes an error record and goes on with its next input, as PowerShell's own cmdlets do; the record's `FullyQualifiedErrorId` begins with the id. A method that fails throws an exception whose message begins with the id in brackets. Every id, with its category and its messages, is in the [Error Reference](Error-Reference.md).

## Carry on, or stop

A cmdlet's error leaves the cmdlet working through the rest of its input. A method's error ends the statement that called it, and the script goes on with the next:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $done = (New-WgpuCommandEncoder 'done').Finish()
PS> Submit-WgpuCommand $done
PS> $fresh = (New-WgpuCommandEncoder 'fresh').Finish()
PS> $done, $fresh | Submit-WgpuCommand
Submit-WgpuCommand: the command buffer of encoder 'done' was already submitted; record the work again

PS> $fresh.IsSubmitted
True
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $buffer = New-WgpuBuffer ([float[]](1, 2, 3))
PS> $buffer.ReadRange(5, 1); 'the next statement runs'
MethodInvocationException: Exception calling "ReadRange" with "2" argument(s): "[WgpuRange] element 5 is past the end of a buffer of 3 elements"
the next statement runs
```
{{< /tab >}}
{{< /tabs >}}

`-ErrorAction Stop` on a cmdlet, or `$ErrorActionPreference = 'Stop'` for a whole script, makes a cmdlet's error stop the script too. Inside `try`, either kind of error goes to `catch`:

```powershell
PS> try { Open-WgpuDevice -Adapter nosuchadapter -ErrorAction Stop; 'not reached' } catch { "caught: $($_.Exception.Message)" }
caught: no adapter's name contains nosuchadapter; the adapters are: NVIDIA GeForce RTX 5070 (Vulkan); AMD Radeon(TM) Graphics (Vulkan); NVIDIA GeForce RTX 5070 (Dx12); AMD Radeon(TM) Graphics (Dx12); Microsoft Basic Render Driver (Dx12)
```

## Read an error's id

A cmdlet's record carries the id at the front of its `FullyQualifiedErrorId`, followed by the cmdlet's class, and its category in `CategoryInfo`. A method's exception reaches PowerShell as a `MethodInvocationException`, whose `InnerException` is the module's own, with the id in brackets at the front of its message:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> Open-WgpuDevice -Adapter nosuchadapter -ErrorAction SilentlyContinue -ErrorVariable failed
PS> $failed[0].FullyQualifiedErrorId
WgpuNoAdapter,Pwrs.Modules.Wgpu.OpenWgpuDeviceCommand

PS> $failed[0].CategoryInfo.Category
ObjectNotFound
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $buffer = New-WgpuBuffer ([float[]](1, 2, 3))
PS> try { $buffer.ReadRange(5, 1) } catch { $_.Exception.InnerException.Message }
[WgpuRange] element 5 is past the end of a buffer of 3 elements
```
{{< /tab >}}
{{< /tabs >}}

This function reads the id from either, and gives the id of an error PowerShell raised itself, such as a parameter it could not bind, the same way:

```powershell
PS> function Get-WgpuErrorId($record) {
>>     if ($record.Exception.GetBaseException().Message -match '^\[(\w+)\]') { $Matches[1] }
>>     else { ($record.FullyQualifiedErrorId -split ',')[0] }
>> }
PS> $buffer = New-WgpuBuffer ([float[]](1, 2, 3))
PS> try { $buffer.ReadRange(5, 1) } catch { Get-WgpuErrorId $_ }
WgpuRange

PS> try { Open-WgpuDevice -Adapter nosuchadapter -ErrorAction Stop } catch { Get-WgpuErrorId $_ }
WgpuNoAdapter

PS> try { Open-WgpuDevice -Backend Nope } catch { Get-WgpuErrorId $_ }
CannotConvertArgumentNoMessage
```

The exception behind the module's errors is a `Pwrs.PwrsException`. Windows PowerShell can catch it by type, with `catch [Pwrs.PwrsException]`, but PowerShell 7 cannot find that type by name, so a script that runs in both reads the id.

## Take another way

With the id in hand, a script can take another way for the errors it expects and pass the rest on. Here an adapter that is not there falls back to the default device, and any other error is thrown again:

```powershell
PS> $gpu = try { Open-WgpuDevice -Adapter nosuchadapter -ErrorAction Stop } catch {
>>     if ((Get-WgpuErrorId $_) -ne 'WgpuNoAdapter') { throw }
>>     Get-WgpuDevice
>> }
PS> $gpu.Adapter
NVIDIA GeForce RTX 5070
```

## Tell the errors PowerShell and PWRS raise

Some arguments never reach a cmdlet: PowerShell's parameter binder checks them first, and its records carry ids of its own. `CannotConvertArgumentNoMessage` is a value that does not convert to the parameter's type, `AmbiguousParameterSet` parameters from different parameter sets together, and `NamedParameterNotFound` a parameter the cmdlet does not have; the command does not run. `InputObjectNotBound` is an object on the pipeline no parameter takes, and the command goes on with the next:

```powershell
PS> try { Open-WgpuDevice -Backend Nope } catch { $_.FullyQualifiedErrorId }
CannotConvertArgumentNoMessage,Pwrs.Modules.Wgpu.OpenWgpuDeviceCommand

PS> try { New-WgpuBuffer -Data 1 -Length 3 } catch { $_.FullyQualifiedErrorId }
AmbiguousParameterSet,Pwrs.Modules.Wgpu.NewWgpuBufferCommand

PS> 'text' | Read-WgpuBuffer -ErrorAction SilentlyContinue -ErrorVariable failed
PS> $failed[0].FullyQualifiedErrorId
InputObjectNotBound,Pwrs.Modules.Wgpu.ReadWgpuBufferCommand
```

PWRS, the runtime the module is built on, converts what a cmdlet is given into Rust values, and a value inside data that does not convert to the number type it is read as raises its `PwrsConversionError`, of the category InvalidType:

```powershell
PS> New-WgpuBuffer @('a', 'b') -ErrorAction SilentlyContinue -ErrorVariable failed
PS> $failed[0].FullyQualifiedErrorId
PwrsConversionError,Pwrs.Modules.Wgpu.NewWgpuBufferCommand

PS> $failed[0].CategoryInfo.Category
InvalidType
```

## Read why a task failed

A method that returns a task refuses at the call what it can check before the work starts, and an error after that ends the task as Faulted, its `Exception` holding the module's error. Here the device is destroyed before a read starts; [Tasks and threads](Tasks.md) shows the ways to wait for a task that may fail:

```powershell
PS> $doomed = [Wgpu.Device]::new($null, $null)
PS> $data = New-WgpuBuffer ([float[]](1, 2, 3)) -Device $doomed
PS> $doomed.Destroy()
PS> $task = $data.ReadAsync($null)
PS> [Threading.Tasks.Task]::WhenAny($task).Wait()
PS> $task.Status
Faulted

PS> $task.Exception.InnerException.Message
[WgpuDeviceLost] the device was lost: Destroyed
```

## Replace a lost device

A device is lost when it is closed or destroyed, or when its driver loses it. A device `Open-WgpuDevice` opened raises the `Wgpu.DeviceLost` engine event, whose `MessageData` names the device, and every later call that uses the device fails with `WgpuDeviceLost`:

```powershell
PS> $brief = Open-WgpuDevice -Label brief
PS> $kept = New-WgpuBuffer ([float[]](1, 2, 3)) -Device $brief
PS> $brief | Close-WgpuDevice
PS> (Wait-Event -SourceIdentifier Wgpu.DeviceLost -Timeout 10).MessageData
Device Kind      Message
------ ----      -------
brief  Destroyed

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

PS> (New-WgpuBuffer ([float[]](1, 2, 3))).Read() -join ','
1,2,3
```

## Hear about errors outside any call

wgpu can report an error that no call of the module was waiting for. The device then raises the `Wgpu.UncapturedError` engine event, holds the error in `Status().PendingErrors`, and its next call fails with `WgpuUncaptured`, which carries it. An action registered for the event runs for each one:

```powershell
PS> $null = Register-EngineEvent -SourceIdentifier Wgpu.UncapturedError -Action {
>>     Write-Warning "$($Event.MessageData.Device): $($Event.MessageData.Kind): $($Event.MessageData.Message)"
>> }
```

A device opened by `[Wgpu.Device]::new` raises no engine events, since no runspace opened it; its `Status()` still reports what it would have raised.
