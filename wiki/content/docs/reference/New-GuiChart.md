---
title: New-GuiChart
weight: 36
---

Describes a chart of series, drawn as lines, bars or points. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiChart`; the series and points as both renderers read them, `crates/pwrs-gui-proto/src/chart.rs`; the points it takes from a SubEtha ring, `crates/pwrs-gui-proto/src/source.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiChart -Syntax
```

```text
New-GuiChart [[-Series] <Object[]>] [-Type <ChartType>] [-XLabel <string>] [-YLabel <string>] [-Height <double>] [-Ring <Object>] [-OnSelect <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiChart` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as an app's chart, on the window's own background with faint lines across and the axes' text in a weaker tone, its bars solid and its lines 2 points wide; a legend names the series when there are several. The chart is of `-Series`, each a hashtable of:

- its `Name`;
- its `Y` numbers;
- its `X`, numbers or `[datetime]` values as many as `Y`, which are 0, 1, 2 and on when not given;
- its `Type`, `Line`, `Bar` or `Points`, the chart's `-Type` when not given.

One number or date given for `X` or `Y` is one point, as PowerShell binds one value to an array parameter. Every series' `X` are numbers or dates alike. With `-Height` the chart is that high; without it the chart shares the height its column leaves. In a `-Content` block `Chart` stands for `New-GuiChart`.

On a chart of dates, the X axis's labels name only what changes along it. Where its lines are less than a day apart, a label is the time, to the minute or to the second; where they are a day or more apart, or at midnight, it is the date. The lines are far enough apart for their labels to fit, and the values under the pointer give the full date and time.

A person drags to pan, zooms with Ctrl and the wheel, drags a box with the secondary button to zoom to it, double-clicks to go back, and reads the values under the pointer beside it. Each follows egui's rules for a drag and a click: a press is a drag once it goes past 6 points or lasts past 0.8 seconds, and the box starts where the pointer is then. A click on a point raises `Select`, which `-OnSelect` acts on, with an ordered dictionary of the point's `Series`, `X`, a `[datetime]` on a chart of dates, and `Y`; a bar takes the click anywhere on it, any other point within 5 points. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) clicks a point with `-Event Select -Value @{ Series; X; Y }` and refuses a point the chart does not have.

[Add-GuiItem](Add-GuiItem.md) adds points, hashtables of the `Series` each joins or starts, `X` and `Y`; a point without `X` takes its series' count of points, and on a chart of dates each point gives its date. Points another process writes to the SubEtha ring `-Ring` names are added the same way, each item a JSON object of its `Series`, `Y` and `X`, a number or a date and time written as ISO 8601. [Update-GuiWidget](Update-GuiWidget.md)'s `Series` replaces the series and the points added, a hashtable given alone being one series, and it sets `Type`, `XLabel`, `YLabel` and `Height`. `-Ring` takes the object one of these wrote:

| Ring | Its items |
|---|---|
| `New-SubEthaRing` | sent with `SendFrame`, of any length |
| `New-SubEthaLocaleRing` | sent with `Send`, at most one slot, once the writer has moved the ring to `File` or `ShmFs` with `MigrateTo`: a locale ring starts in its writer's own memory, where no other process can read it |
| `New-SubEthaBroadcastRing` | pushed with `Push`, at most 52 bytes; every window reading the ring gets every item |

A point the chart cannot take, such as a date on a chart of numbers, is written as a warning naming the item and why. The renderer reads the ring on a thread of its own, woken by the ring when an item arrives, so nothing polls. A ring made with order stamps, and a capacity ring, whose resizes a reader in another process does not follow, are refused. [How To Feed A Widget From A SubEtha Ring](How-To-Feed-A-Widget-From-A-SubEtha-Ring.md) shows each kind of ring.

## Parameters

### -Series

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty chart |

The series, each a hashtable of `Name` and `Y`, and of `X` and `Type` when given; one number or date for `X` or `Y` is one point.

### -Type

| | |
|---|---|
| Type | `PwrsGui.ChartType` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Line` |

How a series without a `Type` of its own is drawn: `Line`, `Bar` or `Points`.

### -XLabel

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name of the horizontal axis.

### -YLabel

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name of the vertical axis.

### -Height

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: a share of the height its column leaves |

How high the chart is, in points, above 0.

### -Ring

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A SubEtha ring another process writes, each item one point, as the description says. It is taken when the window opens; `Update-GuiWidget` cannot change it.

### -OnSelect

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person clicks a point, with the event as its argument and as `$_`; its `Value` has the point's `Series`, `X` and `Y`.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the chart, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the chart.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Chart` and its `Properties` with `Series`, and `Type`, `XLabel`, `YLabel`, `Height` and `Ring` when they are given.

## Examples

Two series, one of them bars, and a point added while the window is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Load' {
    Chart -Name Load -Height 240 -YLabel 'percent' -Series @{ Name = 'cpu'; Y = 3, 5, 4, 6 }, @{ Name = 'memory'; Y = 40, 42, 41, 45; Type = 'Bar' }
}
Add-GuiItem -Window $w.Id -Name Load -Item @{ Series = 'cpu'; Y = 7 }
$chart = (Get-GuiSnapshot -Window $w.Id).Tree
'{0} series, {1} point added, the vertical axis {2}' -f $chart.Properties.series.Count, $chart.Properties.points.Count, $chart.Properties.y_label
Close-GuiWindow -Window $w.Id
```

```text
2 series, 1 point added, the vertical axis percent
```

A click on a point, then the window closed by its button. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Load'; Invoke-GuiTestEvent -Window $w.Id -Name Load -Event Select -Value @{ Series = 'cpu'; X = 2; Y = 4 }; Invoke-GuiTestEvent -Window $w.Id -Name Done -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Load' -HandlerOutput Pipeline {
    Chart -Name Load -Height 240 -Series @{ Name = 'cpu'; Y = 3, 5, 4, 6 } -OnSelect { '{0} at {1}: {2}' -f $_.Value.Series, $_.Value.X, $_.Value.Y }
    ButtonBar { Button 'Done' -Name Done -Default -Result Done }
}
$out[0]
$out[1].Result
```

```text
cpu at 2: 4
Done
```

A chart of dates, told by its first series:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Daily' {
    Chart -Name Daily -Height 240 -Series @{ Name = 'deploys'; X = (0..2 | ForEach-Object { (Get-Date).Date.AddDays($_ - 2) }); Y = 2, 0, 3 }
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.dates
Close-GuiWindow -Window $w.Id
```

```text
True
```

A series whose `X` and `Y` differ in number:

```powershell
try { New-GuiChart -Series @{ Name = 'cpu'; X = 1, 2, 3; Y = 3, 5 } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiChartCommand
Series: series 1: X has 3 values and Y 2; they must be as many
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | a series is not a hashtable of `Name` and `Y` with as many `X` as `Y` and a `Type` of `Line`, `Bar` or `Points`; the series mix dates and numbers; or `-Ring` names no ring a widget reads |
| `GuiBadSize` | `-Height` is not above 0 |
| `GuiOpeningOnly` | `Update-GuiWidget` sets `Ring`, which is taken when the window opens |

Each is described in the [Error Reference](Error-Reference.md).
