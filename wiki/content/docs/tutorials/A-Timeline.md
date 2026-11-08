---
title: A Timeline
weight: 8
---

Chapter 7's LogLens summarizes the log as a page. This chapter draws the log as a timeline: each line a dot at its time, in a row for its source and in its level's color, on a [Canvas](New-GuiCanvas.md) a person pans and zooms as they would a map, and a click on a dot shows its line. Source: the items are read in `crates/pwrs-gui/src/canvas.rs`, and `Set-GuiCanvasView` is `SetGuiCanvasView` in `crates/pwrs-gui/src/gpu.rs`.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows, and the line added to the log was written from that runspace too, standing for the service that writes the log.

<details>
<summary>The log and the line reader from chapter 1, written again so this page's examples have them</summary>

```powershell
Set-Content -Path app.log -Value @'
2026-10-20T09:30:01 INFO web01 started
2026-10-20T09:30:02 INFO web02 started
2026-10-20T09:30:04 INFO db01 accepting connections
2026-10-20T09:30:15 INFO web01 GET /orders 200
2026-10-20T09:30:41 WARN web02 GET /orders took 1.8 s
2026-10-20T09:31:07 INFO web01 GET /orders/17 200
2026-10-20T09:31:30 ERROR db01 deadlock on table orders
2026-10-20T09:31:31 ERROR web01 GET /orders/17 500
2026-10-20T09:31:52 INFO web02 GET /health 200
2026-10-20T09:32:10 WARN db01 replica 2 behind by 4 s
2026-10-20T09:32:44 INFO web01 GET /orders 200
2026-10-20T09:33:05 INFO web02 POST /orders 201
2026-10-20T09:33:29 ERROR web02 POST /orders 503
2026-10-20T09:33:30 WARN web02 retrying POST /orders
2026-10-20T09:33:31 INFO web02 POST /orders 201
2026-10-20T09:34:12 INFO db01 checkpoint done
2026-10-20T09:34:48 WARN web01 cache miss rate 41%
2026-10-20T09:35:20 INFO web01 GET /orders 200
2026-10-20T09:35:57 ERROR db01 disk 92% full
2026-10-20T09:36:03 WARN db01 vacuum skipped
2026-10-20T09:36:40 INFO web02 GET /health 200
2026-10-20T09:37:15 ERROR web01 GET /orders 500
2026-10-20T09:37:16 ERROR web02 GET /orders 500
2026-10-20T09:37:30 WARN db01 replica 2 behind by 9 s
2026-10-20T09:38:02 INFO db01 replica 2 caught up
2026-10-20T09:38:40 INFO web01 GET /orders 200
2026-10-20T09:39:11 INFO web02 GET /orders 200
2026-10-20T09:39:55 INFO web01 stopping
'@
function ConvertFrom-LogLine {
    param([Parameter(ValueFromPipeline)] [string] $Line)
    process {
        $time, $level, $source, $message = $Line -split ' ', 4
        [pscustomobject]@{ Time = [datetime]$time; Level = $level; Source = $source; Message = $message }
    }
}
$entries = @(Get-Content -LiteralPath app.log | ConvertFrom-LogLine)
```

</details>

## The lines as dots

A canvas draws items: hashtables that each name their `Kind` and the keys of their shape, drawn in the order given, the first underneath. LogLens puts each line's dot at its time across, one point to a second from `$start`, the minute the log starts in, and in its source's row down, in its level's color. A faint line and a label mark each minute up to the first one after the last line, so every dot is between two of them, and each source's name is at the left of its row. The minutes' lines end at `$bottom`, below the last row, and their labels are under it. A dot's `Name` gives its line's place in `$entries`, which the sections after this one use to find the line a person clicks:

<!-- keep: a-timeline-dots.png images/a-timeline-dots.png -->
<!-- person: $w = Wait-Window 'LogLens timeline'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-timeline-dots.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$first = $entries[0].Time
$start = $first.Date.AddHours($first.Hour).AddMinutes($first.Minute)
$lanes = @($entries.Source | Sort-Object -Unique)
$levelColor = @{ INFO = 'SteelBlue'; WARN = 'DarkOrange'; ERROR = 'Crimson' }
function Get-X($Time) { 60 + ($Time - $start).TotalSeconds }
function Get-Y($Source) { 20 + 30 * $lanes.IndexOf($Source) }
$bottom = 30 * $lanes.Count + 5
$items = @(
    $minute = $start
    while ($minute -lt $entries[-1].Time.AddMinutes(1)) {
        @{ Kind = 'Line'; X1 = (Get-X $minute); Y1 = 5; X2 = (Get-X $minute); Y2 = $bottom; Opacity = 0.2 }
        @{ Kind = 'Text'; X = (Get-X $minute) - 15; Y = $bottom + 5; Text = $minute.ToString('HH:mm'); Size = 11; Opacity = 0.6 }
        $minute = $minute.AddMinutes(1)
    }
    foreach ($source in $lanes) { @{ Kind = 'Text'; X = 0; Y = (Get-Y $source) - 8; Text = $source; Size = 13 } }
    for ($i = 0; $i -lt $entries.Count; $i++) {
        $line = $entries[$i]
        @{ Kind = 'Ellipse'; Name = "line$i"; X = (Get-X $line.Time) - 5; Y = (Get-Y $line.Source) - 5; Width = 10; Height = 10; Fill = $levelColor[$line.Level] }
    }
)
(Show-GuiWindow -Title 'LogLens timeline' {
    Canvas -Item $items -Name Timeline -Width 680 -Height 120
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}).Result
```

```text
Close
```

![The LogLens timeline window: rows for db01, web01 and web02 with each line's dot in its row, blue for INFO, orange for WARN and red for ERROR, faint lines and labels at each minute from 09:30 to 09:40, and a Close button at the bottom right](/images/a-timeline-dots.png)

The minute lines and every label have neither `Fill` nor `Stroke`, so they are drawn in the theme's text color, and the timeline reads in a light window and a dark one alike. `Opacity` fades the lines to hairlines and the minutes to the tone of the window's weaker text, so the dots stand out, as an app's chart leaves its grid in the background. A canvas without a view draws its items at their own points, from its top left.

## A view to pan and zoom

[Set-GuiCanvasView](Set-GuiCanvasView.md) gives a canvas a view: a point of the items' coordinates at the canvas's middle, at a zoom, and controls a person moves it with. `-Fit` fits the view to every item. The controls, when not named, are `Map`'s: a drag pans and glides on when let go, the wheel zooms about the pointer, a double-click zooms in a step, and Shift and a drag draw a box to zoom to.

The view needs an open window, so this one opens with `-NoWait`, and the script reads its events with [Wait-GuiEvent](Wait-GuiEvent.md) until it closes, as chapter 4 did. Without `-Width` and `-Height` the canvas takes the room the window leaves, and the view fits the dots to it. Click Close:

<!-- keep: a-timeline-view.png images/a-timeline-view.png -->
<!-- person: $w = Wait-Window 'LogLens timeline'; $null = Get-GuiSnapshot -Window $w.Id -Name Timeline -Pixels; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-timeline-view.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'LogLens timeline' -Width 760 -Height 240 {
    Canvas -Item $items -Name Timeline
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
Set-GuiCanvasView Timeline -Fit -Window $w.Id
do { $e = Wait-GuiEvent -Window $w.Id } until ($e.Kind -eq 'Close')
$e.Value
```

```text
Close
```

![The LogLens timeline window 760 points across, its rows of dots and its minutes fitted to the canvas's width, and a Close button at the bottom right](/images/a-timeline-view.png)

What a person does with the view stays with the canvas, and [Get-GuiCanvasView](Get-GuiCanvasView.md) reads where they left it.

## Clicking a dot

A click on a named item runs the canvas's `-OnClick` handler, whose event's `Value.Item` is the item's name, here `line` and the line's place in `$entries`. The handler marks the dot clicked with a ring and writes the line under the timeline. The ring is an item named `Mark`, hidden at first, that [Update-GuiWidget](Update-GuiWidget.md) `-Item` moves to the dot and shows. It comes first in the items, so it is drawn under the dots, and the dot stays the topmost item where the person clicks. Click the ERROR dot at 09:31:30 in db01's row, then Close:

<!-- keep: a-timeline-picked.png images/a-timeline-picked.png -->
<!-- person: $w = Wait-Window 'LogLens timeline'; $null = Get-GuiSnapshot -Window $w.Id -Name Timeline -Pixels; Invoke-GuiTestEvent -Window $w.Id -Name Timeline -Event Click -Value @{ Item = 'line6' }; Wait-Property $w Picked text 'ERROR from db01 at 09:31:30: deadlock on table orders'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-timeline-picked.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$mark = @{ Kind = 'Ellipse'; Name = 'Mark'; X = 0; Y = 0; Width = 18; Height = 18; Fill = 'Transparent'; Stroke = 'DodgerBlue'; StrokeWidth = 2; Visible = $false }
$picked = {
    if ($_.Value.Item -notlike 'line*') { return }
    $line = $entries[[int]$_.Value.Item.Substring(4)]
    Update-GuiWidget -Name Timeline -Item Mark -Property @{ X = (Get-X $line.Time) - 9; Y = (Get-Y $line.Source) - 9; Visible = $true }
    Update-GuiWidget -Name Picked -Property @{ Text = "$($line.Level) from $($line.Source) at $($line.Time.ToString('HH:mm:ss')): $($line.Message)" }
}
(Show-GuiWindow -Title 'LogLens timeline' {
    Canvas -Item (@($mark) + $items) -Name Timeline -Width 680 -Height 120 -OnClick $picked
    Text 'Click a dot to see its line' -Name Picked
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}).Result
```

```text
Close
```

![The LogLens timeline window with a blue ring around the red dot at 09:31:30 in db01's row, the text ERROR from db01 at 09:31:30: deadlock on table orders under the timeline, and a Close button at the bottom right](/images/a-timeline-picked.png)

A canvas tells its handlers of clicks only on named items, and a click on the ring around a dot names `Mark`, which the handler passes over. A point within 5 points of a dot counts as on it, so dots a few seconds apart touch at one point to a second, and a click between them goes to the later line's dot, drawn on top; a view zoomed in parts them. The handler runs on the command's own thread while the window waits, so `Update-GuiWidget` finds the window without `-Window`, as chapter 4's handlers did.

## The whole script so far

`LogLens.ps1` now has a Timeline tab. Choosing it fits the view to the dots, and a click on a dot marks it and shows its line. The minutes and the sources' names are a [Group](New-GuiCanvas.md) of their own, named `Grid`, drawn from the log's span and its sources. Each line the log gains adds its dot, and then the script takes the grid away and adds it again, so its minutes reach the newest line and a new source has its row. The grid's `Order` of -1 draws it under the ring and the dots, whatever order they were added in. The `summary` folder from chapter 7 stays beside the script, for the View menu's Summary:

<!-- file: LogLens.ps1 -->
```powershell
param([string] $Path, [int] $Large = 10000)
Import-Module PWRS-GUI

function ConvertFrom-LogLine {
    param([Parameter(ValueFromPipeline)] [string] $Line)
    process {
        $time, $level, $source, $message = $Line -split ' ', 4
        [pscustomobject]@{ Time = [datetime]$time; Level = $level; Source = $source; Message = $message }
    }
}

function New-Tail([string] $Path) {
    [pscustomobject]@{ Path = (Resolve-Path -LiteralPath $Path).ProviderPath; Position = 0; Rest = '' }
}

function Read-NewLines($Tail) {
    $stream = [System.IO.FileStream]::new($Tail.Path, 'Open', 'Read', 'ReadWrite, Delete')
    try {
        if ($stream.Length -lt $Tail.Position) { $Tail.Position = 0; $Tail.Rest = '' }
        $stream.Position = $Tail.Position
        $text = [System.IO.StreamReader]::new($stream).ReadToEnd()
        $Tail.Position = $stream.Position
    } finally {
        $stream.Dispose()
    }
    $lines = ($Tail.Rest + $text) -split "`r?`n"
    $Tail.Rest = $lines[-1]
    if ($lines.Count -gt 1) { $lines[0..($lines.Count - 2)] }
}

function Test-Matching($Entry, $State) {
    ($State.Level -eq 'All' -or $Entry.Level -eq $State.Level) -and $Entry.Message -like "*$($State.Filter)*"
}

function Show-Matching($WindowEvent) {
    $state = $WindowEvent.State
    $matching = @($entries | Where-Object { Test-Matching $_ $state })
    if ($shown.Count) { Remove-GuiItem -Window $WindowEvent.Window -Name Lines -Item $shown.ToArray() }
    $shown.Clear()
    if ($matching.Count) {
        Add-GuiItem -Window $WindowEvent.Window -Name Lines -Item $matching
        $shown.AddRange($matching)
    }
}

function Get-ErrorSeries {
    $perMinute = @($entries | Where-Object Level -eq 'ERROR' | Group-Object { $_.Time.ToString('yyyy-MM-dd HH:mm') })
    @{ Name = 'Errors'; Type = 'Bar'; X = @($perMinute | ForEach-Object { [datetime]$_.Name }); Y = @($perMinute | ForEach-Object Count) }
}

function Get-Counts { "$($entries.Count) lines, $(@($entries | Where-Object Level -eq 'ERROR').Count) errors" }

function Get-Summary {
    foreach ($source in $entries | Group-Object Source) {
        $count = $source.Group | Group-Object Level -AsHashTable -AsString
        [pscustomobject]@{ Source = $source.Name; INFO = $count['INFO'].Count; WARN = $count['WARN'].Count; ERROR = $count['ERROR'].Count }
    }
}

function Get-X($Time) { 60 + ($Time - $start).TotalSeconds }
function Get-Y($Source) { 20 + 30 * $lanes.IndexOf($Source) }

function Get-Dot($At) {
    $line = $entries[$At]
    @{ Kind = 'Ellipse'; Name = "line$At"; X = (Get-X $line.Time) - 5; Y = (Get-Y $line.Source) - 5; Width = 10; Height = 10; Fill = $levelColor[$line.Level] }
}

function Get-Grid {
    $bottom = 30 * $lanes.Count + 5
    $marks = @(
        $minute = $start
        while ($entries.Count -and $minute -lt $entries[-1].Time.AddMinutes(1)) {
            @{ Kind = 'Line'; X1 = (Get-X $minute); Y1 = 5; X2 = (Get-X $minute); Y2 = $bottom; Opacity = 0.2 }
            @{ Kind = 'Text'; X = (Get-X $minute) - 15; Y = $bottom + 5; Text = $minute.ToString('HH:mm'); Size = 11; Opacity = 0.6 }
            $minute = $minute.AddMinutes(1)
        }
        foreach ($source in $lanes) { @{ Kind = 'Text'; X = 0; Y = (Get-Y $source) - 8; Text = $source; Size = 13 } }
    )
    @{ Kind = 'Group'; Name = 'Grid'; Order = -1; Children = $marks }
}

function Get-TimelineItems {
    Get-Grid
    @{ Kind = 'Ellipse'; Name = 'Mark'; X = 0; Y = 0; Width = 18; Height = 18; Fill = 'Transparent'; Stroke = 'DodgerBlue'; StrokeWidth = 2; Visible = $false }
    for ($i = 0; $i -lt $entries.Count; $i++) { Get-Dot $i }
}

function Show-Picked($CanvasEvent) {
    if ($CanvasEvent.Value.Item -notlike 'line*') { return }
    $line = $entries[[int]$CanvasEvent.Value.Item.Substring(4)]
    Update-GuiWidget -Name Timeline -Item Mark -Property @{ X = (Get-X $line.Time) - 9; Y = (Get-Y $line.Source) - 9; Visible = $true }
    Update-GuiWidget -Name Picked -Property @{ Text = "$($line.Level) from $($line.Source) at $($line.Time.ToString('HH:mm:ss')): $($line.Message)" }
}

function Add-NewEntries($WindowEvent) {
    $window = $WindowEvent.Window
    $new = @(Read-NewLines $tail | ConvertFrom-LogLine)
    foreach ($entry in $new) {
        $entries.Add($entry)
        if ($sourceNames.Add($entry.Source)) {
            Add-GuiItem -Window $window -Name Sources -Item ([pscustomobject]@{ Source = $entry.Source })
            $lanes.Add($entry.Source)
        }
        Add-GuiItem -Window $window -Name Timeline -Item (Get-Dot ($entries.Count - 1))
        if (Test-Matching $entry $WindowEvent.State) {
            Add-GuiItem -Window $window -Name Lines -Item $entry
            $shown.Add($entry)
        }
    }
    if ($new.Count) {
        Remove-GuiItem -Window $window -Name Timeline -Item Grid
        Add-GuiItem -Window $window -Name Timeline -Item (Get-Grid)
        Update-GuiWidget -Window $window -Name Errors -Property @{ Series = Get-ErrorSeries }
        Update-GuiWidget -Window $window -Name Counts -Property @{ Text = Get-Counts }
    }
}

if (-not $Path) {
    $Path = Read-GuiFile -Filter 'Logs|*.log', 'All files|*' -Title 'Open a log'
    if (-not $Path) { return }
}
if (-not (Test-Path -LiteralPath $Path)) {
    $null = Show-GuiMessage "$(Split-Path -Leaf $Path) does not exist." -Title 'LogLens' -Icon Error
    return
}
$Path = (Resolve-Path -LiteralPath $Path).ProviderPath
$tail = New-Tail $Path
$lines = @(Read-NewLines $tail)
$entries = [System.Collections.Generic.List[object]]::new()
$progress = if ($lines.Count -gt $Large) { Show-GuiProgress -Activity "Reading $(Split-Path -Leaf $Path)" -Status "0 of $($lines.Count) lines" }
foreach ($line in $lines) {
    $entries.Add(($line | ConvertFrom-LogLine))
    if ($progress -and $entries.Count % 10000 -eq 0) {
        if ($progress.State -eq 'Closed') { return }
        Set-GuiProgress $progress -Status "$($entries.Count) of $($lines.Count) lines" -PercentComplete ($entries.Count * 100 / $lines.Count)
    }
}
if ($progress) { Set-GuiProgress $progress -Completed }
$shown = [System.Collections.Generic.List[object]]::new($entries)
$sourceNames = [System.Collections.Generic.HashSet[string]]::new()
$sources = foreach ($entry in $entries) {
    if ($sourceNames.Add($entry.Source)) { [pscustomobject]@{ Source = $entry.Source } }
}
$first = if ($entries.Count) { $entries[0].Time } else { Get-Date }
$start = $first.Date.AddHours($first.Hour).AddMinutes($first.Minute)
$lanes = [System.Collections.Generic.List[string]]::new([string[]]@($entries.Source | Sort-Object -Unique))
$levelColor = @{ INFO = 'SteelBlue'; WARN = 'DarkOrange'; ERROR = 'Crimson' }
$details = {
    $e = $_.Value
    Update-GuiWidget -Name Details -Property @{ Text = "$($e.Level) from $($e.Source) at $($e.Time.ToString('HH:mm:ss')): $($e.Message)" }
}
$nodeText = { if ($_.Message) { "$($_.Time.ToString('HH:mm:ss')) $($_.Message)" } elseif ($_.Level) { $_.Level } else { $_.Source } }
$opens = { -not $_.Message }
$inside = {
    $node = $_.Value
    if ($node.Level) {
        $entries | Where-Object { $_.Source -eq $node.Source -and $_.Level -eq $node.Level }
    } else {
        $entries | Where-Object Source -eq $node.Source | Select-Object -ExpandProperty Level -Unique | ForEach-Object { [pscustomobject]@{ Source = $node.Source; Level = $_ } }
    }
}
$chosen = {
    $line = $_.Value
    $text = if ($line.Message) { "$($line.Level) from $($line.Source) at $($line.Time.ToString('HH:mm:ss')): $($line.Message)" } else { 'Open a source and a level, and select a line' }
    Update-GuiWidget -Name Chosen -Property @{ Text = $text }
}
$summary = Join-Path $PSScriptRoot 'summary'
$asked = {
    param($message)
    if ($message.Want -eq 'Summary') { , @(Get-Summary) }
    else { , @($entries | Where-Object Source -eq $message.Source | ForEach-Object { [pscustomobject]@{ Time = $_.Time.ToString('HH:mm:ss'); Level = $_.Level; Message = $_.Message } }) }
}
$out = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 -HandlerOutput Pipeline -WatchPath $Path -OnFileChange { Add-NewEntries $_ } {
    MenuBar {
        Menu 'Edit' { MenuItem 'Find' -Name Find -Shortcut Ctrl+F -OnClick { Set-GuiFocus Filter } }
        Menu 'View' { MenuItem 'Summary' -Name Summary -OnClick { $null = Show-GuiWindow -Path $summary -Title 'LogLens summary' -Width 460 -Height 510 -OnMessage $asked } }
    }
    Tabs -Name Views -OnChange { if ($_.Value -eq 'Timeline') { Set-GuiCanvasView Timeline -Fit } } {
        Tab 'Lines' {
            Row {
                Text 'Find'
                TextBox -Name Filter -Placeholder 'text to find' -OnChange { Show-Matching $_ }
                Text 'Level'
                ComboBox 'All', 'INFO', 'WARN', 'ERROR' -Name Level -Selected All -OnChange { Show-Matching $_ }
            }
            Table $entries -Name Lines -OnSelect $details -OnActivate { $_.Value }
            Text 'Select a line to see it here' -Name Details
        }
        Tab 'Errors' { Chart -Series (Get-ErrorSeries) -XLabel 'Minute' -YLabel 'Errors' -Name Errors }
        Tab 'Sources' {
            Split -Position 0.4 {
                TreeView $sources -Name Sources -Text $nodeText -HasChildren $opens -OnExpand $inside -OnSelect $chosen
                Text 'Open a source and a level, and select a line' -Name Chosen
            }
        }
        Tab 'Timeline' {
            Canvas -Item (Get-TimelineItems) -Name Timeline -OnClick { Show-Picked $_ }
            Text 'Click a dot to see its line' -Name Picked
        }
    }
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
    StatusBar { Text (Get-Counts) -Name Counts }
}
$out | Select-Object -SkipLast 1
```

Run it. A line from a new source, cache01, arrives as the log grows: the timeline's minutes run on to 09:42, cache01 gets a row, and the line's dot is in it. Choose Timeline, click the new dot, and look at its line; then choose Lines, select the new line at the table's end, press Enter, and press Close:

<!-- keep: a-timeline-loglens.png images/a-timeline-loglens.png -->
<!-- person: $w = Wait-Window 'LogLens'; '2026-10-20T09:41:03 ERROR cache01 eviction storm' | Out-File -LiteralPath (Join-Path $PWD 'app.log') -Append -Encoding utf8; Wait-Property $w Counts text '29 lines, 7 errors'; Invoke-GuiTestEvent -Window $w.Id -Name Views -Event Change -Value 3; $null = Get-GuiSnapshot -Window $w.Id -Name Timeline -Pixels; Invoke-GuiTestEvent -Window $w.Id -Name Timeline -Event Click -Value @{ Item = 'line28' }; Wait-Property $w Picked text 'ERROR from cache01 at 09:41:03: eviction storm'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-timeline-loglens.png'); Invoke-GuiTestEvent -Window $w.Id -Name Views -Event Change -Value 0; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 28; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Activate -Value 28; Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log | Format-Table Time, Level, Source, Message
```

```text
Time                  Level Source  Message
----                  ----- ------  -------
10/20/2026 9:41:03 AM ERROR cache01 eviction storm
```

![LogLens with the Timeline tab chosen: rows for db01, web01, web02 and cache01 over the minutes from 09:30 to 09:42, a blue ring around cache01's red dot after 09:41, the text ERROR from cache01 at 09:41:03: eviction storm under it, and a status bar reading 29 lines, 7 errors](/images/a-timeline-loglens.png)

The view fits the dots each time the tab is chosen, so a dot added while another tab was shown is in sight when the person comes back.

## Next

[A Heat Map On The GPU](A-Heat-Map-On-The-GPU.md): a heat map of the log's lines per minute and source, counted and drawn on the GPU.
