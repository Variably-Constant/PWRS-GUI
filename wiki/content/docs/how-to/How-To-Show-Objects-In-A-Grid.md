---
title: How To Show Objects In A Grid
weight: 2
---

Show objects in a table that sorts and filters, leave it open while the script goes on, or wait for rows to be picked and pass them on. Source: `crates/pwrs-gui/src/grid.rs`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows; a person's clicks give the same output.

## Look at objects and go on

Without `-PassThru`, `-OutputMode` or `-Wait`, `Out-GuiGridView` returns once its input has ended and its window has opened, and leaves the window open, as Out-GridView does, so the script goes on while the table is read:

```powershell
Get-ChildItem -Path $PSHOME -Filter *.dll | Select-Object -First 20 | Out-GuiGridView -Title 'Libraries'
'The script went on while the window stays open'
```

```text
The script went on while the window stays open
```

In the window, click a column's heading to sort by it, again to reverse; type in Filter to show only the rows with a cell that contains the text. The window stays until it is closed, the module is removed from every runspace that has it, or PowerShell exits.

## Pick rows and pass them on

`-PassThru` waits for OK and writes the selected rows, the objects as they were piped, in their original order whatever the sort; Ctrl+click and Shift+click select several:

<!-- person: $w = Wait-Window 'Stop which?'; Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Select -Value 0, 1; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$jobs = 1..3 | ForEach-Object { Start-Job -Name "worker$_" -ScriptBlock { Start-Sleep -Seconds 60 } }
$jobs | Out-GuiGridView -Title 'Stop which?' -PassThru | Stop-Job -PassThru | Select-Object Name, State
$jobs | Remove-Job -Force
```

```text
Name    State
----    -----
worker1 Stopped
worker2 Stopped
```

Cancel writes nothing, so the pipeline after it does nothing.

## Pick exactly one

`-OutputMode Single` writes one row, taken with OK or a double-click:

<!-- person: $w = Wait-Window 'Choose a verb'; Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Activate -Value 2 -->
```powershell
$verb = Get-Verb -Verb Start, Stop, Restart, Suspend, Resume | Sort-Object Verb |
    Out-GuiGridView -OutputMode Single -Title 'Choose a verb'
$verb.Verb
```

```text
Start
```

## Wait for the window to close

`-Wait` makes the command wait until the window is closed, and it writes nothing: a script that shows results at its end can stay open this way until they have been read.

## The columns

The columns come from the first object: the table view `Get-FormatData` gives its type, with that view's labels, format strings and script blocks, as the console's table shows it; otherwise its default display properties, or all its properties. To choose the columns, select them first. [Get-GuiWindow](Get-GuiWindow.md) lists the window once it has drawn its first frame, so the script waits for it before reading its columns:

```powershell
Get-Process -Id $PID | Select-Object Name, Id, @{ Name = 'MemoryMB'; Expression = { [math]::Round($_.WorkingSet64 / 1MB) } } |
    Out-GuiGridView -Title 'Chosen columns'
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($window = Get-GuiWindow -Title 'Chosen columns') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
$table = (Get-GuiSnapshot -Window $window.Id).Tree.Children | Where-Object Name -eq Grid
$table.Properties.columns.label -join ' | '
```

```text
Name | Id | MemoryMB
```

Numbers, dates and time spans sort by value, so a column such as `MemoryMB` sorts as numbers, not as text.

## Many rows

Rows stream into the window while the pipeline runs: objects arriving quickly are converted and sent 256 at a time, and one that arrives more than a tenth of a second after the last is sent at once. The command reads the window's events between rows, so once the window is closed the objects still arriving are passed over and the command writes nothing.
