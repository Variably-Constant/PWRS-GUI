---
title: Following A Log
weight: 5
---

Chapter 4's LogLens reads its log once, as it opens, but a service goes on writing its log while it runs. This chapter has LogLens follow the log: first with Get-Content -Wait, PowerShell's own way to follow a file, then with the window's `-WatchPath`, through which the system tells LogLens of each change. A log too big to read at a glance loads under a progress window a person can close, and LogLens gains a menu with Find under Edit. Source: a file's changes are read in `crates/pwrs-gui-client/src/file_watch.rs`, the windows `crates/pwrs-gui/src/window.rs`, and the progress window `crates/pwrs-gui/src/progress.rs`.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the keys and the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows. The lines added to a log were written from that runspace too, standing for the service that writes the log.

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

## Get-Content -Wait

`Get-Content -Wait` writes the lines a file has, then waits, looking at the file once a second and writing each line added to it. Piped into [Add-GuiItem](Add-GuiItem.md), it adds each line to a [LogView](New-GuiLogView.md) in a window that does not wait. The log's `-Width` and `-Height` give it its size, so the window opens with room for the lines still to come. `Select-Object -First 3` ends the pipeline at the third line; without it, the pipeline never ends. The log starts with one line, and two more are added while the window is open. Then press Close:

<!-- focus: its pictures show Find taking the keyboard to the field -->
<!-- keep: following-a-log-get-content.png images/following-a-log-get-content.png -->
<!-- person: $w = Wait-Window 'Following'; $log = Join-Path $PWD 'follow.log'; '2026-10-20T09:40:05 WARN web01 slow start' | Out-File -LiteralPath $log -Append -Encoding utf8; '2026-10-20T09:40:09 INFO web01 ready' | Out-File -LiteralPath $log -Append -Encoding utf8; Wait-Property $w Lines lines '2026-10-20T09:40:00 INFO web01 started 2026-10-20T09:40:05 WARN web01 slow start 2026-10-20T09:40:09 INFO web01 ready'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'following-a-log-get-content.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
Set-Content -Path follow.log -Value '2026-10-20T09:40:00 INFO web01 started'
$w = Show-GuiWindow -NoWait -Title 'Following' {
    LogView -Name Lines -Width 360 -Height 120
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
Get-Content -Path follow.log -Wait | Select-Object -First 3 | ForEach-Object {
    Add-GuiItem -Window $w.Id -Name Lines -Item $_
    $_
}
do { $e = Wait-GuiEvent -Window $w.Id } until ($e.Kind -eq 'Close')
```

```text
2026-10-20T09:40:00 INFO web01 started
2026-10-20T09:40:05 WARN web01 slow start
2026-10-20T09:40:09 INFO web01 ready
```

![The Following window with three lines of the log in a log area, and a Close button at the bottom right](/images/following-a-log-get-content.png)

Each line shows within a second of being written. While no line comes, though, the pipeline cannot see the window: a person who closes it before the third line leaves the pipeline waiting for that line, with nothing to show it in.

## The window follows the file

[Show-GuiWindow](Show-GuiWindow.md) `-WatchPath` names files whose changes the window reports while it is open. A change to one, such as a line added, raises a `FileChange` event, whose `Value` is the file's new length, and `-OnFileChange` runs with it. A window that waits then follows a log through a handler, as chapter 4's handlers act on the field and the table, and it closes as any window does. The system tells of each change; on Windows the window also looks at the file once a second, since NTFS reports no line a service leaves in the system's cache until it closes the file.

The handler reads what was added since it last read. `New-Tail` writes an object with the log's path and `Position`, how far into the log the reading has come. `Read-NewLines` opens the log, reads from there to the end and closes it again, so the log is open only for the moment it is read, and even then other programs may write, rename or remove it. It writes the lines it finds whole; the text after the last line break waits in `Rest` until its line is ended, since a writer may add a line in two writes. A log shorter than `Position`, as when a service empties it to start again, is read again from its start:

<!-- keep: following-a-log-watch.png images/following-a-log-watch.png -->
<!-- person: $w = Wait-Window 'Following'; $log = Join-Path $PWD 'follow.log'; '2026-10-20T09:40:05 WARN web01 slow start' | Out-File -LiteralPath $log -Append -Encoding utf8; '2026-10-20T09:40:09 INFO web01 ready' | Out-File -LiteralPath $log -Append -Encoding utf8; Wait-Property $w Lines lines '2026-10-20T09:40:00 INFO web01 started 2026-10-20T09:40:05 WARN web01 slow start 2026-10-20T09:40:09 INFO web01 ready'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'following-a-log-watch.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
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
Set-Content -Path follow.log -Value '2026-10-20T09:40:00 INFO web01 started'
$tail = New-Tail follow.log
$out = Show-GuiWindow -Title 'Following' -HandlerOutput Pipeline -WatchPath follow.log -OnFileChange {
    $new = @(Read-NewLines $tail)
    if ($new) { Add-GuiItem -Name Lines -Item $new; $new }
} {
    LogView @(Read-NewLines $tail) -Name Lines -Width 360 -Height 120
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$out | Select-Object -SkipLast 1
$out[-1].Result
```

```text
2026-10-20T09:40:05 WARN web01 slow start
2026-10-20T09:40:09 INFO web01 ready
Close
```

![The Following window with three lines of the log in a log area, and a Close button at the bottom right](/images/following-a-log-watch.png)

The handler wrote the two lines added, before the window's result. A file has one change waiting at a time: changes made while the handler runs bring one more run, which reads everything they added, so the handler misses nothing and never falls behind.

Windows PowerShell 5.1's `Add-Content` and `Set-Content` open a file so that no other program may read it meanwhile, and fail on a file another program has open. A Windows PowerShell 5.1 script writing the log with `Add-Content` and LogLens reading it then fail when both open the log at the same moment, with "The process cannot access the file ... because it is being used by another process". `Out-File -Append`, PowerShell 7's `Add-Content` and .NET's `StreamWriter` share the file and never fail so. [Windows PowerShell 5.1](Windows-PowerShell.md) lists the other ways it differs.

## A window that does not wait

A window opened with `-NoWait` gives its `FileChange` events to [Wait-GuiEvent](Wait-GuiEvent.md), as chapter 4's last example took the field's changes, so a script that does other work between events follows the log in its own loop:

<!-- keep: following-a-log-loop.png images/following-a-log-loop.png -->
<!-- person: $w = Wait-Window 'Following'; $log = Join-Path $PWD 'follow.log'; '2026-10-20T09:40:05 WARN web01 slow start' | Out-File -LiteralPath $log -Append -Encoding utf8; '2026-10-20T09:40:09 INFO web01 ready' | Out-File -LiteralPath $log -Append -Encoding utf8; Wait-Property $w Lines lines '2026-10-20T09:40:00 INFO web01 started 2026-10-20T09:40:05 WARN web01 slow start 2026-10-20T09:40:09 INFO web01 ready'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'following-a-log-loop.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
Set-Content -Path follow.log -Value '2026-10-20T09:40:00 INFO web01 started'
$tail = New-Tail follow.log
$w = Show-GuiWindow -NoWait -Title 'Following' -WatchPath follow.log {
    LogView @(Read-NewLines $tail) -Name Lines -Width 360 -Height 120
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
do {
    $e = Wait-GuiEvent -Window $w.Id
    if ($e.Kind -eq 'FileChange') {
        $new = @(Read-NewLines $tail)
        if ($new) { Add-GuiItem -Window $w.Id -Name Lines -Item $new; $new }
    }
} until ($e.Kind -eq 'Close')
```

```text
2026-10-20T09:40:05 WARN web01 slow start
2026-10-20T09:40:09 INFO web01 ready
```

![The Following window with three lines of the log in a log area, and a Close button at the bottom right](/images/following-a-log-loop.png)

## A big log

A log of many lines takes a while to read into objects, and a progress window shows how far the reading has come. [Show-GuiProgress](Show-GuiProgress.md) opens it and writes it once it has opened; [Set-GuiProgress](Set-GuiProgress.md) moves its bar. A person may close the window to stop, and the script reads the window's `State` as it goes, stopping once it reads `Closed`. Here the log has 200,000 lines, and the person closes the window part way:

<!-- keep: following-a-log-progress.png images/following-a-log-progress.png -->
<!-- person: $w = Wait-Window 'Reading big.log'; Wait-Above $w Progress value 0; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'following-a-log-progress.png'); Close-GuiWindow -Window $w.Id -->
```powershell
$lines = foreach ($i in 1..200000) { "2026-10-20T10:00:00 INFO web01 request $i" }
Set-Content -Path big.log -Value $lines
$read = @(Get-Content -Path big.log)
$p = Show-GuiProgress -Activity 'Reading big.log' -Status "0 of $($read.Count) lines"
$loaded = [System.Collections.Generic.List[object]]::new()
foreach ($line in $read) {
    $loaded.Add(($line | ConvertFrom-LogLine))
    if ($loaded.Count % 10000 -eq 0) {
        if ($p.State -eq 'Closed') { break }
        Set-GuiProgress $p -Status "$($loaded.Count) of $($read.Count) lines" -PercentComplete ($loaded.Count * 100 / $read.Count)
    }
}
if ($p.State -eq 'Closed') { 'stopped before the end' } else { Set-GuiProgress $p -Completed; "$($loaded.Count) lines read" }
```

```text
stopped before the end
```

![The Reading big.log window, its status giving how many of the 200000 lines have been read with the percentage done at its right, over a thin bar part way along](/images/following-a-log-progress.png)

The script looks at `State` every 10,000 lines, so it stops within 10,000 lines of the close. Read to the end, the window closes with `Set-GuiProgress -Completed`.

## A menu

A [MenuBar](New-GuiMenuBar.md) puts menus along the window's top. A [Menu](New-GuiMenu.md) has [MenuItem](New-GuiMenuItem.md) widgets, and an item's `-Shortcut` is the keys that choose it, shown beside it. LogLens puts Find under Edit, as desktop programs do, its shortcut Ctrl+F, in place of chapter 4's `-OnShortcut`. Press Ctrl+F:

<!-- keep: following-a-log-menu.png images/following-a-log-menu.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+F'; Wait-Focused $w Filter; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'following-a-log-menu.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 {
    MenuBar { Menu 'Edit' { MenuItem 'Find' -Name Find -Shortcut Ctrl+F -OnClick { Set-GuiFocus Filter } } }
    TextBox -Name Filter -Placeholder 'text to find'
    Table $entries -Name Lines
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$result.Result
```

```text
Close
```

![The LogLens window with an Edit menu along its top, a field above the table of the log's lines, and a Close button at the bottom right](/images/following-a-log-menu.png)

The menu bar is drawn along the window's top wherever the block writes it. Ctrl+F chooses Find whether or not the menu is open, and from the field too, which leaves the shortcut to the window.

## The whole script so far

`LogLens.ps1` now reads its log with `Read-NewLines`, under a progress window when the log has more than `-Large` lines, and closing that window stops it. Its window reports each change to the log, and `Add-NewEntries` adds each new line to the entries, and to the table when it matches the field and the drop-down. `Test-Matching` gives that one test to both, and Find is under Edit:

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

function Add-NewEntries($WindowEvent) {
    foreach ($entry in @(Read-NewLines $tail | ConvertFrom-LogLine)) {
        $entries.Add($entry)
        if (Test-Matching $entry $WindowEvent.State) {
            Add-GuiItem -Window $WindowEvent.Window -Name Lines -Item $entry
            $shown.Add($entry)
        }
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
$details = {
    $e = $_.Value
    Update-GuiWidget -Name Details -Property @{ Text = "$($e.Level) from $($e.Source) at $($e.Time.ToString('HH:mm:ss')): $($e.Message)" }
}
$out = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 -HandlerOutput Pipeline -WatchPath $Path -OnFileChange { Add-NewEntries $_ } {
    MenuBar { Menu 'Edit' { MenuItem 'Find' -Name Find -Shortcut Ctrl+F -OnClick { Set-GuiFocus Filter } } }
    Row {
        Text 'Find'
        TextBox -Name Filter -Placeholder 'text to find' -OnChange { Show-Matching $_ }
        Text 'Level'
        ComboBox 'All', 'INFO', 'WARN', 'ERROR' -Name Level -Selected All -OnChange { Show-Matching $_ }
    }
    Table $entries -Name Lines -OnSelect $details -OnActivate { $_.Value }
    Text 'Select a line to see it here' -Name Details
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$out | Select-Object -SkipLast 1
```

Run it, type `replica`, and once the table shows the log's three lines about replica 2, a fourth arrives as the log grows. Select it, press Enter, then press Close:

<!-- keep: following-a-log-loglens.png images/following-a-log-loglens.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+F'; Wait-Focused $w Filter; Invoke-GuiTestEvent -Window $w.Id -Name Filter -Event Change -Value 'replica'; while ((Get-GuiSnapshot -Window $w.Id).State.Lines.rows -ne 3) { Start-Sleep -Milliseconds 50 }; '2026-10-20T09:40:21 WARN db01 replica 2 behind by 12 s' | Out-File -LiteralPath (Join-Path $PWD 'app.log') -Append -Encoding utf8; while ((Get-GuiSnapshot -Window $w.Id).State.Lines.rows -ne 4) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 3; Wait-Property $w Details text 'WARN from db01 at 09:40:21: replica 2 behind by 12 s'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'following-a-log-loglens.png'); Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Activate -Value 3; Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log | Format-Table Time, Level, Source, Message
```

```text
Time                  Level Source Message
----                  ----- ------ -------
10/20/2026 9:40:21 AM WARN  db01   replica 2 behind by 12 s
```

![The LogLens window with its Edit menu, replica typed in the field, the four lines about replica 2 in the table, the newest selected, and under the table the text WARN from db01 at 09:40:21: replica 2 behind by 12 s](/images/following-a-log-loglens.png)

## Next

[Charts And Trees](Charts-And-Trees.md): errors per minute as a chart, the entries grouped by source and level in a tree, and the views on tabs.
