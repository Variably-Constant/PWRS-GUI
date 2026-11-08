---
title: How To Act On Events
weight: 4
---

Run code when a person clicks, types, selects or presses a shortcut in a window described in PowerShell, change the window from that code, collect what it writes, and see what it throws. Source: `crates/pwrs-gui/src/window.rs`, `show` and `run_handler` for a window the command waits on, and `Reader` for one opened with `-NoWait`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. A person's typing and clicks were sent by `Invoke-GuiTestEvent`, as [How To Test A Window](How-To-Test-A-Window.md) shows.

## Give a widget a handler

A widget's `-On<Event>` parameters take script blocks, which run when it raises that event: `-OnClick` on a [Button](New-GuiButton.md), `-OnChange` on an input, `-OnSelect` on a [ListBox](New-GuiListBox.md), [Table](New-GuiTable.md) or [TreeView](New-GuiTreeView.md). Each widget's page lists the events it raises. A handler gets the event as `$_` and as its argument:

| Property | What it gives |
|---|---|
| `Window` | the window's number |
| `Name` | the widget's name |
| `Kind` | the event, such as `Click`, `Change` or `Select` |
| `Value` | what the event contains, as the widget's page says: a changed input's new value, the object a table's row was made from |
| `State` | every named input's value at that moment, under its name |

Inside a handler, [Update-GuiWidget](Update-GuiWidget.md), [Add-GuiItem](Add-GuiItem.md), [Remove-GuiItem](Remove-GuiItem.md) and [Close-GuiWindow](Close-GuiWindow.md) act on the handler's own window without `-Window`.

## Show details of what is selected

A table's `Select` has the object the row was made from, so a handler reads its properties. The handler writes them into a [Text](New-GuiText.md):

<!-- person: $w = Wait-Window 'Services'; $deadline = [DateTime]::UtcNow.AddSeconds(20); while ((Get-GuiSnapshot -Window $w.Id).State.Pick.rows -lt 2 -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Pick -Event Select -Value 1; while ((Get-GuiSnapshot -Window $w.Id).Tree.Children[1].Properties.text -notlike 'db*' -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$services = [pscustomobject]@{ Name = 'web'; Port = 80 }, [pscustomobject]@{ Name = 'db'; Port = 5432 }
$result = Show-GuiWindow -Title 'Services' {
    Table $services -Name Pick -OnSelect {
        Update-GuiWidget -Name Detail -Property @{ Text = "$($_.Value.Name) listens on port $($_.Value.Port)" }
    }
    Text 'Select a service' -Name Detail
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.State.Pick
```

```text
Name Port
---- ----
db   5432
```

## Check what was entered before the window closes

A button without `-Result` leaves the window open. Its handler closes the window with [Close-GuiWindow](Close-GuiWindow.md) once what was entered is right, and says what is wrong otherwise:

```powershell
Set-Content -Path app.log -Value 'started'
```

<!-- person: $w = Wait-Window 'Open log'; Invoke-GuiTestEvent -Window $w.Id -Name Path -Event Change -Value 'missing.log'; Invoke-GuiTestEvent -Window $w.Id -Name Open -Event Click; $deadline = [DateTime]::UtcNow.AddSeconds(20); while ((Get-GuiSnapshot -Window $w.Id).Tree.Children[2].Properties.text -notlike 'No file*' -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Path -Event Change -Value 'app.log'; Invoke-GuiTestEvent -Window $w.Id -Name Open -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Open log' {
    Text 'Log file'
    TextBox -Name Path
    Text 'Name a file in this folder' -Name Problem
    ButtonBar {
        Button 'Open' -Name Open -Default -OnClick {
            if (Test-Path -LiteralPath $_.State.Path -PathType Leaf) {
                Close-GuiWindow -Result $_.State.Path
            } else {
                Update-GuiWidget -Name Problem -Property @{ Text = "No file is named $($_.State.Path)" }
            }
        }
    }
}
$result.Result
```

```text
app.log
```

The handler runs in the session that showed the window, so `Test-Path` looks in the folder the script is in.

## Collect what handlers write

What a handler writes is dropped unless `Show-GuiWindow -HandlerOutput` says where it goes: `Pipeline` writes it as the command's output, before the result, and `Console` sends it to Out-Default, so it shows in the console:

<!-- person: $w = Wait-Window 'Output'; Invoke-GuiTestEvent -Window $w.Id -Name Log -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Log -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Output' -HandlerOutput Pipeline {
    Button 'Log' -Name Log -OnClick { "pressed $($_.Name)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out.Count
$out[0..1]
$out[2].Result
```

```text
3
pressed Log
pressed Log
Ok
```

## Give the window shortcuts of its own

`Show-GuiWindow -OnShortcut` takes a hashtable from each shortcut, written as `Ctrl+S`, to the handler it runs, whose event's `Kind` is `Shortcut` and `Value` the shortcut. An enabled [MenuItem](New-GuiMenuItem.md) with the same keys acts first, and while a text field has the focus, the keys it edits with go to the field, and in a password field Alt+F8 too:

<!-- person: $w = Wait-Window 'Editor'; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+S'; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'F5'; Invoke-GuiTestEvent -Window $w.Id -Name Done -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Editor' -HandlerOutput Pipeline -OnShortcut @{
    'Ctrl+S' = { "saved by $($_.Value)" }
    'F5' = { "refreshed by $($_.Value)" }
} {
    TextBox -Name Body -Multiline
    ButtonBar { Button 'Close' -Name Done -Default -Result Done }
}
$out[0..1]
```

```text
saved by Ctrl+S
refreshed by F5
```

## See what a handler throws

A handler's error is written as an error record of the command, and the window stays open, so a person can try again:

<!-- person: $w = Wait-Window 'Fails'; Invoke-GuiTestEvent -Window $w.Id -Name Fail -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Fails' -ErrorVariable failed -ErrorAction SilentlyContinue {
    Button 'Fail' -Name Fail -OnClick { throw 'the share is offline' }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.Result
$failed.Count
$failed[0].Exception.Message
```

```text
Ok
1
the share is offline
```

## Act on the events of a window that does not wait

With `-NoWait`, `Show-GuiWindow` writes the window once it has opened, and its handlers run as engine-event actions when PowerShell is idle, as it is at a prompt, on the engine event the window's `SourceIdentifier` names. A script that goes on running reads the events itself with [Wait-GuiEvent](Wait-GuiEvent.md), which returns every one, whether or not a handler ran for it, and ends with the window's `Close`:

<!-- person: $w = Wait-Window 'Queue'; Invoke-GuiTestEvent -Window $w.Id -Name Ping -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Ping -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Stop -Event Click -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'Queue' {
    Button 'Ping' -Name Ping
    ButtonBar { Button 'Stop' -Name Stop -Result Stop }
}
do {
    $e = Wait-GuiEvent -Window $w.Id -Timeout 30
    if ($e.Name -eq 'Ping') { 'ping' }
} until ($null -eq $e -or $e.Kind -eq 'Close')
$e.Value
```

```text
ping
ping
Stop
```

The `-Timeout` ends the loop when no event comes for 30 seconds; without it, `Wait-GuiEvent` waits until one does, or until Ctrl+C.
