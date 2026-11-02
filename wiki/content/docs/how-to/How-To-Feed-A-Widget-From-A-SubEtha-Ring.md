---
title: How To Feed A Widget From A SubEtha Ring
weight: 6
---

Move a progress bar, add a log's lines or a chart's points, or change a canvas's items from another process, through a SubEtha ring the widget reads itself. Source: `crates/pwrs-gui-proto/src/source.rs`; the module's side, `ring_of` in `crates/pwrs-gui/src/widgets.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, with the SubEtha module installed. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example's writer is a job, which stands for any process that writes to the ring: a worker script, a service, a program in another language.

## How a widget reads a ring

A [ProgressBar](New-GuiProgressBar.md), a [LogView](New-GuiLogView.md), a [Chart](New-GuiChart.md) and a [Canvas](New-GuiCanvas.md) each take `-Ring`: the object `New-SubEthaRing`, `New-SubEthaLocaleRing` or `New-SubEthaBroadcastRing` wrote. The module reads the ring's path and sizes from it, and the renderer opens the ring again by them and reads it on a thread of its own, which the ring wakes when an item arrives, so nothing polls and no PowerShell thread is involved. Each item changes the widget as the update commands would:

| Widget | An item is | Changes it as |
|---|---|---|
| ProgressBar | a JSON object of any of `Value`, `Maximum` and `Text` | `Update-GuiWidget` |
| LogView | one line of text, its line break at the end dropped | `Add-GuiItem` |
| Chart | a JSON object of a point's `Series`, `Y` and `X`, a number or a date and time written as ISO 8601 | `Add-GuiItem` |
| Canvas | a JSON object of the `Item` it changes, by its name, and the keys it changes, or an array of such objects; an Image's picture as `Path`, the full path of its file, or `Bytes`, its bytes in base64 | `Update-GuiWidget -Item` |

Keys may be in any case. An item the widget cannot take is written as a warning naming the item and why, and the widget shows what it showed before; the items after it are read as usual. A canvas's item is checked whole, so one with a mistake in any of its changes makes none of them.

Which ring to make:

| Ring | Items | Readers |
|---|---|---|
| `New-SubEthaRing` | sent with `SendFrame`, of any length | each item goes to one reader |
| `New-SubEthaLocaleRing` | sent with `Send`, at most one slot; the writer moves the ring to `File` or `ShmFs` first, since a locale ring starts in its writer's own memory | each item goes to one reader |
| `New-SubEthaBroadcastRing` | pushed with `Push`, at most 52 bytes | every reader gets every item, up to 16 readers |

A ring made with order stamps (`-Stamps`, `-Stamped`) and a capacity ring (`New-SubEthaCapacityRing`) are refused, the second because a reader in another process does not follow its resizes. `-Ring` is taken when the window opens; `Update-GuiWidget` cannot change it.

## Move a progress bar from a worker

The worker sends a JSON object as each step ends; the bar shows the last one. The loop gives up after 20 seconds:

```powershell
Import-Module SubEtha
$path = Join-Path ([IO.Path]::GetTempPath()) "steps-$PID"
$ring = New-SubEthaRing -Path $path -Capacity 64
$w = Show-GuiWindow -NoWait -Title 'Steps' { ProgressBar -Name Steps -Maximum 3 -Ring $ring }
$worker = Start-Job -ArgumentList $path {
    param($path)
    Import-Module SubEtha
    $ring = New-SubEthaRing -Path $path -Capacity 64
    $producer = $ring.RegisterProducer()
    $steps = 'fetch', 'build', 'test'
    for ($i = 0; $i -lt $steps.Count; $i++) {
        $null = $ring.SendFrame($producer, (@{ Value = $i + 1; Text = "$($steps[$i]) done" } | ConvertTo-Json -Compress))
    }
    $ring.Dispose()
}
$null = Wait-Job $worker
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while ((Get-GuiSnapshot -Window $w.Id).Tree.Properties.value -ne 3 -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
$bar = (Get-GuiSnapshot -Window $w.Id).Tree.Properties
'{0} of {1}: {2}' -f $bar.value, $bar.maximum, $bar.text
Close-GuiWindow -Window $w.Id
$ring.Dispose()
Remove-Job $worker
Remove-Item -Path "$path*" -Force
```

```text
3 of 3: test done
```

## Show a log every window reads

A broadcast ring gives every reader every line, so two windows can show one log. A reader gets what is pushed after it starts reading, so the writer waits for its readers first:

```powershell
Import-Module SubEtha
$path = Join-Path ([IO.Path]::GetTempPath()) "log-$PID"
$ring = New-SubEthaBroadcastRing -Path $path -Capacity 64
$w = Show-GuiWindow -NoWait -Title 'Deploy log' { LogView -Name Log -Height 120 -Ring $ring }
$writer = Start-Job -ArgumentList $path {
    param($path)
    Import-Module SubEtha
    $ring = New-SubEthaBroadcastRing -Path $path -Capacity 64
    $null = $ring.WaitForConsumers(1, 20)
    foreach ($line in 'copying files', 'restarting the service', 'done') { $null = $ring.Push($line) }
    $ring.Dispose()
}
$null = Wait-Job $writer
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.lines).Count -lt 3 -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.lines
Close-GuiWindow -Window $w.Id
$ring.Dispose()
Remove-Job $writer
Remove-Item -Path "$path*" -Force
```

```text
copying files
restarting the service
done
```

## Plot points from a locale ring

A locale ring can live in a file or in named shared memory and move between them while it runs. It starts in its writer's own memory, so the writer moves it to `File` or `ShmFs` before the window reads it; a window whose ring moves back into a writer's memory says so in a warning and reads again once the ring moves out:

```powershell
Import-Module SubEtha
$path = Join-Path ([IO.Path]::GetTempPath()) "load-$PID"
$ring = New-SubEthaLocaleRing -Path $path -Capacity 64
$ring.MigrateTo('File')
$w = Show-GuiWindow -NoWait -Title 'Load' { Chart -Name Load -Height 160 -Ring $ring }
$sampler = Start-Job -ArgumentList $path {
    param($path)
    Import-Module SubEtha
    $ring = New-SubEthaLocaleRing -Path $path -Capacity 64
    $producer = $ring.RegisterProducer()
    foreach ($y in 12, 40, 33) { $null = $ring.Send($producer, "{""Series"": ""cpu"", ""Y"": $y}") }
    $ring.Dispose()
}
$null = Wait-Job $sampler
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.points).Count -lt 3 -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.points) | ForEach-Object { [pscustomobject]$_ }
Close-GuiWindow -Window $w.Id
$ring.Dispose()
Remove-Job $sampler
Remove-Item -Path "$path*" -Force
```

```text
series     y
------     -
cpu    12.00
cpu    40.00
cpu    33.00
```

Windows PowerShell 5.1 writes the numbers without the two decimal places PowerShell 7.6 gives them:

<!-- powershell -->
```text
series  y
------  -
cpu    12
cpu    40
cpu    33
```

## Move a canvas's items from a worker

Each item changes one item of the canvas or, as an array, several at once:

```powershell
Import-Module SubEtha
$path = Join-Path ([IO.Path]::GetTempPath()) "board-$PID"
$ring = New-SubEthaRing -Path $path -Capacity 64
$w = Show-GuiWindow -NoWait -Title 'Board' {
    Canvas -Name Board -Width 200 -Height 100 -Ring $ring @(
        @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 40; Height = 40; Fill = 'Tomato' }
        @{ Kind = 'Ellipse'; Name = 'Dot'; X = 100; Y = 50; Width = 20; Height = 20 }
    )
}
$mover = Start-Job -ArgumentList $path {
    param($path)
    Import-Module SubEtha
    $ring = New-SubEthaRing -Path $path -Capacity 64
    $producer = $ring.RegisterProducer()
    $null = $ring.SendFrame($producer, '[{"Item": "Box", "X": 120, "Fill": "Gold"}, {"item": "dot", "y": 10}]')
    $ring.Dispose()
}
$null = Wait-Job $mover
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.items)[0].x -ne 120 -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.items | ForEach-Object { '{0} at {1}, {2}' -f $_.name, $_.x, $_.y }
Close-GuiWindow -Window $w.Id
$ring.Dispose()
Remove-Job $mover
Remove-Item -Path "$path*" -Force
```

```text
Box at 120, 0
Dot at 100, 10
```

A canvas's numbers can also come from shared memory another process writes, row by row, as [How To Move Canvas Items From Shared Memory](How-To-Move-Canvas-Items-From-Shared-Memory.md) shows.

## See an item a widget cannot take

The window's command writes a warning naming the item and why. A blocking window writes it as its own warning; a `-NoWait` window, to the console:

```powershell
Import-Module SubEtha
$path = Join-Path ([IO.Path]::GetTempPath()) "bad-$PID"
$ring = New-SubEthaRing -Path $path -Capacity 64
$producer = $ring.RegisterProducer()
$null = $ring.SendFrame($producer, '{"Value": "half"}')
$closer = [powershell]::Create().AddScript({
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not ($w = Get-GuiWindow -Title 'Odd item') -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
    Start-Sleep -Milliseconds 500
    Close-GuiWindow -Window $w.Id
})
$closing = $closer.BeginInvoke()
$result = Show-GuiWindow -Title 'Odd item' { ProgressBar -Name Odd -Ring $ring } -WarningVariable said -WarningAction SilentlyContinue
$closer.EndInvoke($closing)
$closer.Dispose()
"$said"
$ring.Dispose()
Remove-Item -Path "$path*" -Force
```

```text
the window reported: ring: ProgressBar Odd took no item from its ring: Value is "half"; it must be a finite number; the item was "{\"Value\": \"half\"}"
```
