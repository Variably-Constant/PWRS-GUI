---
title: Renderers, Modes And Systems
weight: 11
---

Every chapter so far ran LogLens under PWRS-GUI's defaults. This chapter runs the same script, unchanged, drawn by the HTML renderer, in PowerShell's own process, and in a theme and fonts chosen for it, and then says on which systems it runs. Each choice is made outside the script, with [Set-GuiOption](Set-GuiOption.md), and [Get-GuiOption](Get-GuiOption.md) says what is in use. Source: `crates/pwrs-gui/src/options.rs`.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows.

<details>
<summary>The log from chapter 1 and LogLens.ps1 from chapter 9, written again so this page's examples have them</summary>

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
```

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

## The options it runs under

Every chapter so far ran LogLens as PWRS-GUI runs a script by default: drawn by the egui renderer, in a helper process the module starts, in the system's light or dark theme, and in the fonts the module brings. `Get-GuiOption` shows those choices:

```powershell
Get-GuiOption | Select-Object Mode, Renderer, Theme, Font
```

```text
Mode Renderer  Theme    Font
---- --------  -----    ----
Auto     Egui System Bundled
```

The options belong to the PowerShell session, not to a script: every window opened after a `Set-GuiOption` takes them, LogLens's among them, and LogLens names none of them itself.

## Drawn by HTML

`Set-GuiOption -Renderer Html` has the HTML renderer draw the windows a script describes: the system's webview, WebView2 on Windows, laying out the same widgets in the same colors, spacing and fonts as egui. Run LogLens, choose Heat and press Close:

<!-- keep: where-it-runs-html.png images/where-it-runs-html.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Views -Event Change -Value 4; do { $shot = Get-GuiSnapshot -Window $w.Id -Name Heat -Pixels } until ([System.Collections.Generic.HashSet[byte]]::new([byte[]]$shot.Pixels).Count -gt 4); Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'where-it-runs-html.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
Set-GuiOption -Renderer Html
.\LogLens.ps1 -Path .\app.log
Get-GuiOption | Select-Object Renderer, Running, Backend, WebGpu
Set-GuiOption -Renderer Egui
```

```text
Renderer Running Backend  WebGpu
-------- ------- -------  ------
    Html Helper  WebView2 WebGPU
```

![LogLens drawn by the HTML renderer in WebView2, its Heat tab chosen: the rows for db01, web01 and web02 beside the minutes from 09:30 to 09:39, drawn by the page's WebGPU, web02's at 09:33 the brightest, and the same menus, tabs, Close button and status bar as the egui window](/images/where-it-runs-html.png)

The page draws the heat map with its engine's WebGPU, which `WebGpu` names, running chapter 9's shader as written. The counting still runs on the egui renderer's GPU, so the page draws from a copy of the counts the module sends it.

## In the PowerShell process

On Windows a renderer can also run in PowerShell's own process, on a thread of the module's, with no helper: where AppLocker's rules refuse a helper, PWRS-GUI draws there by itself. A renderer goes on with the options it started with, so this example first ends the session, by removing the module, and imports it again. A module removed and imported again still has the options set before, so the example sets each one it needs:

```powershell
Remove-Module PWRS-GUI
Import-Module PWRS-GUI
Set-GuiOption -Mode InProcess -Renderer Egui
```

Run LogLens and press Close:

<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log
Get-GuiOption | Select-Object Mode, Renderer, Running
```

```text
     Mode Renderer Running
     ---- -------- -------
InProcess     Egui InProcess
```

The window is the same window, and LogLens cannot tell where it is drawn. [How To Choose Where Windows Are Drawn](How-To-Choose-Where-Windows-Are-Drawn.md) shows the helper process itself, the GPU and graphics API a renderer draws with, and a webview's data.

## A theme and fonts of its own

`-Theme Light` or `-Theme Dark` draws every window in that theme whatever the system's is, and `-Font System` draws text in the system's own fonts, Segoe UI on Windows, in place of the Inter that PWRS-GUI brings. The fonts are read as a renderer starts, so this example ends the session again first:

```powershell
Remove-Module PWRS-GUI
Import-Module PWRS-GUI
Set-GuiOption -Mode Auto -Renderer Egui -Theme Light -Font System
```

Run LogLens, select the line of the deadlock and press Close:

<!-- keep: where-it-runs-light.png images/where-it-runs-light.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 6; Wait-Property $w Details text 'ERROR from db01 at 09:31:30: deadlock on table orders'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'where-it-runs-light.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log
Get-GuiOption | Select-Object Mode, Running, Theme, Font
Set-GuiOption -Theme System -Font Bundled
```

```text
Mode Running Theme   Font
---- ------- -----   ----
Auto Helper  Light System
```

![LogLens in the light theme with text in Segoe UI: the Lines tab with the find field, the level drop-down and the table of the log's lines, the deadlock line selected and shown under the table, a Close button, and a status bar reading 28 lines, 6 errors](/images/where-it-runs-light.png)

## On each system

PWRS-GUI runs on Windows x64, in PowerShell 7.4 and later and in Windows PowerShell 5.1, and in PowerShell 7.4 and later on macOS on Apple silicon and on Linux and FreeBSD x64, under X11 or Wayland. LogLens names no path, renderer or system of its own, and egui draws all of it on each:

| System | egui draws with | HTML is drawn by | In PowerShell's process |
|---|---|---|---|
| Windows | DirectX 12, then Vulkan, then OpenGL | WebView2 | yes |
| macOS | Metal, from `PWRS-GUI.app` | WebKit, from macOS 15.8's | no, a helper only |
| Linux | Vulkan, then OpenGL | WebKitGTK 2.46 or later, the system's or an add-on's | no, a helper only |
| FreeBSD | Vulkan, then OpenGL | WebKitGTK 2.46 or later, the system's or an add-on's | no, a helper only |

In the HTML renderer the heat map needs WebGPU, since its shader reads a storage buffer, which WebGL 2 does not offer a page. WebView2 has WebGPU, and WebKit has it from macOS 26. Where a page has WebGL 2 alone, as in WebKitGTK, `Set-GuiGpuSurface` refuses the heat map's draw with `GuiGpuNoWebGpu`, naming `-Renderer Egui`, and the rest of LogLens draws as it does here.

[Tested Platforms](Tested-Platforms.md) lists the machines and versions the module and its tests have run on, and what each run showed.

## The whole script so far

`LogLens.ps1` is the script at the top of this page, as chapter 9 left it, and its tests are chapter 10's. Nothing in it names a renderer, a mode, a theme or a font, so whoever runs it chooses them, and the same script runs wherever PWRS-GUI does. That is the end of the series: the [reference](../reference/) has a page for every command and widget LogLens used and those it did not, and the [how-to guides](../how-to/) go further into each part.
