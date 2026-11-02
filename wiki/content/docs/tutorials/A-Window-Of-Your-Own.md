---
title: A Window Of Your Own
weight: 3
---

Chapter 2's LogLens opens three windows one after another: the levels, the search, and the grid. This chapter puts LogLens in one window of its own, built a step at a time from widgets: a line of text and a button, a table of the log's lines, a field and a drop-down for what to show, and a layout that places them. Source: the widgets are `crates/pwrs-gui/src/widgets.rs`, and the windows they make `crates/pwrs-gui/src/window.rs`.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the typing and the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows; a person's give the same output.

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

## Text and a button

`Show-GuiWindow` takes a block that writes the window's widgets, and shows them one under another. Press OK:

<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'LogLens' {
    Text "app.log has $($entries.Count) lines"
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.Result
```

```text
Ok
```

While the block runs, `Text`, `ButtonBar` and `Button` stand for the commands `New-GuiText`, `New-GuiButtonBar` and `New-GuiButton`. A button bar puts its buttons along the window's bottom, at the right, as the system's own dialogs do. A button with `-Result` closes the window when it is pressed, and `Show-GuiWindow` writes what the window closed with; its `Result` is the button's. `-Default` makes OK the button Enter presses, and `-Name` gives it a name that scripts and tests find it by. The block is ordinary PowerShell, so it reads `$entries` from the script.

Close the window instead, and the result is `Closed`:

<!-- person: $w = Wait-Window 'LogLens'; Close-GuiWindow -Window $w.Id -->
```powershell
(Show-GuiWindow -Title 'LogLens' { Text 'app.log'; ButtonBar { Button 'OK' -Name Ok -Default -Result Ok } }).Result
```

```text
Closed
```

## A table of the lines

`Table` shows objects in rows, a column for each property, which a person sorts by clicking a column's heading, as the grid in chapter 1 does. With `-Filter` it also has the grid's field that hides the rows without the text typed in it; LogLens leaves that out, since the next section gives it a field of its own. Give it a name, and the result's `State` gives the object of the row selected under that name:

<!-- keep: a-window-of-your-own-table.png images/a-window-of-your-own-table.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 18; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-window-of-your-own-table.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 {
    Table $entries -Name Lines
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.State.Lines | Format-List
```

```text
Time    : 10/20/2026 9:35:57 AM
Level   : ERROR
Source  : db01
Message : disk 92% full
```

![The LogLens window with a table of the columns Time, Level, Source and Message, the line "disk 92% full" selected, and an OK button at the bottom right](/images/a-window-of-your-own-table.png)

Without `-Width` and `-Height` a window fits its widgets, as the first two did, which suits a dialog. LogLens is a window a person works in, so it opens at a size of its own, 900 points across and 600 down, as an app's main window does. The table takes the height the window leaves it and scrolls when its rows do not fit, so a long log does not push the button out of the window.

## What to show

An input's value is what a person entered in it. A text field takes the text to find, and a drop-down the level, with `All` for every level; give each a name, and `State` gives their values under those names. Type `orders`, pick ERROR and press Find:

<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Filter -Event Change -Value 'orders'; Invoke-GuiTestEvent -Window $w.Id -Name Level -Event Change -Value 3; Invoke-GuiTestEvent -Window $w.Id -Name Find -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'LogLens' {
    TextBox -Name Filter -Placeholder 'text to find'
    ComboBox 'All', 'INFO', 'WARN', 'ERROR' -Name Level -Selected All
    ButtonBar { Button 'Find' -Name Find -Default -Result Find }
}
$result.State
```

```text
Filter Level
------ -----
orders ERROR
```

Each value is of its input's kind: the text field's is a string, and the drop-down's the text of the choice. `-Selected` sets the choice the window opens with, and `-Placeholder` the gray text an empty field shows.

## Lay them out

Widgets go one under another unless a layout places them. `Row` puts its widgets side by side, so the field and the drop-down are on one line above the table; [Grid](New-GuiGrid.md) puts widgets in columns, and [Group](New-GuiGroup.md) draws a frame around them, under a title. The window's own buttons go in its button bar, in the system's order: Find, then Close, on Windows, and Close, then Find, on macOS, Linux and the BSDs. `-Cancel` makes Close the button Escape presses. Type `disk` and press Find:

<!-- keep: a-window-of-your-own-layout.png images/a-window-of-your-own-layout.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Filter -Event Change -Value 'disk'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-window-of-your-own-layout.png'); Invoke-GuiTestEvent -Window $w.Id -Name Find -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 {
    Row {
        Text 'Find'
        TextBox -Name Filter -Placeholder 'text to find'
        Text 'Level'
        ComboBox 'All', 'INFO', 'WARN', 'ERROR' -Name Level -Selected All
    }
    Table $entries -Name Lines
    ButtonBar {
        Button 'Find' -Name Find -Default -Result Find
        Button 'Close' -Name Close -Cancel -Result Close
    }
}
$result.Result
$entries | Where-Object {
    ($result.State.Level -eq 'All' -or $_.Level -eq $result.State.Level) -and $_.Message -like "*$($result.State.Filter)*"
} | Format-Table Level, Source, Message
```

```text
Find

Level Source Message
----- ------ -------
ERROR db01   disk 92% full
```

![The LogLens window with Find and a field with disk typed in it, Level and a drop-down showing All, on one line above the table of the log's lines, and Find and Close at the bottom right](/images/a-window-of-your-own-layout.png)

A window closed by Close, by Escape or by its close box writes its `State` too, so a script reads `Result` to tell which way it closed. The field does not filter the table while a person types yet; the next chapter makes it.

## The whole script so far

`LogLens.ps1` now shows one window, 900 points by 600: the field and the drop-down on a line, the table of the log's lines under them, and Find and Close. Find writes the lines that match; Close writes nothing:

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
$result = Show-GuiWindow -Title 'LogLens' -Width 900 -Height 600 {
    Row {
        Text 'Find'
        TextBox -Name Filter -Placeholder 'text to find'
        Text 'Level'
        ComboBox 'All', 'INFO', 'WARN', 'ERROR' -Name Level -Selected All
    }
    Table $entries -Name Lines
    ButtonBar {
        Button 'Find' -Name Find -Default -Result Find
        Button 'Close' -Name Close -Cancel -Result Close
    }
}
if ($result.Result -ne 'Find') { return }
$entries | Where-Object {
    ($result.State.Level -eq 'All' -or $_.Level -eq $result.State.Level) -and $_.Message -like "*$($result.State.Filter)*"
}
```

Run it, type `replica` and press Find:

<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Filter -Event Change -Value 'replica'; Invoke-GuiTestEvent -Window $w.Id -Name Find -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log | Format-Table Time, Level, Source, Message
```

```text
Time                  Level Source Message
----                  ----- ------ -------
10/20/2026 9:32:10 AM WARN  db01   replica 2 behind by 4 s
10/20/2026 9:37:30 AM WARN  db01   replica 2 behind by 9 s
10/20/2026 9:38:02 AM INFO  db01   replica 2 caught up
```

## Next

[Acting On Events](Acting-On-Events.md): the table follows the field as a person types, a selected line shows its details, Ctrl+F goes to the field, and the window stays open while the script goes on.
