---
title: Charts And Trees
weight: 6
---

Chapter 5's LogLens follows its log in a table of lines. This chapter gives it two more views of the same entries: a chart of the errors in each minute, and a tree of the entries by source and level, each on a tab of its own, the tree beside the details of the line chosen in it. A status bar along the window's bottom counts the lines and the errors as the log grows. Source: the widgets are `crates/pwrs-gui/src/widgets.rs`, and a chart's series and points are read by `crates/pwrs-gui-proto/src/chart.rs`.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the keys and the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows. The lines added to a log were written from that runspace too, standing for the service that writes the log.

<details>
<summary>The log and the line reader from chapter 1, and the tail reader from chapter 5, written again so this page's examples have them</summary>

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
$entries = @(Get-Content -LiteralPath app.log | ConvertFrom-LogLine)
```

</details>

## Errors per minute

A [Chart](New-GuiChart.md) draws series of points, each series a hashtable of its `Name`, its `X` and `Y` values, and its `Type`: `Line`, `Bar` or `Points`. `Group-Object` counts the log's errors by the minute they came in, and each minute's `[datetime]` is a bar's `X`, so the chart's axis reads as times. Press Close:

<!-- keep: charts-and-trees-chart.png images/charts-and-trees-chart.png -->
<!-- person: $w = Wait-Window 'LogLens'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'charts-and-trees-chart.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$perMinute = @($entries | Where-Object Level -eq 'ERROR' | Group-Object { $_.Time.ToString('yyyy-MM-dd HH:mm') })
$series = @{ Name = 'Errors'; Type = 'Bar'; X = @($perMinute | ForEach-Object { [datetime]$_.Name }); Y = @($perMinute | ForEach-Object Count) }
$result = Show-GuiWindow -Title 'LogLens' {
    Chart -Series $series -XLabel 'Minute' -YLabel 'Errors' -Name Errors
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$perMinute | Format-Table Name, Count
```

```text
Name             Count
----             -----
2026-10-20 09:31     2
2026-10-20 09:33     1
2026-10-20 09:35     1
2026-10-20 09:37     2
```

![The LogLens window with a bar chart of errors by minute, bars of 2 at 09:31, 1 at 09:33, 1 at 09:35 and 2 at 09:37, and a Close button at the bottom right](/images/charts-and-trees-chart.png)

A person drags the chart to pan it, zooms with Ctrl and the wheel, and reads the values under the pointer beside it; a double click goes back. [Update-GuiWidget](Update-GuiWidget.md) gives a chart new series while its window is open, which the whole script does as the log grows.

## The log by source

A [TreeView](New-GuiTreeView.md) shows nodes that open to show the nodes inside them. Its top nodes here are objects naming the log's sources. `-HasChildren` says which nodes open, and `-OnExpand` writes the nodes inside one as it opens, worked out from the entries at that moment: a source's levels, then a level's lines. `-Text` gives each node's text, from the node as `$_`. A window fits the widgets it opens with, so `-Height` gives this one room for the nodes that open later. Open db01, then its ERROR, and select the line about the disk:

<!-- keep: charts-and-trees-tree.png images/charts-and-trees-tree.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Expand -Value 2; while (@((Get-GuiSnapshot -Window $w.Id).State.Sources.shown).Count -lt 6) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Expand -Value 4; while (@((Get-GuiSnapshot -Window $w.Id).State.Sources.shown).Count -lt 8) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Select -Value 6; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'charts-and-trees-tree.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$sources = $entries | Select-Object -ExpandProperty Source -Unique | ForEach-Object { [pscustomobject]@{ Source = $_ } }
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
$result = Show-GuiWindow -Title 'LogLens' -Height 300 {
    TreeView $sources -Name Sources -Text $nodeText -HasChildren $opens -OnExpand $inside
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.State.Sources | Format-List
```

```text
Time    : 10/20/2026 9:35:57 AM
Level   : ERROR
Source  : db01
Message : disk 92% full
```

![The LogLens window with a tree of web01, web02 and db01, db01 open to its levels INFO, ERROR and WARN, ERROR open to its two lines, the line about the disk selected, and an OK button at the bottom right](/images/charts-and-trees-tree.png)

`State` gives the object the selected node was made from: here a line's entry. A node's nodes are worked out once, as it first opens, so a node opened before the log grew shows the lines it had then.

## Tabs and a split

[Tabs](New-GuiTabs.md) shows one [Tab](New-GuiTab.md) at a time under a row of their titles, so the three views share one window. A [Split](New-GuiSplit.md) puts two widgets side by side, with a divider a person drags: here the tree, beside the line chosen in it. A line too long for the tree ends in an ellipsis, and shows whole beside the tree once chosen. Choose the Sources tab, open web02 and its WARN, and select the line about retrying:

<!-- keep: charts-and-trees-tabs.png images/charts-and-trees-tabs.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Views -Event Change -Value 2; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Expand -Value 1; while (@((Get-GuiSnapshot -Window $w.Id).State.Sources.shown).Count -lt 6) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Expand -Value 3; while (@((Get-GuiSnapshot -Window $w.Id).State.Sources.shown).Count -lt 8) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Select -Value 5; Wait-Property $w Chosen text 'WARN from web02 at 09:33:30: retrying POST /orders'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'charts-and-trees-tabs.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$chosen = {
    $line = $_.Value
    $text = if ($line.Message) { "$($line.Level) from $($line.Source) at $($line.Time.ToString('HH:mm:ss')): $($line.Message)" } else { 'Open a source and a level, and select a line' }
    Update-GuiWidget -Name Chosen -Property @{ Text = $text }
}
$result = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 {
    Tabs -Name Views {
        Tab 'Lines' { Table $entries -Name Lines }
        Tab 'Errors' { Chart -Series $series -XLabel 'Minute' -YLabel 'Errors' -Name Errors }
        Tab 'Sources' {
            Split -Position 0.4 {
                TreeView $sources -Name Sources -Text $nodeText -HasChildren $opens -OnExpand $inside -OnSelect $chosen
                Text 'Open a source and a level, and select a line' -Name Chosen
            }
        }
    }
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$result.State.Views
```

```text
Sources
```

![The LogLens window with the tabs Lines, Errors and Sources, Sources chosen, its tree on the left with web02 and its WARN open and the line about retrying selected, the text WARN from web02 at 09:33:30: retrying POST /orders on the right, and a Close button at the bottom right](/images/charts-and-trees-tabs.png)

Tabs is an input: `State` gives the title of the tab shown, and choosing a tab raises `Change` with it. The inputs on a tab not shown do not lose their values, so the table's selection is still there when Lines is chosen again.

## A status bar

A [StatusBar](New-GuiStatusBar.md) runs along the window's bottom, wherever the block writes it, and has widgets the update commands find by their names. Here a Text in it counts the lines and the errors, and `-OnFileChange`, from chapter 5, sets it as the log grows. The example follows a copy of the log, so the rest of the page reads the log as it was:

<!-- keep: charts-and-trees-status.png images/charts-and-trees-status.png -->
<!-- person: $w = Wait-Window 'LogLens'; '2026-10-20T09:40:12 ERROR db01 disk 95% full' | Out-File -LiteralPath (Join-Path $PWD 'status.log') -Append -Encoding utf8; Wait-Property $w Counts text '29 lines, 7 errors'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'charts-and-trees-status.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
Copy-Item -Path app.log -Destination status.log
$all = [System.Collections.Generic.List[object]]::new($entries)
$tail = New-Tail status.log
$null = Read-NewLines $tail
function Get-Counts { "$($all.Count) lines, $(@($all | Where-Object Level -eq 'ERROR').Count) errors" }
$result = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 -WatchPath status.log -OnFileChange {
    foreach ($entry in @(Read-NewLines $tail | ConvertFrom-LogLine)) {
        $all.Add($entry)
        Add-GuiItem -Name Lines -Item $entry
    }
    Update-GuiWidget -Name Counts -Property @{ Text = Get-Counts }
} {
    Table $entries -Name Lines
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
    StatusBar { Text (Get-Counts) -Name Counts }
}
Get-Counts
```

```text
29 lines, 7 errors
```

![The LogLens window with the table of the log's lines, the newest about the disk at 95%, a Close button, and along the bottom a status bar reading 29 lines, 7 errors](/images/charts-and-trees-status.png)

`$null = Read-NewLines $tail` reads the lines there now and drops them, since `$entries` has them already, so the tail starts at the log's end.

## The whole script so far

`LogLens.ps1` now shows its views on three tabs: the field, the drop-down, the table and the line's details on Lines; the chart on Errors; the tree beside the line chosen in it on Sources. The status bar counts the lines and the errors. `Add-NewEntries` adds each new line to the table when it matches, sets the chart and the counts again, and adds a top node to the tree for a source the log had not named before:

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

function Add-NewEntries($WindowEvent) {
    $window = $WindowEvent.Window
    $new = @(Read-NewLines $tail | ConvertFrom-LogLine)
    foreach ($entry in $new) {
        $entries.Add($entry)
        if ($sourceNames.Add($entry.Source)) {
            Add-GuiItem -Window $window -Name Sources -Item ([pscustomobject]@{ Source = $entry.Source })
        }
        if (Test-Matching $entry $WindowEvent.State) {
            Add-GuiItem -Window $window -Name Lines -Item $entry
            $shown.Add($entry)
        }
    }
    if ($new.Count) {
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
$out = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 -HandlerOutput Pipeline -WatchPath $Path -OnFileChange { Add-NewEntries $_ } {
    MenuBar { Menu 'Edit' { MenuItem 'Find' -Name Find -Shortcut Ctrl+F -OnClick { Set-GuiFocus Filter } } }
    Tabs -Name Views {
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
    }
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
    StatusBar { Text (Get-Counts) -Name Counts }
}
$out | Select-Object -SkipLast 1
```

Run it. A line from a new source, cache01, arrives as the log grows: the status bar counts it, and the tree gains cache01. Choose Sources, open cache01 and its ERROR, and select its line; then choose Lines, select the new line at the table's end, press Enter, and press Close:

<!-- keep: charts-and-trees-loglens.png images/charts-and-trees-loglens.png -->
<!-- person: $w = Wait-Window 'LogLens'; '2026-10-20T09:41:03 ERROR cache01 eviction storm' | Out-File -LiteralPath (Join-Path $PWD 'app.log') -Append -Encoding utf8; Wait-Property $w Counts text '29 lines, 7 errors'; Invoke-GuiTestEvent -Window $w.Id -Name Views -Event Change -Value 2; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Expand -Value 3; while (@((Get-GuiSnapshot -Window $w.Id).State.Sources.shown).Count -lt 5) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Expand -Value 4; while (@((Get-GuiSnapshot -Window $w.Id).State.Sources.shown).Count -lt 6) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Sources -Event Select -Value 5; Wait-Property $w Chosen text 'ERROR from cache01 at 09:41:03: eviction storm'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'charts-and-trees-loglens.png'); Invoke-GuiTestEvent -Window $w.Id -Name Views -Event Change -Value 0; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 28; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Activate -Value 28; Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log | Format-Table Time, Level, Source, Message
```

```text
Time                  Level Source  Message
----                  ----- ------  -------
10/20/2026 9:41:03 AM ERROR cache01 eviction storm
```

![The LogLens window with its Edit menu, the Sources tab chosen, the tree with cache01 open to its ERROR and its one line selected, the text ERROR from cache01 at 09:41:03: eviction storm beside it, a Close button, and along the bottom a status bar reading 29 lines, 7 errors](/images/charts-and-trees-loglens.png)

## Next

[A Page Of Your Own](A-Page-Of-Your-Own.md): a summary of the log as an HTML page of LogLens's own, which sends the source a person clicks back to the script.
