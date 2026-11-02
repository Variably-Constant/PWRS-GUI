---
title: How To Handle Errors
weight: 12
---

Tell the errors that end a command from those it reports and goes on after, catch them by id, and recover when the renderer goes away. Source: `gui_error` in `crates/pwrs-gui/src/session.rs`, and the [Error Reference](Error-Reference.md).

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Two kinds

An error that leaves the command nothing to do ends it: a renderer that cannot start, one that goes away, arguments that cannot work. It is terminating, so `try` catches it as it does an exception, and in a script without `try` it ends the statement. An error about one target among several does not end the command: `Invoke-GuiTestEvent` and `Get-GuiSnapshot` report a window or widget that does not exist, or a window with no renderer yet, and go on with the next target piped to them. `-ErrorAction Stop` makes those terminating too, and `-ErrorVariable` collects them.

## Catch by id

The first part of an error's `FullyQualifiedErrorId` names what happened, whatever the message says:

```powershell
try {
    Read-GuiChoice -Choice 'alpha' -Default 'omega'
} catch {
    switch -Wildcard ($_.FullyQualifiedErrorId) {
        'GuiDefaultNotAChoice,*' { 'The default is not one of the choices' }
        'GuiNoDisplay,*'         { 'No window can open here' }
        default                  { throw }
    }
}
```

```text
The default is not one of the choices
```

## Collect the errors that do not end the command

```powershell
Invoke-GuiTestEvent -Window 1 -Name Ok -Event Click -ErrorVariable failed -ErrorAction SilentlyContinue
$failed.Count
$failed[0].FullyQualifiedErrorId
```

```text
1
GuiNoSession,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand
```

`-ErrorAction Stop` makes the same error terminating, so `try` catches it:

```powershell
try {
    Invoke-GuiTestEvent -Window 1 -Name Ok -Event Click -ErrorAction Stop
} catch {
    "Caught: $($_.FullyQualifiedErrorId)"
}
```

```text
Caught: GuiNoSession,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand
```

## When the renderer goes away

A helper that exits while a command waits on one of its windows, killed here to show it, ends that command with `GuiRendererGone`, whose message says how the helper ended and quotes the end of its log. The session starts a new helper for the next window:

<!-- varies: the helper's exit, log path and session -->
```powershell
$dialog = [powershell]::Create().AddScript({ Read-GuiChoice -Choice 'one', 'two' -Title 'Interrupted' })
$running = $dialog.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not (Get-GuiWindow -Title 'Interrupted') -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
Get-CimInstance Win32_Process -Filter "Name = 'pwrs-gui-egui.exe' AND ParentProcessId = $PID" | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
$null = $dialog.EndInvoke($running)
$gone = $dialog.Streams.Error[0]
$gone.FullyQualifiedErrorId
($gone.Exception.Message -split ';')[0]
$dialog.Dispose()
```

```text
GuiRendererGone,Pwrs.Modules.PWRS_GUI.ReadGuiChoiceCommand
the GUI renderer has gone: the helper ended (exit code 0xffffffff)
```

The next command that opens a window starts a new helper:

<!-- person: $w = Wait-Window 'Again'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 0; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Read-GuiChoice -Choice 'still works' -Title 'Again'
(Get-GuiOption).Running
```

```text
still works
Helper
```

## Ctrl+C

Ctrl+C in the console, or `Stop()` on the runspace a command runs in, ends a command waiting on a window and closes the window; the command writes nothing and no error. A command waiting for a renderer to start stops waiting the same way.
