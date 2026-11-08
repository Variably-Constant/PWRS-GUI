---
title: A Page Of Your Own
weight: 7
---

Chapter 6's LogLens shows the log in widgets. This chapter adds a page of its own: a summary of the log written as HTML, which a window shows as it is, and which tells the script the source a person clicks. A page reaches PowerShell through `window.pwrs`, and PowerShell reaches the page through `Send-GuiMessage`. Source: `crates/pwrs-gui/src/page.rs`; the page's `window.pwrs` is `crates/pwrs-gui-web/src/runtime/bridge.js`.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the clicks were sent from a second runspace of the same process: in a window of widgets by `Invoke-GuiTestEvent`, as [How To Test A Window](How-To-Test-A-Window.md) shows, and in a page through UI Automation, which reaches a page's buttons as a screen reader does.

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

## A summary as a page

[Show-GuiWindow](Show-GuiWindow.md) `-Html` shows a page given as text, and waits until the page calls `window.pwrs.close(value)`; the value comes back as the result's `Result`. LogLens counts each source's lines by level in PowerShell and writes a row of the page for each, the source's name a button, and the page's script closes the window with the name of the source clicked.

The page draws with LogLens's own look: one stylesheet, in `$style` for every page of this chapter, gives it the colors PWRS-GUI's windows draw with, light or dark as the system is set. Text is at the left of its column and numbers at the right, and a WARN or ERROR count above zero is drawn in that level's color. Click db01:

<!-- keep: a-page-of-your-own-summary.png images/a-page-of-your-own-summary.png -->
<!-- person: $w = Wait-Window 'LogLens summary'; Wait-PageText 'LogLens summary' 'db01'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-page-of-your-own-summary.png'); Invoke-PageControl 'LogLens summary' 'db01' -->
```powershell
$style = @'
:root {
  color-scheme: light dark;
  --window: #f8f8f8; --field: #ffffff; --row: #f0f0f0; --text: #505050; --strong: #000000; --muted: #787878;
  --line: #bebebe; --button: #e6e6e6; --hover: #dcdcdc; --strip: #ececec;
  --pick: #90d1ff; --pick-text: #00537d; --link: #0078d4; --WARN: #c45500; --ERROR: #d00000;
}
@media (prefers-color-scheme: dark) {
  :root {
    --window: #1b1b1b; --field: #0a0a0a; --row: #161616; --text: #c8c8c8; --strong: #f5f5f5; --muted: #8c8c8c;
    --line: #3c3c3c; --button: #3c3c3c; --hover: #464646; --strip: #131313;
    --pick: #005c80; --pick-text: #c0deff; --link: #5aaaff; --WARN: #ff8f00; --ERROR: #ff5a5a;
  }
}
body { margin: 0; padding: 12px; background: var(--window); color: var(--text); font: 14px/1.45 "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; }
body:has(footer) { padding-bottom: 61px; }
h1 { margin: 0; font-size: 16px; font-weight: 600; color: var(--strong); }
.note { margin: 2px 0 12px; color: var(--muted); }
table { width: 100%; border-collapse: collapse; background: var(--field); border: 1px solid var(--line); }
th, td { padding: 5px 10px; text-align: right; font-variant-numeric: tabular-nums; }
th { font-weight: 600; color: var(--muted); border-bottom: 1px solid var(--line); }
th:first-child, td:first-child { text-align: left; }
tbody tr:hover { background: var(--row); }
tr.chosen, tr.chosen:hover { background: var(--pick); color: var(--pick-text); }
td.WARN { color: var(--WARN); }
td.ERROR { color: var(--ERROR); }
button.source { padding: 0; border: 0; background: none; color: var(--link); font: inherit; font-weight: 600; cursor: pointer; }
button.source:hover { text-decoration: underline; }
button.source:focus-visible { outline: 1px solid var(--pick-text); outline-offset: 2px; border-radius: 3px; }
tr.chosen button.source { color: var(--pick-text); }
.lines { margin: 12px 0 0; padding: 4px 0; list-style: none; background: var(--field); border: 1px solid var(--line); font: 13px/1.6 "Cascadia Mono", Consolas, ui-monospace, monospace; }
.lines li { padding: 0 10px; white-space: pre; }
.lines .time { color: var(--muted); }
.lines .WARN { color: var(--WARN); }
.lines .ERROR { color: var(--ERROR); }
.stats { display: flex; gap: 8px; }
.stat { flex: 1; padding: 8px 10px; background: var(--field); border: 1px solid var(--line); color: var(--muted); }
.stat b { display: block; font-size: 22px; font-weight: 600; color: var(--strong); font-variant-numeric: tabular-nums; }
.stat.WARN b { color: var(--WARN); }
.stat.ERROR b { color: var(--ERROR); }
footer { position: fixed; left: 0; right: 0; bottom: 0; display: flex; justify-content: flex-end; gap: 8px; padding: 12px; background: var(--strip); border-top: 1px solid var(--line); }
footer button { min-width: 80px; min-height: 24px; padding: 0 12px; border: 0; border-radius: 6px; background: var(--button); color: var(--strong); font: inherit; }
footer button:hover { background: var(--hover); }
footer button:focus-visible { outline: 1px solid var(--pick-text); outline-offset: 1px; }
'@
$sourceCount = @($entries.Source | Sort-Object -Unique).Count
$rows = foreach ($source in $entries | Group-Object Source) {
    $count = $source.Group | Group-Object Level -AsHashTable -AsString
    $cells = foreach ($level in 'INFO', 'WARN', 'ERROR') {
        $n = $count[$level].Count
        if ($n -and $level -ne 'INFO') { "<td class=`"$level`">$n</td>" } else { "<td>$n</td>" }
    }
    $name = [System.Net.WebUtility]::HtmlEncode($source.Name)
    "<tr><td><button class=`"source`">$name</button></td>$(-join $cells)</tr>"
}
$page = @"
<style>$style</style>
<h1>Lines by source</h1>
<p class="note">$($entries.Count) lines from $sourceCount sources. Click a source.</p>
<table>
  <thead><tr><th>Source</th><th>INFO</th><th>WARN</th><th>ERROR</th></tr></thead>
  <tbody>
  $($rows -join "`n  ")
  </tbody>
</table>
<script>
  document.querySelectorAll('button.source').forEach(function (b) {
    b.onclick = function () { window.pwrs.close(b.textContent); };
  });
</script>
"@
$choice = Show-GuiWindow -Html $page -Title 'LogLens summary' -Width 400 -Height 220
$entries | Where-Object Source -eq $choice.Result | Format-Table Time, Level, Message
```

```text
Time                  Level Message
----                  ----- -------
10/20/2026 9:30:04 AM INFO  accepting connections
10/20/2026 9:31:30 AM ERROR deadlock on table orders
10/20/2026 9:32:10 AM WARN  replica 2 behind by 4 s
10/20/2026 9:34:12 AM INFO  checkpoint done
10/20/2026 9:35:57 AM ERROR disk 92% full
10/20/2026 9:36:03 AM WARN  vacuum skipped
10/20/2026 9:37:30 AM WARN  replica 2 behind by 9 s
10/20/2026 9:38:02 AM INFO  replica 2 caught up
```

![The LogLens summary window headed Lines by source, 28 lines from 3 sources, with a table of db01, web01 and web02, each a link-styled button, beside its counts of INFO, WARN and ERROR lines, the WARN and ERROR counts in their colors](/images/a-page-of-your-own-summary.png)

`HtmlEncode` writes a source's name into the page as text, so a name with a `<` or an `&` in it shows as written rather than becoming part of the page. A page window is 800 by 600 points unless `-Width` and `-Height` say otherwise. When a person closes the window instead of clicking, `Result` is `Closed`, which names no source.

## A page that sends to the script

`window.pwrs.send(value)` passes a value to `-OnMessage`, and what that script block writes goes back to the page as the value of the Promise `send` returned: nothing as `null`, one object as itself, and several as an array, so `,@(...)` sends an array of any length. Objects reach the page as JSON objects with the same properties. Here a click marks the source's row and requests its lines, which the page lists under the table, each line's time, level and message apart so the page can color them. Close ends the window. Click db01, then Close:

<!-- keep: a-page-of-your-own-lines.png images/a-page-of-your-own-lines.png -->
<!-- person: $w = Wait-Window 'LogLens summary'; Invoke-PageControl 'LogLens summary' 'db01'; Wait-PageText 'LogLens summary' 'disk 92% full'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-page-of-your-own-lines.png'); Invoke-PageControl 'LogLens summary' 'Close' -->
```powershell
$page = @"
<style>$style</style>
<h1>Lines by source</h1>
<p class="note">$($entries.Count) lines from $sourceCount sources. Click a source to list its lines.</p>
<table>
  <thead><tr><th>Source</th><th>INFO</th><th>WARN</th><th>ERROR</th></tr></thead>
  <tbody>
  $($rows -join "`n  ")
  </tbody>
</table>
<ol class="lines" id="lines" hidden></ol>
<footer><button id="close">Close</button></footer>
<script>
  // Lists the lines the script sends, each its time, level and message.
  function showLines(lines) {
    var list = document.getElementById('lines');
    list.replaceChildren();
    lines.forEach(function (line) {
      var item = document.createElement('li');
      [['time', line.Time], [line.Level, line.Level.padEnd(5)], ['message', line.Message]].forEach(function (part, at) {
        var span = document.createElement('span');
        span.className = part[0];
        span.textContent = part[1];
        if (at) { item.append(' '); }
        item.appendChild(span);
      });
      list.appendChild(item);
    });
    list.hidden = false;
  }
  document.querySelectorAll('button.source').forEach(function (b) {
    b.onclick = function () {
      document.querySelectorAll('tr.chosen').forEach(function (tr) { tr.classList.remove('chosen'); });
      b.closest('tr').classList.add('chosen');
      window.pwrs.send(b.textContent).then(showLines);
    };
  });
  document.getElementById('close').onclick = function () { window.pwrs.close('done'); };
</script>
"@
$lines = {
    param($source)
    , @($entries | Where-Object Source -eq $source | ForEach-Object { [pscustomobject]@{ Time = $_.Time.ToString('HH:mm:ss'); Level = $_.Level; Message = $_.Message } })
}
(Show-GuiWindow -Html $page -Title 'LogLens summary' -Width 460 -Height 470 -OnMessage $lines).Result
```

```text
done
```

![The LogLens summary window with the table of sources, db01's row marked, under it db01's eight lines from 09:30:04 accepting connections to 09:38:02 replica 2 caught up, each WARN and ERROR in its color, and a Close button on a strip along the bottom](/images/a-page-of-your-own-lines.png)

`-OnMessage` runs on the command's own thread while the window waits, so it reads `$entries` as any script block in the session does. The page puts each part in with `textContent`, which writes it as text, as `HtmlEncode` did in PowerShell. Its Close button is at the bottom right on a strip with a line over it, 12 points in from the edges and at least 80 by 24 points, where a [ButtonBar](New-GuiButtonBar.md) puts the buttons of PWRS-GUI's own windows on Windows.

## Sending to the page

With `-NoWait` the command writes the window once its page has loaded and drawn its first frame, and the script goes on. [Send-GuiMessage](Send-GuiMessage.md) then sends the page values, which reach the function the page gave `window.pwrs.on`. Here the script counts the log's levels into the page line by line, then says how many lines it read. Then click Close:

<!-- keep: a-page-of-your-own-counts.png images/a-page-of-your-own-counts.png -->
<!-- person: $w = Wait-Window 'LogLens counts'; Wait-PageText 'LogLens counts' 'Read 28 lines'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-page-of-your-own-counts.png'); Invoke-PageControl 'LogLens counts' 'Close' -->
```powershell
$page = @"
<style>$style</style>
<h1>Reading app.log</h1>
<p class="note" id="status">Reading</p>
<div class="stats">
  <div class="stat">INFO<b id="INFO">0</b></div>
  <div class="stat WARN">WARN<b id="WARN">0</b></div>
  <div class="stat ERROR">ERROR<b id="ERROR">0</b></div>
</div>
<footer><button id="close">Close</button></footer>
<script>
  window.pwrs.on(function (value) {
    if (value.Read) {
      document.getElementById('status').textContent = 'Read ' + value.Read + ' lines';
    } else {
      var count = document.getElementById(value);
      count.textContent = Number(count.textContent) + 1;
    }
  });
  document.getElementById('close').onclick = function () { window.pwrs.close(); };
</script>
"@
$w = Show-GuiWindow -Html $page -Title 'LogLens counts' -Width 380 -Height 220 -NoWait
foreach ($entry in $entries) { Send-GuiMessage -Window $w.Id -Message $entry.Level }
Send-GuiMessage -Window $w.Id -Message @{ Read = $entries.Count }
"sent $($entries.Count) lines"
```

```text
sent 28 lines
```

![The LogLens counts window headed Reading app.log, reading Read 28 lines, with three boxes counting INFO 16, WARN 6 and ERROR 6, the WARN and ERROR counts in their colors, and a Close button at the bottom right on a strip along the window's bottom](/images/a-page-of-your-own-counts.png)

A window that has closed refuses a value with a non-terminating `GuiRefused`, so a script sending to a window the person may close goes on.

## A page in a folder

`-Path` names a folder whose `index.html` is the page, and the page loads the folder's other files by their paths, with no network server. This page has no data of its own: its script requests the summary from PowerShell with `send`, draws the table from the objects that come back, and requests a source's lines when its button is clicked. The folder takes the stylesheet from `$style`:

```powershell
$null = New-Item -ItemType Directory -Path summary
Set-Content -Path summary/loglens.css -Value $style
```

The page and its script. The page gets each file as it is written, so `index.html` starts with the doctype that `-Html` puts before a page given as text:

<!-- file: summary/index.html -->
```html
<!DOCTYPE html>
<meta charset="utf-8">
<link rel="stylesheet" href="loglens.css">
<h1>Lines by source</h1>
<p class="note" id="note"></p>
<table>
  <thead><tr><th>Source</th><th>INFO</th><th>WARN</th><th>ERROR</th></tr></thead>
  <tbody id="sources"></tbody>
</table>
<ol class="lines" id="lines" hidden></ol>
<footer><button id="close">Close</button></footer>
<script src="summary.js"></script>
```

<!-- file: summary/summary.js -->
```javascript
// Lists the lines the script sends, each its time, level and message.
function showLines(lines) {
  var list = document.getElementById('lines');
  list.replaceChildren();
  lines.forEach(function (line) {
    var item = document.createElement('li');
    [['time', line.Time], [line.Level, line.Level.padEnd(5)], ['message', line.Message]].forEach(function (part, at) {
      var span = document.createElement('span');
      span.className = part[0];
      span.textContent = part[1];
      if (at) { item.append(' '); }
      item.appendChild(span);
    });
    list.appendChild(item);
  });
  list.hidden = false;
}

// Draws a row for each source the script counts, its name a button that
// marks the row and requests the source's lines.
window.pwrs.send({ Want: 'Summary' }).then(function (sources) {
  var lines = sources.reduce(function (n, s) { return n + s.INFO + s.WARN + s.ERROR; }, 0);
  document.getElementById('note').textContent = lines + ' lines from ' + sources.length + ' sources. Click a source to list its lines.';
  var body = document.getElementById('sources');
  sources.forEach(function (source) {
    var row = body.insertRow();
    var button = document.createElement('button');
    button.className = 'source';
    button.textContent = source.Source;
    button.onclick = function () {
      body.querySelectorAll('tr.chosen').forEach(function (tr) { tr.classList.remove('chosen'); });
      row.classList.add('chosen');
      window.pwrs.send({ Want: 'Lines', Source: source.Source }).then(showLines);
    };
    row.insertCell().appendChild(button);
    ['INFO', 'WARN', 'ERROR'].forEach(function (level) {
      var cell = row.insertCell();
      cell.textContent = source[level];
      if (source[level] && level !== 'INFO') { cell.className = level; }
    });
  });
});

document.getElementById('close').onclick = function () { window.pwrs.close('done'); };
```

`-OnMessage` writes the summary as objects, a source's counts in each, and a source's lines as objects too. Click web02, then Close:

<!-- keep: a-page-of-your-own-folder.png images/a-page-of-your-own-folder.png -->
<!-- person: $w = Wait-Window 'LogLens summary'; Invoke-PageControl 'LogLens summary' 'web02'; Wait-PageText 'LogLens summary' 'POST /orders 503'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'a-page-of-your-own-folder.png'); Invoke-PageControl 'LogLens summary' 'Close' -->
```powershell
function Get-Summary {
    foreach ($source in $entries | Group-Object Source) {
        $count = $source.Group | Group-Object Level -AsHashTable -AsString
        [pscustomobject]@{ Source = $source.Name; INFO = $count['INFO'].Count; WARN = $count['WARN'].Count; ERROR = $count['ERROR'].Count }
    }
}
$asked = {
    param($message)
    if ($message.Want -eq 'Summary') { , @(Get-Summary) }
    else { , @($entries | Where-Object Source -eq $message.Source | ForEach-Object { [pscustomobject]@{ Time = $_.Time.ToString('HH:mm:ss'); Level = $_.Level; Message = $_.Message } }) }
}
(Show-GuiWindow -Path summary -Title 'LogLens summary' -Width 460 -Height 510 -OnMessage $asked).Result
```

```text
done
```

![The LogLens summary window from the folder, with the table of sources, web02's row marked in the selection color, under it web02's ten lines from 09:30:02 INFO started to 09:39:11 INFO GET /orders 200, and a Close button at the bottom right on a strip along the window's bottom](/images/a-page-of-your-own-folder.png)

A page stays on its own origin: a link elsewhere and content from the network are refused unless `-AllowNavigation`, and `-DevTools` lets a person open the webview's developer tools to look into the page.

## The whole script so far

`LogLens.ps1` now has a View menu whose Summary opens the page in the `summary` folder beside the script. `Get-Summary` counts the entries the window has, the log's new lines among them, and the page lists the lines of the source a person clicks:

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

Run it. Choose View, then Summary, click db01, and close the summary; then select the last line of the table, press Enter, and press Close:

<!-- keep: a-page-of-your-own-loglens.png images/a-page-of-your-own-loglens.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Summary -Event Click; $null = Wait-Window 'LogLens summary'; Invoke-PageControl 'LogLens summary' 'db01'; Wait-PageText 'LogLens summary' 'disk 92% full'; Save-SnapshotPng (Get-GuiSnapshot -Window (Wait-Window 'LogLens summary').Id -Pixels) (Join-Path $PWD 'a-page-of-your-own-loglens.png'); Invoke-PageControl 'LogLens summary' 'Close'; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value 27; Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Activate -Value 27; Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log | Format-Table Time, Level, Source, Message
```

```text
Time                  Level Source Message
----                  ----- ------ -------
10/20/2026 9:39:55 AM INFO  web01  stopping
```

![The LogLens summary window opened from LogLens's View menu, with the table of sources, db01's row marked in the selection color, under it db01's eight lines, and a Close button at the bottom right on a strip along the window's bottom](/images/a-page-of-your-own-loglens.png)

While the summary is open, LogLens's own window waits for it, as it waits for any handler, and the lines the log gains meanwhile reach LogLens once the summary closes.

## Next

[A Timeline](A-Timeline.md): the entries on a timeline that pans and zooms, a click on an entry selecting it.
