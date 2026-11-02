---
title: Testing LogLens
weight: 10
---

Chapter 9's LogLens is a window a person drives. This chapter tests it the way that person uses it: each test starts LogLens on a log of its own, finds its window, acts on it with [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md), reads back what the person would see with [Get-GuiSnapshot](Get-GuiSnapshot.md), and checks what the script wrote. It is the pattern of [How To Test A Window](How-To-Test-A-Window.md), applied to a whole app, and the last section puts the tests in a Pester file. Source: `crates/pwrs-gui/src/inspect.rs` for the commands.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own, with Pester 6.2.0 in both. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. No person clicked anything: every click on this page is the tests' own.

<details>
<summary>LogLens.ps1 from chapter 9, written again so this page's examples have it</summary>

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

function Get-Minutes {
    if ($entries.Count) { [int][math]::Floor(($entries[-1].Time - $start).TotalMinutes) + 1 } else { 1 }
}

function Get-NameLabels {
    $names = @(for ($row = 0; $row -lt $lanes.Count; $row++) { @{ Kind = 'Text'; X = 0; Y = ($row + 0.5) * 160 / $lanes.Count - 8; Text = $lanes[$row]; Size = 13 } })
    @{ Kind = 'Group'; Name = 'Labels'; Children = $names }
}

function Get-MinuteLabels([int] $Minutes) {
    $marks = @(for ($m = 0; $m -lt $Minutes; $m++) { @{ Kind = 'Text'; X = ($m + 0.5) * 720 / $Minutes - 15; Y = 2; Text = $start.AddMinutes($m).ToString('HH:mm'); Size = 11; Opacity = 0.6 } })
    @{ Kind = 'Group'; Name = 'Labels'; Children = $marks }
}

function Update-Heat {
    if (-not $entries.Count) { return }
    $minutes = Get-Minutes
    $cells = [uint32[]]@(foreach ($e in $entries) { $lanes.IndexOf($e.Source) * $minutes + [int][math]::Floor(($e.Time - $start).TotalMinutes) })
    $counts = New-GuiGpuBuffer -Length ($lanes.Count * $minutes) -Type UInt32
    Invoke-GuiGpuCompute $count @{ cells = $cells; counts = $counts } -Invocations $cells.Count
    $perCell = Read-GuiGpuBuffer $counts
    $top = ($perCell | Measure-Object -Maximum).Maximum
    Set-GuiGpuSurface Heat $heat -VertexCount 3 -Binding @{ counts = $counts } -Uniform ([float[]]($minutes, $lanes.Count, $top))
    if ($map.Counts) { Remove-GuiGpuBuffer $map.Counts }
    $map.Counts = $counts
    $map.PerCell = $perCell
    $map.Minutes = $minutes
    $map.Rows = $lanes.Count
    Remove-GuiItem -Name Names -Item Labels
    Add-GuiItem -Name Names -Item (Get-NameLabels)
    Remove-GuiItem -Name Minutes -Item Labels
    Add-GuiItem -Name Minutes -Item (Get-MinuteLabels $minutes)
}

function Get-Cell($At) {
    $minute = [math]::Min([int][math]::Floor($At.U * $map.Minutes), $map.Minutes - 1)
    $row = [math]::Min([int][math]::Floor($At.V * $map.Rows), $map.Rows - 1)
    [pscustomobject]@{ Source = $lanes[$row]; Minute = $minute; Count = $map.PerCell[$row * $map.Minutes + $minute] }
}

function Show-Cell($SurfaceEvent) {
    $cell = Get-Cell $SurfaceEvent.Value
    $said = if ($cell.Count -eq 1) { '1 line' } else { "$($cell.Count) lines" }
    Update-GuiWidget -Name Cell -Property @{ Text = "$($cell.Source) at $($start.AddMinutes($cell.Minute).ToString('HH:mm')): $said" }
}

function Get-CellLines($SurfaceEvent) {
    $cell = Get-Cell $SurfaceEvent.Value
    $entries | Where-Object { $_.Source -eq $cell.Source -and [int][math]::Floor(($_.Time - $start).TotalMinutes) -eq $cell.Minute }
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
        if ($WindowEvent.State.Views -eq 'Heat') { Update-Heat }
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
$count = @'
@group(0) @binding(0) var<storage, read> cells: array<u32>;
@group(0) @binding(1) var<storage, read_write> counts: array<atomic<u32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&cells)) { atomicAdd(&counts[cells[id.x]], 1u); }
}
'@
$heat = @'
struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, dark: f32, scale: f32, view: mat4x4<f32>, projection: mat4x4<f32>, minutes: f32, sources: f32, top: f32 }
@group(1) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(0) var<storage, read> counts: array<u32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let dark = frame.dark > 0.5;
    let size = frame.resolution / vec2<f32>(frame.minutes, frame.sources);
    let cell = min(floor(at.xy / size), vec2<f32>(frame.minutes - 1.0, frame.sources - 1.0));
    let inside = at.xy - cell * size;
    if (inside.x < 2.0 * frame.scale || inside.y < 2.0 * frame.scale) {
        return vec4<f32>(select(vec3<f32>(0.973), vec3<f32>(0.106), dark), 1.0);
    }
    let n = counts[u32(cell.y) * u32(frame.minutes) + u32(cell.x)];
    if (n == 0u) {
        return vec4<f32>(select(vec3<f32>(0.882, 0.878, 0.851), vec3<f32>(0.173, 0.173, 0.165), dark), 1.0);
    }
    var light = array<vec3<f32>, 6>(vec3<f32>(0.620, 0.773, 0.957), vec3<f32>(0.427, 0.655, 0.925), vec3<f32>(0.224, 0.529, 0.898), vec3<f32>(0.145, 0.416, 0.749), vec3<f32>(0.094, 0.310, 0.584), vec3<f32>(0.051, 0.212, 0.420));
    var night = array<vec3<f32>, 6>(vec3<f32>(0.094, 0.310, 0.584), vec3<f32>(0.145, 0.416, 0.749), vec3<f32>(0.224, 0.529, 0.898), vec3<f32>(0.427, 0.655, 0.925), vec3<f32>(0.620, 0.773, 0.957), vec3<f32>(0.804, 0.886, 0.984));
    let t = (f32(n) - 1.0) / max(frame.top - 1.0, 1.0) * 5.0;
    let low = u32(floor(t));
    let high = min(low + 1u, 5u);
    var color = mix(light[low], light[high], fract(t));
    if (dark) { color = mix(night[low], night[high], fract(t)); }
    return vec4<f32>(color, 1.0);
}
'@
$map = @{ Counts = $null; PerCell = $null; Minutes = 0; Rows = 0 }
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
    Tabs -Name Views -OnChange { if ($_.Value -eq 'Timeline') { Set-GuiCanvasView Timeline -Fit } elseif ($_.Value -eq 'Heat') { Update-Heat } } {
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
        Tab 'Heat' {
            Grid -Columns 2 {
                Canvas -Name Names -Width 60 -Height 160 -Item (Get-NameLabels)
                GpuSurface Heat -Width 720 -Height 160 -OnPointerMove { Show-Cell $_ } -OnPointerDown { Get-CellLines $_ }
                Text ''
                Canvas -Name Minutes -Width 720 -Height 20 -Item (Get-MinuteLabels (Get-Minutes))
            }
            Text 'Point at a cell to see its count, and click it for its lines' -Name Cell
        }
    }
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
    StatusBar { Text (Get-Counts) -Name Counts }
}
$out | Select-Object -SkipLast 1
```

</details>

## Starting LogLens from a test

LogLens waits until its window closes, so a test starts it in a second runspace of the same process, as How To Test A Window starts a dialog, and finds the window it opened, since every runspace of the process shares one session. These helpers write the log a test runs LogLens on, start LogLens on it, wait for its window, and take what LogLens wrote, stopping it when it has not returned. Each wait has a deadline, so a window that never opens fails the test instead of stalling it:

```powershell
Import-Module PWRS-GUI
$sample = @'
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

function Start-LogLens([string] $Script, [string] $Log) {
    Set-Content -LiteralPath $Log -Value $sample
    $ps = [powershell]::Create().AddCommand($Script).AddParameter('Path', $Log)
    [pscustomobject]@{ PS = $ps; Async = $ps.BeginInvoke() }
}

function Wait-Window([string] $Title, [int] $Seconds = 30) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $window = Get-GuiWindow -Title $Title
        if ($window) { return $window }
        Start-Sleep -Milliseconds 50
    }
    throw "no window titled '$Title' within $Seconds s"
}

function Complete-LogLens($Run, [int] $Seconds = 20) {
    if (-not $Run.Async.AsyncWaitHandle.WaitOne($Seconds * 1000)) {
        $Run.PS.Stop()
        throw "LogLens did not return within $Seconds s; it was stopped"
    }
    $output = $Run.PS.EndInvoke($Run.Async)
    $Run.PS.Dispose()
    $Run.PS = $null
    $output
}

function Find-Node($Node, [string] $Name) {
    if ($Node.Name -eq $Name) { return $Node }
    foreach ($child in $Node.Children) {
        $found = Find-Node $child $Name
        if ($found) { return $found }
    }
}

function Get-WidgetText($Window, [string] $Name) {
    (Find-Node (Get-GuiSnapshot -Window $Window.Id).Tree $Name).Properties.text
}

function Wait-Until([scriptblock] $Condition, [string] $What, [int] $Seconds = 10) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not (& $Condition)) {
        if ([DateTime]::UtcNow -ge $deadline) { throw "$What did not happen within $Seconds s" }
        Start-Sleep -Milliseconds 50
    }
}
```

`Start-LogLens` gives LogLens the full paths of the script and the log, since the second runspace does not start in this one's folder. Start LogLens, list the widgets [Get-GuiWindow](Get-GuiWindow.md) finds in its window by name, and close it as a person would:

```powershell
$loglens = Join-Path $PWD 'LogLens.ps1'
$log = Join-Path $PWD 'app.log'
$run = Start-LogLens $loglens $log
$window = Wait-Window 'LogLens'
$window.Names -join ', '
Invoke-GuiTestEvent -Window $window.Id -Name Close -Event Click
$null -eq (Complete-LogLens $run)
```

```text
Find, Summary, Views, Filter, Level, Lines, Details, Errors, Sources, Chosen, Timeline, Picked, Names, Heat, Minutes, Cell, Close, Counts
True
```

## Acting as a person

A test acts through those names and waits for what a person would wait for. Typing in the filter runs LogLens's own handler in its runspace, which changes the table's rows a moment later, so the test waits until the table shows the rows it expects before it selects one; the table's `State` gives how many it shows. Then it reads the line shown under the table, presses Ctrl+F and finds the keyboard in the filter, and presses Enter on the row, which LogLens writes as it ends:

```powershell
$run = Start-LogLens $loglens $log
$window = Wait-Window 'LogLens'
Invoke-GuiTestEvent -Window $window.Id -Name Filter -Event Change -Value 'deadlock'
Wait-Until { (Get-GuiSnapshot -Window $window.Id).State.Lines.rows -eq 1 } 'the table showing one row'
Invoke-GuiTestEvent -Window $window.Id -Name Lines -Event Select -Value 0
Wait-Until { (Get-WidgetText $window Details) -ne 'Select a line to see it here' } 'the line shown under the table'
Get-WidgetText $window Details
Invoke-GuiTestEvent -Window $window.Id -Shortcut Ctrl+F
Wait-Until { (Get-GuiSnapshot -Window $window.Id).Focused -eq 'Filter' } 'the keyboard in the filter'
(Get-GuiSnapshot -Window $window.Id).Focused
Invoke-GuiTestEvent -Window $window.Id -Name Lines -Event Activate -Value 0
Invoke-GuiTestEvent -Window $window.Id -Name Close -Event Click
Complete-LogLens $run | Format-Table Time, Level, Source, Message
```

```text
ERROR from db01 at 09:31:30: deadlock on table orders
Filter

Time                  Level Source Message
----                  ----- ------ -------
10/20/2026 9:31:30 AM ERROR db01   deadlock on table orders
```

`Invoke-GuiTestEvent -Shortcut` presses the window's shortcut as a person's keys would, and the snapshot's `Focused` names the widget that has the keyboard, so the test checks what Edit, Find did rather than how it did it.

## A log that grows

LogLens follows its log, so a test can stand for the service that writes it. It adds a line with `Out-File -Append`, which shares the file with LogLens's reads as a service's writer would, and waits for the status bar to count it:

```powershell
$run = Start-LogLens $loglens $log
$window = Wait-Window 'LogLens'
Get-WidgetText $window Counts
'2026-10-20T09:41:03 ERROR cache01 eviction storm' | Out-File -LiteralPath $log -Append -Encoding utf8
Wait-Until { (Get-WidgetText $window Counts) -ne '28 lines, 6 errors' } 'the status bar counting the new line'
Get-WidgetText $window Counts
Invoke-GuiTestEvent -Window $window.Id -Name Close -Event Click
$null = Complete-LogLens $run
```

```text
28 lines, 6 errors
29 lines, 7 errors
```

## Reading the heat map's pixels

No text in the window states the heat map's colors, so a test reads them from the pixels: `Get-GuiSnapshot -Name Heat -Pixels` gives what the surface drew, four bytes a pixel, and a cell's middle is found from its minute and row, since the cells divide the surface. The colors follow the window's theme, so the test fixes the theme with [Set-GuiOption](Set-GuiOption.md), which chapter 11 covers, and puts it back after. Until its first draw a surface shows its window's background, so the test waits until the cell is drawn. [Save-GuiGpuSurface](Save-GuiGpuSurface.md) writes what the surface drew to a PNG, which a test that fails can leave beside its report:

<!-- keep: testing-it-heat.png images/testing-it-heat.png -->
```powershell
function Get-CellColor($Window, [int] $Minute, [int] $Row, [int] $Minutes, [int] $Rows) {
    $shot = Get-GuiSnapshot -Window $Window.Id -Name Heat -Pixels
    $x = [int][math]::Floor(($Minute + 0.5) / $Minutes * $shot.Width)
    $y = [int][math]::Floor(($Row + 0.5) / $Rows * $shot.Height)
    $at = ($y * $shot.Width + $x) * 4
    $shot.Pixels[$at..($at + 2)] -join ' '
}

Set-GuiOption -Theme Light
$run = Start-LogLens $loglens $log
$window = Wait-Window 'LogLens'
Invoke-GuiTestEvent -Window $window.Id -Name Views -Event Change -Value 4
Wait-Until { (Get-CellColor $window 3 2 10 3) -ne '248 248 248' } 'the heat map drawn'
"web02 at 09:33, the busiest: $(Get-CellColor $window 3 2 10 3)"
"web02 at 09:32, empty:       $(Get-CellColor $window 2 2 10 3)"
Save-GuiGpuSurface Heat (Join-Path $PWD 'testing-it-heat.png') -Window $window.Id
Invoke-GuiTestEvent -Window $window.Id -Name Close -Event Click
$null = Complete-LogLens $run
Set-GuiOption -Theme System
```

```text
web02 at 09:33, the busiest: 13 54 107
web02 at 09:32, empty:       225 224 217
```

![The heat map LogLens drew in a light window, as Save-GuiGpuSurface saved it: three rows of ten cells, web02's at 09:33 in the darkest blue, the counts of one and two in lighter blues, and the empty cells a pale gray](/images/testing-it-heat.png)

The busiest cell is the last step of chapter 9's light blues, 13, 54 and 107, and an empty cell the gray of 225, 224 and 217, as the shader's colors say.

## The whole script so far

`LogLens.ps1` is as chapter 9 left it. Its tests are a Pester file beside it, `LogLens.Tests.ps1`, with the helpers in `BeforeAll`. Each test starts LogLens on a log in Pester's `TestDrive` folder, which Pester makes for the file and removes after it, and an `AfterEach` closes any window a test left open, so a failing test does not leave the next one a window of its own. The pixel tests fix the theme around them, in a `Context` of their own. Pester 5 or later is needed, which How To Test A Window says how to install in Windows PowerShell 5.1:

<!-- file: LogLens.Tests.ps1 -->
```powershell
BeforeAll {
    Import-Module PWRS-GUI
    $script:LogLens = Join-Path $PSScriptRoot 'LogLens.ps1'
    $script:Sample = @'
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

    function Start-LogLens([string] $Log) {
        Set-Content -LiteralPath $Log -Value $script:Sample
        $ps = [powershell]::Create().AddCommand($script:LogLens).AddParameter('Path', $Log)
        [pscustomobject]@{ PS = $ps; Async = $ps.BeginInvoke() }
    }

    function Wait-Window([string] $Title, [int] $Seconds = 30) {
        $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
        while ([DateTime]::UtcNow -lt $deadline) {
            $window = Get-GuiWindow -Title $Title
            if ($window) { return $window }
            Start-Sleep -Milliseconds 50
        }
        throw "no window titled '$Title' within $Seconds s"
    }

    function Complete-LogLens($Run, [int] $Seconds = 20) {
        if (-not $Run.Async.AsyncWaitHandle.WaitOne($Seconds * 1000)) {
            $Run.PS.Stop()
            throw "LogLens did not return within $Seconds s; it was stopped"
        }
        $output = $Run.PS.EndInvoke($Run.Async)
        $Run.PS.Dispose()
        $Run.PS = $null
        $output
    }

    function Find-Node($Node, [string] $Name) {
        if ($Node.Name -eq $Name) { return $Node }
        foreach ($child in $Node.Children) {
            $found = Find-Node $child $Name
            if ($found) { return $found }
        }
    }

    function Get-WidgetText($Window, [string] $Name) {
        (Find-Node (Get-GuiSnapshot -Window $Window.Id).Tree $Name).Properties.text
    }

    function Wait-Until([scriptblock] $Condition, [string] $What, [int] $Seconds = 10) {
        $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
        while (-not (& $Condition)) {
            if ([DateTime]::UtcNow -ge $deadline) { throw "$What did not happen within $Seconds s" }
            Start-Sleep -Milliseconds 50
        }
    }

    function Get-CellColor($Window, [int] $Minute, [int] $Row, [int] $Minutes, [int] $Rows) {
        $shot = Get-GuiSnapshot -Window $Window.Id -Name Heat -Pixels
        $x = [int][math]::Floor(($Minute + 0.5) / $Minutes * $shot.Width)
        $y = [int][math]::Floor(($Row + 0.5) / $Rows * $shot.Height)
        $at = ($y * $shot.Width + $x) * 4
        $shot.Pixels[$at..($at + 2)] -join ' '
    }
}

Describe 'LogLens' {
    BeforeEach {
        $log = Join-Path $TestDrive 'app.log'
        $run = Start-LogLens $log
        $window = Wait-Window 'LogLens'
    }

    AfterEach {
        if (Get-GuiWindow -Title 'LogLens') { Invoke-GuiTestEvent -Window $window.Id -Name Close -Event Click }
        if ($run.PS) { $null = Complete-LogLens $run }
    }

    It 'shows only the lines with the text a person types' {
        Invoke-GuiTestEvent -Window $window.Id -Name Filter -Event Change -Value 'orders'
        Wait-Until { (Get-GuiSnapshot -Window $window.Id).State.Lines.rows -eq 15 } 'the table showing the 15 lines with orders'
        Invoke-GuiTestEvent -Window $window.Id -Name Lines -Event Select -Value 0
        Wait-Until { (Get-WidgetText $window Details) -ne 'Select a line to see it here' } 'the line shown under the table'
        Get-WidgetText $window Details | Should -Be 'INFO from web01 at 09:30:15: GET /orders 200'
    }

    It 'puts the keyboard in the filter for Ctrl+F' {
        Invoke-GuiTestEvent -Window $window.Id -Shortcut Ctrl+F
        Wait-Until { (Get-GuiSnapshot -Window $window.Id).Focused -eq 'Filter' } 'the keyboard in the filter'
    }

    It 'writes the line a person opens with Enter' {
        Invoke-GuiTestEvent -Window $window.Id -Name Lines -Event Select -Value 6
        Invoke-GuiTestEvent -Window $window.Id -Name Lines -Event Activate -Value 6
        Invoke-GuiTestEvent -Window $window.Id -Name Close -Event Click
        $written = Complete-LogLens $run
        $written.Message | Should -Be 'deadlock on table orders'
    }

    It 'counts a line the log gains' {
        '2026-10-20T09:41:03 ERROR cache01 eviction storm' | Out-File -LiteralPath $log -Append -Encoding utf8
        Wait-Until { (Get-WidgetText $window Counts) -ne '28 lines, 6 errors' } 'the status bar counting the new line'
        Get-WidgetText $window Counts | Should -Be '29 lines, 7 errors'
    }

    Context 'in a light window' {
        BeforeAll { Set-GuiOption -Theme Light }
        AfterAll { Set-GuiOption -Theme System }

        It 'draws the busiest minute in the darkest blue and an empty one gray' {
            Invoke-GuiTestEvent -Window $window.Id -Name Views -Event Change -Value 4
            Wait-Until { (Get-CellColor $window 3 2 10 3) -ne '248 248 248' } 'the heat map drawn'
            Get-CellColor $window 3 2 10 3 | Should -Be '13 54 107'
            Get-CellColor $window 2 2 10 3 | Should -Be '225 224 217'
        }

        It 'names the cell under the pointer' {
            Invoke-GuiTestEvent -Window $window.Id -Name Views -Event Change -Value 4
            Wait-Until { (Get-CellColor $window 3 2 10 3) -ne '248 248 248' } 'the heat map drawn'
            Invoke-GuiTestEvent -Window $window.Id -Name Heat -Event PointerMove -Value @{ U = 0.35; V = 0.85 }
            Wait-Until { (Get-WidgetText $window Cell) -like 'web02*' } 'the cell named under the map'
            Get-WidgetText $window Cell | Should -Be 'web02 at 09:33: 4 lines'
        }
    }
}
```

Run it with Pester:

```powershell
Import-Module Pester -MinimumVersion 5.0
$result = Invoke-Pester -Path .\LogLens.Tests.ps1 -PassThru -Output None
$result.Result
$result.Tests | Select-Object ExpandedName, Result
```

```text
Passed

ExpandedName                                                       Result
------------                                                       ------
shows only the lines with the text a person types                  Passed
puts the keyboard in the filter for Ctrl+F                         Passed
writes the line a person opens with Enter                          Passed
counts a line the log gains                                        Passed
draws the busiest minute in the darkest blue and an empty one gray Passed
names the cell under the pointer                                   Passed
```

A wait is itself a check: `Wait-Until` throws when what it waits for has not happened by its deadline, which fails the test with a message naming it, so a test that fails says why instead of hanging the run. A test waits for the value it expects, not for any change, since LogLens changes the table in two steps, its rows taken away and then the matching ones added.

## Next

[Renderers, Modes And Systems](Renderers-Modes-And-Systems.md): the same script in the helper and in-process, drawn by egui and by HTML, on each system.
