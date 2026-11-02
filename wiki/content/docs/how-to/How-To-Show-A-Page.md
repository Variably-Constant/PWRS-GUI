---
title: How To Show A Page
weight: 3
---

Show a page of your own in a window: show a form and take the result, reply to the page from PowerShell, send it values while the script goes on, show a folder of files, and restore the page's state when its engine ends. Source: `crates/pwrs-gui/src/page.rs`; the page's `window.pwrs` is `crates/pwrs-gui-web/src/runtime/bridge.js`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths. No person used these windows: each page submits its own form or closes once it has what it waited for, and the text says where a page of yours would wait for a person instead.

## A form and its result

[Show-GuiWindow](Show-GuiWindow.md) `-Html` shows the HTML and waits until the page calls `window.pwrs.close(value)`. The value comes back as the result's `Result`, made by `ConvertFrom-Json`:

```powershell
$form = @'
<form id="f">
  <label>Server <input id="server" value="web01"></label>
  <label><input id="restart" type="checkbox" checked> Restart it</label>
  <button>OK</button>
</form>
<script>
  var f = document.getElementById('f');
  f.onsubmit = function (e) {
    e.preventDefault();
    window.pwrs.close({ Server: document.getElementById('server').value, Restart: document.getElementById('restart').checked });
  };
  f.requestSubmit();
</script>
'@
$answer = (Show-GuiWindow -Html $form -Title 'Restart a server').Result
$answer
```

```text
Server Restart
------ -------
web01     True
```

The last line of the script submits the form as the page loads, so the example runs without a person; without it, the window waits for OK. When the person closes the window instead, `Result` is the string `Closed`, and Ctrl+C closes the window and writes nothing.

## Reply to the page from PowerShell

`window.pwrs.send(value)` passes the value to `-OnMessage`, and what the script block writes comes back to the page as the value of the Promise `send` returned. Nothing comes back as `null`, one object as itself and several as an array, so `,@(...)` returns an array of any length:

```powershell
Set-Content -Path one.txt, two.txt -Value 'x'
$page = '<ul id="files"></ul><script>window.pwrs.send("files").then(function (names) { names.forEach(function (n) { var li = document.createElement("li"); li.textContent = n; document.getElementById("files").appendChild(li); }); window.pwrs.close(names.join(", ")); });</script>'
(Show-GuiWindow -Html $page -OnMessage { param($m) , @(Get-ChildItem -Name) }).Result
```

```text
one.txt, two.txt
```

`-OnMessage` runs on the pipeline thread while the command waits, so it reads and changes the session's variables as any script block in it does.

## Send values while the script goes on

With `-NoWait` the command writes the window once its page has loaded and drawn its first frame, and [Send-GuiMessage](Send-GuiMessage.md) sends the page values for the functions it registered with `window.pwrs.on`:

```powershell
$page = '<progress id="p" max="3" value="0"></progress><script>window.pwrs.on(function (v) { if (v === "done") { window.pwrs.close(); } else { document.getElementById("p").value = v.Step; } });</script>'
$w = Show-GuiWindow -Html $page -Title 'Copying' -NoWait
foreach ($step in 1..3) {
    Start-Sleep -Milliseconds 200
    Send-GuiMessage -Window $w.Id -Message @{ Step = $step }
}
Send-GuiMessage -Window $w.Id -Message 'done'
```

A window that has closed refuses a value with a non-terminating `GuiRefused`, so a loop sending to a window the person may close goes on.

## Show a folder of files

`-Path` names a folder whose `index.html` is the page; the page loads the folder's other files by relative path, through `pwrs://`, with no network server:

```powershell
$null = New-Item -ItemType Directory site
Set-Content site/index.html '<link rel="stylesheet" href="style.css"><h1>Report</h1><script src="app.js"></script>'
Set-Content site/style.css 'h1 { color: steelblue; }'
Set-Content site/app.js 'window.pwrs.close(getComputedStyle(document.querySelector("h1")).color);'
(Show-GuiWindow -Path site).Result
```

```text
rgb(70, 130, 180)
```

A page stays on its own origin: a navigation elsewhere and remote content are refused unless `-AllowNavigation`, and `-DevTools` lets the person open the webview's developer tools.

## Restore a page's state when its engine ends

When the process drawing a page ends under it, the renderer loads the page again, and `window.pwrs.restore()` returns what the page last gave `window.pwrs.save`. [Web Page Data And Recovery](Web-Page-Data-And-Recovery.md) explains what ends and what comes back.

On Windows, this page counts its loads, and after the first it sends PowerShell a message whose handler ends the page's WebView2 browser process. The session's web data is its own, in a folder named by the web helper's process id, so the browser process on that folder is the page's alone; in `Shared` mode the same browser process would run every PowerShell process's pages:

```powershell
Remove-Module PWRS-GUI
Import-Module PWRS-GUI
Set-GuiOption -Mode Helper -WebView PerProcess
$page = '<script>window.pwrs.restore().then(function (count) { count = (count || 0) + 1; window.pwrs.save(count); if (count === 1) { window.pwrs.send("end my engine"); } else { window.pwrs.close("loaded " + count + " times"); } });</script>'
$end = {
    $helper = (Get-CimInstance Win32_Process -Filter "Name = 'pwrs-gui-web.exe' AND ParentProcessId = $PID").ProcessId
    Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" |
        Where-Object { $_.CommandLine -like "*WebView2-PerProcess\$helper-*" -and $_.CommandLine -notlike '*--type=*' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
}
(Show-GuiWindow -Html $page -OnMessage $end).Result
```

```text
loaded 2 times
```

A page that cannot be shown, because its webview could not be made or its engine ended again before it drew, closes its window, and the command fails with `GuiPageFailed`, saying why; [How To Handle Errors](How-To-Handle-Errors.md) shows catching an error by its id.
