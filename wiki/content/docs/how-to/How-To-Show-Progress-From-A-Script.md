---
title: How To Show Progress From A Script
weight: 5
---

Show how far a script's work has gone in a window that stays open while it runs, let a person stop the work from the window, and close the window when the work ends. Source: `crates/pwrs-gui/src/window.rs`, `UpdateGuiWidget`, `AddGuiItem`, `WaitGuiEvent` and `CloseGuiWindow`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. A person's clicks were sent by `Invoke-GuiTestEvent`, as [How To Test A Window](How-To-Test-A-Window.md) shows, and `Start-Sleep` stands for each step's work.

## Follow a loop, and let a person stop it

`Show-GuiWindow -NoWait` writes the window once it has opened, so the loop runs while the window shows it. Each step sets the [ProgressBar](New-GuiProgressBar.md) with [Update-GuiWidget](Update-GuiWidget.md) and adds a line to the [LogView](New-GuiLogView.md) with [Add-GuiItem](Add-GuiItem.md). Before each step, [Wait-GuiEvent](Wait-GuiEvent.md) with `-Timeout 0` takes a press of Stop if there is one, without waiting. It takes the window's `Close` the same way, so a person who closes the window stops the work too. Press Stop once two files are copied:

<!-- person: $w = Wait-Window 'Copying'; $deadline = [DateTime]::UtcNow.AddSeconds(20); while (@((Get-GuiSnapshot -Window $w.Id).Tree.Children[1].Properties.lines).Count -lt 2 -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Stop -Event Click -->
```powershell
$files = 1..5 | ForEach-Object { "report$_.csv" }
$w = Show-GuiWindow -NoWait -Title 'Copying' {
    ProgressBar -Name Progress -Maximum $files.Count -Text 'Starting'
    LogView -Name Log -Height 120
    ButtonBar { Button 'Stop' -Name Stop -Cancel }
}
$copied = 0
$asked = $null
foreach ($file in $files) {
    $asked = Wait-GuiEvent -Window $w.Id -Name Stop -Timeout 0
    if ($asked) { break }
    Start-Sleep -Milliseconds 500
    $copied++
    Add-GuiItem -Window $w.Id -Name Log -Item "copied $file"
    Update-GuiWidget -Window $w.Id -Name Progress -Property @{ Value = $copied; Text = "$copied of $($files.Count)" }
}
"copied $copied of $($files.Count), then $($asked.Kind) from $($asked.Name)"
```

```text
copied 3 of 5, then Click from Stop
```

Stop has no `-Result`, so pressing it leaves the window open until the script closes it, as the next section does. As the window's `-Cancel` button, it is also what Escape presses.

## Close the window when the work ends

[Close-GuiWindow](Close-GuiWindow.md) closes the window. The events of a window opened with `-NoWait` wait in the module until its `Close` is taken, so the script then takes what is left, up to the `Close`. A window the person already closed has had its `Close` taken by the loop:

```powershell
if ($asked.Kind -ne 'Close') {
    Close-GuiWindow -Window $w.Id
    while ((Wait-GuiEvent -Window $w.Id).Kind -ne 'Close') { }
}
@(Get-GuiWindow -Title 'Copying').Count
```

```text
0
```

`Get-GuiWindow` finds no window titled Copying once it has closed.

## Show work of unknown length

A [Spinner](New-GuiSpinner.md) turns while it is shown, and a [Text](New-GuiText.md) beside it says what the script is waiting for. The block writes one widget, the Row, so the Row is the window's root, as [Get-GuiSnapshot](Get-GuiSnapshot.md)'s `Tree` shows:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Waiting' {
    Row { Spinner -Name Busy; Text 'Waiting for the service' -Name Status }
}
Start-Sleep -Seconds 1
Update-GuiWidget -Window $w.Id -Name Status -Property @{ Text = 'The service replied' }
(Get-GuiSnapshot -Window $w.Id).Tree.Children[1].Properties.text
Close-GuiWindow -Window $w.Id
while ((Wait-GuiEvent -Window $w.Id).Kind -ne 'Close') { }
```

```text
The service replied
```

## Show the progress of another process

When the work runs in another process, such as a job or a service, the process can move the bar and add the log's lines itself, through a SubEtha ring the widget reads, with no PowerShell thread in between: [How To Feed A Widget From A SubEtha Ring](How-To-Feed-A-Widget-From-A-SubEtha-Ring.md).
