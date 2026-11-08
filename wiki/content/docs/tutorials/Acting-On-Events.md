---
title: Acting On Events
weight: 4
---

Chapter 3's LogLens filters only once Find is pressed and the window has closed. This chapter makes it act while a person works in it. The table follows the field and the drop-down as a person types and picks, the selected line shows in full under the table, Ctrl+F takes the keyboard to the field, and Enter or a double click writes a line out. At the end the window stays open while the script goes on, reading the window's events itself. Source: the handlers are run by `crates/pwrs-gui/src/window.rs`, and the events are raised by the renderers.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the typing, the keys and the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows; a person's give the same output.

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

## Follow the field

A handler is a script block a widget runs when it raises an event: `-OnChange` for a field or a drop-down, `-OnSelect` for a table, `-OnClick` for a button. It is given the event as `$_`: the window's number in `Window`, the widget's `Name`, the event's `Kind` and `Value`, and in `State` every named input's value at that moment.

A table's rows change while its window is open through [Add-GuiItem](Add-GuiItem.md) and [Remove-GuiItem](Remove-GuiItem.md), so a handler can make the table follow the field: it takes away the rows shown and adds the lines that match. Type `orders`, pick ERROR, and close the window:

<!-- focus: its pictures show Ctrl+F taking the keyboard to the field -->
<!-- keep: acting-on-events-follow.png images/acting-on-events-follow.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Filter -Event Change -Value 'orders'; Invoke-GuiTestEvent -Window $w.Id -Name Level -Event Change -Value 3; while ((Get-GuiSnapshot -Window $w.Id).State.Lines.rows -ne 5) { Start-Sleep -Milliseconds 50 }; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'acting-on-events-follow.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$shown = [System.Collections.Generic.List[object]]::new($entries)
function Show-Matching($WindowEvent) {
    $state = $WindowEvent.State
    $matching = @($entries | Where-Object {
        ($state.Level -eq 'All' -or $_.Level -eq $state.Level) -and $_.Message -like "*$($state.Filter)*"
    })
    if ($shown.Count) { Remove-GuiItem -Window $WindowEvent.Window -Name Lines -Item $shown.ToArray() }
    $shown.Clear()
    if ($matching.Count) {
        Add-GuiItem -Window $WindowEvent.Window -Name Lines -Item $matching
        $shown.AddRange($matching)
    }
}
$result = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 {
    Row {
        Text 'Find'
        TextBox -Name Filter -Placeholder 'text to find' -OnChange { Show-Matching $_ }
        Text 'Level'
        ComboBox 'All', 'INFO', 'WARN', 'ERROR' -Name Level -Selected All -OnChange { Show-Matching $_ }
    }
    Table $entries -Name Lines
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$shown | Format-Table Level, Source, Message
```

```text
Level Source Message
----- ------ -------
ERROR db01   deadlock on table orders
ERROR web01  GET /orders/17 500
ERROR web02  POST /orders 503
ERROR web01  GET /orders 500
ERROR web02  GET /orders 500
```

![The LogLens window with orders typed in the field and ERROR picked in the drop-down, above a table of the five ERROR lines about orders, and a Close button at the bottom right](/images/acting-on-events-follow.png)

Each edit of the field raises `Change`, and so does each pick in the drop-down, so the table changes as the person types. `$shown` lists the lines the table shows, which the next change takes away. A table given `-Filter` has a field of its own above its rows, which only hides the rows that do not match what is typed there; Find and Level change which rows the table has, and `$shown` lists them for the script. The handlers run on the thread that waits for the window, while the window is open, so the rows change in the window the person is looking at.

## Show the selected line

A table raises `Select` when a row is selected, with the row's object as its `Value`, and [Update-GuiWidget](Update-GuiWidget.md) changes a widget's properties while its window is open. Together they show the selected line in full under the table. Select the line about the disk:

<!-- keep: acting-on-events-details.png images/acting-on-events-details.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 18; Wait-Property $w Details text 'ERROR from db01 at 09:35:57: disk 92% full'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'acting-on-events-details.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 {
    Table $entries -Name Lines -OnSelect {
        $e = $_.Value
        Update-GuiWidget -Name Details -Property @{ Text = "$($e.Level) from $($e.Source) at $($e.Time.ToString('HH:mm:ss')): $($e.Message)" }
    }
    Text 'Select a line to see it here' -Name Details
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$result.State.Lines.Message
```

```text
disk 92% full
```

![The LogLens window with the line about the disk selected in the table, and under the table the text ERROR from db01 at 09:35:57: disk 92% full](/images/acting-on-events-details.png)

Inside a handler, the update commands act on the handler's own window, so `-Window` can be left out, as here. `Show-Matching` names the window all the same, so the last example on this page can call it where no handler runs.

## Ctrl+F and Enter

A window's own shortcuts come from `Show-GuiWindow -OnShortcut`, a hashtable from each shortcut to its handler, and [Set-GuiFocus](Set-GuiFocus.md) gives a widget the keyboard focus, as a click on it does; Ctrl+F then takes a person to the field from anywhere in the window. A table raises `Activate` when a selected row is double-clicked or Enter is pressed on it. `-HandlerOutput Pipeline` writes what the handlers write, before the window's result. Press Ctrl+F, then select the line about the deadlock and press Enter:

<!-- keep: acting-on-events-keys.png images/acting-on-events-keys.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+F'; Wait-Focused $w Filter; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 6; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'acting-on-events-keys.png'); Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Activate -Value 6; Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 -HandlerOutput Pipeline -OnShortcut @{ 'Ctrl+F' = { Set-GuiFocus Filter } } {
    TextBox -Name Filter -Placeholder 'text to find'
    Table $entries -Name Lines -OnActivate { $_.Value }
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$out[0] | Format-List
$out[1].Result
```

```text
Time    : 10/20/2026 9:31:30 AM
Level   : ERROR
Source  : db01
Message : deadlock on table orders

Close
```

![The LogLens window with a field above the table of the log's lines, the line about the deadlock selected, and a Close button at the bottom right](/images/acting-on-events-keys.png)

With the keyboard on the field, typing goes into it. Tab moves the keyboard to the table's rows, where the arrow keys select a line and Enter writes it out. A field uses the keys it edits text with itself, such as Ctrl+C and Ctrl+V, but Ctrl+F is not one of them, so the shortcut works from the field too.

## Leave the window open

`-NoWait` writes the window once it has opened and leaves it open while the script goes on. A `-NoWait` window's handlers run only while PowerShell is idle, as at a prompt, so a script that is busy reads the events itself. [Wait-GuiEvent](Wait-GuiEvent.md) takes them one at a time, oldest first, and waits when there are none. Each event has the properties a handler's `$_` has, so `Show-Matching` takes it as it is. Type `replica` and press Close:

<!-- keep: acting-on-events-open.png images/acting-on-events-open.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Filter -Event Change -Value 'replica'; while ((Get-GuiSnapshot -Window $w.Id).State.Lines.rows -ne 3) { Start-Sleep -Milliseconds 50 }; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'acting-on-events-open.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$shown = [System.Collections.Generic.List[object]]::new($entries)
$w = Show-GuiWindow -NoWait -Title 'LogLens' -Width 900 -Height 600 {
    Row {
        Text 'Find'
        TextBox -Name Filter -Placeholder 'text to find'
        Text 'Level'
        ComboBox 'All', 'INFO', 'WARN', 'ERROR' -Name Level -Selected All
    }
    Table $entries -Name Lines
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
do {
    $e = Wait-GuiEvent -Window $w.Id
    if ($e.Kind -eq 'Change') { Show-Matching $e }
} until ($e.Kind -eq 'Close')
$shown | Format-Table Level, Source, Message
```

```text
Level Source Message
----- ------ -------
WARN  db01   replica 2 behind by 4 s
WARN  db01   replica 2 behind by 9 s
INFO  db01   replica 2 caught up
```

![The LogLens window with replica typed in the field and All in the drop-down, above a table of the three lines about replica 2, and a Close button at the bottom right](/images/acting-on-events-open.png)

The button's `Click` comes first, then the window's `Close`, which comes last; once it is taken, the window is gone. The next chapter leaves LogLens's window open this way while the script follows a log that grows.

## The whole script so far

`LogLens.ps1` now follows the field and the drop-down as a person types and picks, shows the selected line under the table, goes to the field at Ctrl+F, and writes each line a person opens with Enter or a double click. The window's result comes last, after what the handlers wrote, and `Select-Object -SkipLast 1` drops it:

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

function Show-Matching($WindowEvent) {
    $state = $WindowEvent.State
    $matching = @($entries | Where-Object {
        ($state.Level -eq 'All' -or $_.Level -eq $state.Level) -and $_.Message -like "*$($state.Filter)*"
    })
    if ($shown.Count) { Remove-GuiItem -Window $WindowEvent.Window -Name Lines -Item $shown.ToArray() }
    $shown.Clear()
    if ($matching.Count) {
        Add-GuiItem -Window $WindowEvent.Window -Name Lines -Item $matching
        $shown.AddRange($matching)
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
$lines = Get-Content -LiteralPath $Path
if ($lines.Count -gt $Large) {
    $open = Show-GuiMessage "$(Split-Path -Leaf $Path) has $($lines.Count) lines. Open it anyway?" -Title 'LogLens' -Icon Question -Buttons YesNo
    if ($open -ne 'Yes') { return }
}
$entries = @($lines | ConvertFrom-LogLine)
$shown = [System.Collections.Generic.List[object]]::new($entries)
$details = {
    $e = $_.Value
    Update-GuiWidget -Name Details -Property @{ Text = "$($e.Level) from $($e.Source) at $($e.Time.ToString('HH:mm:ss')): $($e.Message)" }
}
$out = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 -HandlerOutput Pipeline -OnShortcut @{ 'Ctrl+F' = { Set-GuiFocus Filter } } {
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

Run it, press Ctrl+F, type `deadlock`, select the line and press Enter, then press Close:

<!-- keep: acting-on-events-loglens.png images/acting-on-events-loglens.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+F'; Wait-Focused $w Filter; Invoke-GuiTestEvent -Window $w.Id -Name Filter -Event Change -Value 'deadlock'; while ((Get-GuiSnapshot -Window $w.Id).State.Lines.rows -ne 1) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 0; Wait-Property $w Details text 'ERROR from db01 at 09:31:30: deadlock on table orders'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'acting-on-events-loglens.png'); Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Activate -Value 0; Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log | Format-Table Time, Level, Source, Message
```

```text
Time                  Level Source Message
----                  ----- ------ -------
10/20/2026 9:31:30 AM ERROR db01   deadlock on table orders
```

![The LogLens window with deadlock typed in the field, the one line about the deadlock selected in the table, under it the text ERROR from db01 at 09:31:30: deadlock on table orders, and a Close button at the bottom right](/images/acting-on-events-loglens.png)

## Next

[Following A Log](Following-A-Log.md): LogLens follows a log that grows, its new lines added as they arrive, and shows a progress window while a big log loads.
