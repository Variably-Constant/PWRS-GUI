---
title: Measure-GuiLatency
weight: 11
---

Opens a window with one button and measures how long each click takes to reach the module, a handler script block, the pipeline and an engine-event action. Source: `crates/pwrs-gui/src/latency.rs`, `MeasureGuiLatency`, `Latency`, `LatencyRun`, `LatencyClick` and `Clock`; the clock is `now_ns` in `crates/pwrs-gui-proto/src/clock.rs`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and the times, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows, which reach the window as a person's clicks do; the times differ from run to run.

## Syntax

```powershell
Get-Command Measure-GuiLatency -Syntax
```

```text
Measure-GuiLatency [[-Count] <long>] [-NoWait] [<CommonParameters>]
```

## Description

The renderer stamps every event with the moment it took the input, on a clock every process on the machine reads alike: `QueryPerformanceCounter` on Windows and `CLOCK_MONOTONIC` elsewhere, in nanoseconds. `Measure-GuiLatency` reads the same clock at each place a click arrives, so each time is how long the click took to get there from the moment the renderer took it, across the process boundary when the renderer is the helper. It measures the build that is installed, on the machine it runs on.

Without `-NoWait` the command waits for clicks and writes one `PwrsGui.Latency` per click: when the module read it (`ToModuleUs`), when a handler script block run on the pipeline thread read the clock (`ToHandlerUs`), and when the command wrote the record, after the handler (`ToPipelineUs`). It ends after `-Count` clicks, closing the window, or when the window is closed, or on Ctrl+C.

With `-NoWait` it returns a `PwrsGui.LatencyRun` once the window has opened. A thread of the module reads the clicks and raises an engine event for each, and the action the command subscribed to it records how long the click took to reach the action (`ToEventUs`). An action runs when the engine is idle: at an interactive prompt, or while a script waits in `Wait-Event`. The run's `Samples()` gives the clicks recorded so far, and `Close()` closes the window.

The window is laid out as a dialog, inside a 12-point margin: "Click the button to time each click" as its main line, then "Clicks measured: n" and, with `-Count`, "of" the count, and the `Click me` button at its bottom right, the window's default, so Enter clicks it too.

## Parameters

### -Count

| | |
|---|---|
| Type | `System.Int64` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Validation | 1 to 100000 |
| Default | none: clicks are measured until the window is closed |
| Refused with | `ParameterArgumentValidationError` from the binder outside 1 to 100000 |

How many clicks to measure; the window closes after the last.

### -NoWait

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |

Returns a `PwrsGui.LatencyRun` once the window has opened, and each click raises an engine event whose action records when it ran.

## Inputs

None.

## Outputs

Without `-NoWait`, `PwrsGui.Latency`, one per click:

| Property | Type | What it gives |
|---|---|---|
| `Click` | `System.Int64` | the click's number, from 1 |
| `ToModuleUs` | `System.Double` | microseconds to the module reading the click |
| `ToHandlerUs` | `System.Nullable[System.Double]` | microseconds to a handler script block on the pipeline thread reading the clock; null with `-NoWait` |
| `ToPipelineUs` | `System.Nullable[System.Double]` | microseconds to the command writing the record, after the handler; null with `-NoWait` |
| `ToEventUs` | `System.Nullable[System.Double]` | microseconds to an engine-event action reading the clock; set with `-NoWait` only |

With `-NoWait`, `PwrsGui.LatencyRun`, one:

| Member | Type | Is |
|---|---|---|
| `Window` | `System.UInt64` | the window's number |
| `SourceIdentifier` | `System.String` | the engine event each click raises, which `Unregister-Event` takes to remove the action |
| `Samples()` | `PwrsGui.Latency[]` | the clicks recorded so far, in order, each with `ToModuleUs` and `ToEventUs` |
| `Close()` | | closes the window, which ends the measurement |
| `Record(click, atNs, moduleNs, eventNs)` | | what the subscribed action calls for each click |

`[PwrsGui.Clock]::Now()` reads the clock, in nanoseconds:

<!-- varies: a clock reading -->
```powershell
[PwrsGui.Clock]::Now()
```

```text
270367485841100
```

## Examples

The examples from the command's help.

Measure five clicks:

<!-- person: $w = Wait-Window 'PWRS-GUI latency'; 1..5 | ForEach-Object { Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click; Start-Sleep -Milliseconds 200 } -->
<!-- varies: times -->
```powershell
Measure-GuiLatency -Count 5
```

```text
Click ToModuleUs ToHandlerUs ToPipelineUs ToEventUs
----- ---------- ----------- ------------ ---------
1     7899.5     10634.6     12501
2     21.7       123.1       164.1
3     40.3       178.7       234.6
4     46.6       243.6       339.4
5     26.3       139.3       166.6
```

The records' type:

<!-- person: $w = Wait-Window 'PWRS-GUI latency'; 1..3 | ForEach-Object { Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click; Start-Sleep -Milliseconds 200 } -->
<!-- varies: times -->
```powershell
$records = Measure-GuiLatency -Count 3
$records | Get-Member -MemberType Properties
```

```text
   TypeName: PwrsGui.Latency

Name         MemberType Definition
----         ---------- ----------
Click        Property   long Click {get;set;}
ToEventUs    Property   System.Nullable[double] ToEventUs {get;set;}
ToHandlerUs  Property   System.Nullable[double] ToHandlerUs {get;set;}
ToModuleUs   Property   double ToModuleUs {get;set;}
ToPipelineUs Property   System.Nullable[double] ToPipelineUs {get;set;}
```

Measure clicks while the session goes on, recording each in an engine-event action:

<!-- person: $w = Wait-Window 'PWRS-GUI latency'; 1..3 | ForEach-Object { Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click; Start-Sleep -Milliseconds 200 } -->
<!-- varies: times and numbers -->
```powershell
$run = Measure-GuiLatency -NoWait
$run
Wait-Event -Timeout 3
$run.Samples() | Format-Table Click, ToModuleUs, ToEventUs
$run.Close()
Unregister-Event -SourceIdentifier $run.SourceIdentifier
```

```text
Window SourceIdentifier
------ ----------------
15     PwrsGui.Latency.pwrsgui36280_7f4806848bf36cd3.15


Click ToModuleUs ToEventUs
----- ---------- ---------
    1      38.50 145490.30
    2      19.10 164021.60
    3      25.30 160584.40
```

## Errors

A count outside 1 to 100000 is refused by the binder before the command runs:

```powershell
try { Measure-GuiLatency -Count 0 } catch { $_.FullyQualifiedErrorId }
```

```text
ParameterArgumentValidationError,Pwrs.Modules.PWRS_GUI.MeasureGuiLatencyCommand
```

A renderer that cannot start is `GuiStartFailed`, and one that goes away while the command waits is `GuiRendererGone`; both are terminating, and described in the [Error Reference](Error-Reference.md).
