---
title: Asking For Things
weight: 2
---

Chapter 1's LogLens shows every line of a log. A person looking for a problem wants fewer: the levels that matter, the lines that mention something, and no wait on a log too large to read in a grid. This chapter gets each from a ready-made window: a choice of the levels to show, a form for a search, and a question before a large log opens. Source: `crates/pwrs-gui/src/choice.rs` for `Read-GuiChoice`, `crates/pwrs-gui/src/input.rs` for `Read-GuiInput`, and `crates/pwrs-gui/src/dialogs.rs` for `Show-GuiMessage`.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows.

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
$entries = Get-Content -LiteralPath app.log | ConvertFrom-LogLine
```

</details>

## Which levels to show

`Read-GuiChoice` shows a list of choices with OK and Cancel and writes the one chosen. With `-Multiple` each choice has a check box and the command writes every one checked, in the order given; `-Default` checks some before the window opens. A person who only wants the problems leaves INFO unchecked:

<!-- focus: its pictures show the dialogs' list and field with the keyboard focus as they open -->
<!-- keep: asking-for-things-levels.png images/asking-for-things-levels.png -->
<!-- person: $w = Wait-Window 'LogLens: levels'; Wait-Enabled $w Ok; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'asking-for-things-levels.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$levels = Read-GuiChoice -Prompt 'Show which levels?' -Choice 'INFO', 'WARN', 'ERROR' -Default 'WARN', 'ERROR' -Multiple -Title 'LogLens: levels'
$levels
($entries | Where-Object Level -in $levels).Count
```

```text
WARN
ERROR
12
```

![The LogLens: levels window with the prompt "Show which levels?", the choices INFO, WARN and ERROR with WARN and ERROR checked, and OK and Cancel](/images/asking-for-things-levels.png)

Cancel writes nothing, so `$levels` is then `$null`, and LogLens takes that as a person who changed their mind.

## What to search for

`Read-GuiInput` shows a form of fields and writes one object with a property for each field, named by the field's `Name`. Each field is a hashtable of its `Name`, the `Label` shown beside it, its `Type`, and a `Default`. A text field takes the search, a number field the most lines to show, and a check box whether case matters:

<!-- keep: asking-for-things-search.png images/asking-for-things-search.png -->
<!-- person: $w = Wait-Window 'LogLens: search'; Invoke-GuiTestEvent -Window $w.Id -Name Text -Event Change -Value 'orders'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'asking-for-things-search.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$search = Read-GuiInput -Prompt 'Search the log' -Title 'LogLens: search' -Field @(
    @{ Name = 'Text'; Label = 'Text to find'; Type = 'Text' }
    @{ Name = 'Most'; Label = 'Most lines to show'; Type = 'Number'; Default = 10 }
    @{ Name = 'MatchCase'; Label = 'Match case'; Type = 'Check' }
)
$search
```

```text
Text    Most MatchCase
----    ---- ---------
orders 10.00     False
```

<!-- powershell -->

```text
Text   Most MatchCase
----   ---- ---------
orders   10     False
```

![The LogLens: search window with the prompt "Search the log", the field Text to find with orders typed in it, Most lines to show at 10, an unchecked Match case box, and OK and Cancel](/images/asking-for-things-search.png)

A number field writes a `[double]` and a check box a `[bool]`, so the search goes straight into a filter. `-clike` matches with case and `-like` without:

```powershell
$pattern = "*$($search.Text)*"
$found = $entries | Where-Object { if ($search.MatchCase) { $_.Message -clike $pattern } else { $_.Message -like $pattern } }
$found | Select-Object -First $search.Most | Format-Table Level, Source, Message
```

```text
Level Source Message
----- ------ -------
INFO  web01  GET /orders 200
WARN  web02  GET /orders took 1.8 s
INFO  web01  GET /orders/17 200
ERROR db01   deadlock on table orders
ERROR web01  GET /orders/17 500
INFO  web01  GET /orders 200
INFO  web02  POST /orders 201
ERROR web02  POST /orders 503
WARN  web02  retrying POST /orders
INFO  web02  POST /orders 201
```

## Before a large log

A grid of a million lines takes a while to fill. `Show-GuiMessage -Buttons YesNo` shows the system's message box with Yes and No and writes the button pressed, so LogLens can check with the person first. The size that counts as large is the script's choice; here it is 20 lines, so this log of 28 counts:

<!-- keep: asking-for-things-large.png images/asking-for-things-large.png -->
<!-- person: $d = Wait-Dialog 'LogLens'; Save-DialogPng 'LogLens' (Join-Path $PWD 'asking-for-things-large.png'); Invoke-GuiTestEvent -Dialog $d.Id -Value No -->
```powershell
$count = (Get-Content app.log).Count
if ($count -gt 20) {
    $open = Show-GuiMessage "app.log has $count lines. Open it anyway?" -Title 'LogLens' -Icon Question -Buttons YesNo
    "the person pressed $open"
}
```

```text
the person pressed No
```

![A Windows message box titled LogLens with the question icon, the message "app.log has 28 lines. Open it anyway?" and the buttons Yes and No](/images/asking-for-things-large.png)

## The whole script so far

`LogLens.ps1` now takes `-Large`, the line count past which it checks with the person first, and shows the choice of levels and the search form before the grid. Each window has a title of its own, so a person, or a test, can tell them apart:

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
$levels = Read-GuiChoice -Prompt 'Show which levels?' -Choice 'INFO', 'WARN', 'ERROR' -Default 'WARN', 'ERROR' -Multiple -Title 'LogLens: levels'
if (-not $levels) { return }
$search = Read-GuiInput -Prompt 'Search the log' -Title 'LogLens: search' -Field @(
    @{ Name = 'Text'; Label = 'Text to find'; Type = 'Text' }
    @{ Name = 'Most'; Label = 'Most lines to show'; Type = 'Number'; Default = 1000 }
    @{ Name = 'MatchCase'; Label = 'Match case'; Type = 'Check' }
)
if (-not $search) { return }
$pattern = "*$($search.Text)*"
$lines | ConvertFrom-LogLine |
    Where-Object { $_.Level -in $levels } |
    Where-Object { if ($search.MatchCase) { $_.Message -clike $pattern } else { $_.Message -like $pattern } } |
    Select-Object -First $search.Most |
    Out-GuiGridView -Title 'LogLens' -PassThru
```

Run it with a low `-Large`, so the question shows: Yes, the levels as they open, `500` as the search, and the three lines the grid then shows, all selected:

<!-- person: $d = Wait-Dialog 'LogLens'; Invoke-GuiTestEvent -Dialog $d.Id -Value Yes; $w = Wait-Window 'LogLens: levels'; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click; $w = Wait-Window 'LogLens: search'; Invoke-GuiTestEvent -Window $w.Id -Name Text -Event Change -Value '500'; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click; $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Select -Value 0, 1, 2; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log -Large 20 | Format-Table Time, Level, Source, Message
```

```text
Time                  Level Source Message
----                  ----- ------ -------
10/20/2026 9:31:31 AM ERROR web01  GET /orders/17 500
10/20/2026 9:37:15 AM ERROR web01  GET /orders 500
10/20/2026 9:37:16 AM ERROR web02  GET /orders 500
```

## Next

[A Window Of Your Own](A-Window-Of-Your-Own.md): LogLens draws its own window, a table of the lines with a filter and a level beside it, in place of three windows one after another.
