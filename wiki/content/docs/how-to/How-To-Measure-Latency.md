---
title: How To Measure Latency
weight: 10
---

Measure, on your own machine and the installed build, how long a click takes to reach PowerShell: the module, a handler script block, the pipeline, and an engine-event action. Source: `crates/pwrs-gui/src/latency.rs`, and `now_ns` in `crates/pwrs-gui-proto/src/clock.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; the times differ from run to run and between the hosts. The clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, 100 milliseconds apart; they reach the window's command as a person's clicks do, through the renderer, which stamps them when it raises them.

## What is measured

The renderer stamps each click with the moment it took it, on a clock every process on the machine reads alike, and [Measure-GuiLatency](Measure-GuiLatency.md) reads the same clock where the click arrives:

| Column | From the renderer taking the click to |
|---|---|
| `ToModuleUs` | the module reading it, across the process boundary when the renderer is the helper |
| `ToHandlerUs` | a script block, run on the pipeline thread as a window's handler would run, reading the clock |
| `ToPipelineUs` | the command writing its record, after the handler |
| `ToEventUs` | with `-NoWait`, an engine-event action reading the clock |

## Twenty clicks, summarized

<!-- person: $w = Wait-Window 'PWRS-GUI latency'; 1..20 | ForEach-Object { Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click; Start-Sleep -Milliseconds 100 } -->
<!-- varies: times -->
```powershell
$clicks = Measure-GuiLatency -Count 20
$clicks | Select-Object -Skip 1 | Measure-Object -Property ToModuleUs, ToHandlerUs, ToPipelineUs -Minimum -Average -Maximum |
    Format-Table Property, @{ Name = 'Min'; Expression = { '{0:N1}' -f $_.Minimum } }, @{ Name = 'Mean'; Expression = { '{0:N1}' -f $_.Average } }, @{ Name = 'Max'; Expression = { '{0:N1}' -f $_.Maximum } }
```

```text
Property     Min  Mean  Max
--------     ---  ----  ---
ToModuleUs   13.6 22.4  35.7
ToHandlerUs  56.2 82.0  121.6
ToPipelineUs 76.4 117.3 286.8
```

The first click is left out of the summary: its handler run is the script block's first, which takes longer than the rest:

<!-- varies: times -->
```powershell
$clicks | Select-Object -First 2 | Format-Table Click, ToModuleUs, ToHandlerUs, ToPipelineUs
```

```text
Click ToModuleUs ToHandlerUs ToPipelineUs
----- ---------- ----------- ------------
    1      31.90     2595.10      4641.50
    2      28.30       81.20       107.00
```

## Engine-event actions

A window whose command returns while it stays open runs its handlers as engine-event actions, which PowerShell runs when the engine is idle: at an interactive prompt, or inside `Wait-Event` in a script. `-NoWait` measures that path:

<!-- person: $w = Wait-Window 'PWRS-GUI latency'; 1..10 | ForEach-Object { Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click; Start-Sleep -Milliseconds 100 } -->
<!-- varies: times -->
```powershell
$run = Measure-GuiLatency -NoWait -Count 10
Wait-Event -Timeout 3
$run.Samples() | Measure-Object -Property ToModuleUs, ToEventUs -Minimum -Average -Maximum |
    Format-Table Property, Count, @{ Name = 'Min'; Expression = { '{0:N1}' -f $_.Minimum } }, @{ Name = 'Mean'; Expression = { '{0:N1}' -f $_.Average } }, @{ Name = 'Max'; Expression = { '{0:N1}' -f $_.Maximum } }
Unregister-Event -SourceIdentifier $run.SourceIdentifier
```

```text
Property   Count Min      Mean     Max
--------   ----- ---      ----     ---
ToModuleUs    10 16.4     21.8     40.1
ToEventUs     10 17,523.9 91,319.6 153,682.3
```

`ToModuleUs` here is when the module's reading thread took the click, and `ToEventUs` when the action ran on the pipeline thread.

## On your machine

Run the same commands and click the button yourself: `Measure-GuiLatency -Count 20` writes a record per click as you go, and the window counts them. The numbers include whatever else the machine is doing, so a measurement worth comparing is taken on a machine otherwise idle, several times.
