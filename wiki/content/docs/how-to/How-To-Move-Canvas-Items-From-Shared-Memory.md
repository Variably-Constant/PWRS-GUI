---
title: How To Move Canvas Items From Shared Memory
weight: 6
---

Set the positions and sizes of a canvas's items from numbers another process writes into shared memory, row by row, with no PowerShell thread between the writer and the window. Source: `crates/pwrs-gui-proto/src/shared.rs`; the module's side, `crates/pwrs-gui/src/shared.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, with the SubEtha module installed. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example's writer is a job, which stands for any process that writes the memory: a simulation, a service, a program in another language.

## How a canvas reads shared memory

A [Canvas](New-GuiCanvas.md)'s `-SharedMemory` names where the numbers are, their type, the items they go to and the keys they set: row i sets the keys of the i-th item `Item` names, and column j sets the key `Property` names j-th. The renderer reads the memory on a thread of its own, woken by the SubEtha notifier set the writer signals after each change, or every `Interval` seconds, or both, and a row changes its item only when its numbers change. [New-GuiCanvas](New-GuiCanvas.md) lists every key.

Where the numbers live:

| Writer | Gives | Read |
|---|---|---|
| SubEtha, from any language it supports | `Source`: a `SubEtha.Slab` or `SubEtha.Vec`, each slot a row, or a `SubEtha.Cell` with the rows end to end | the slots whose version moved, each whole |
| any other | `Path`, the file, with `Offset`, `Length`, `Sequence` and, when wanted, `Shape` | the whole array, used only when the 64-bit counter at `Sequence` reads the same even number before and after |

The writer outside SubEtha makes its counter odd before it writes and even after, so the canvas never draws a half-written array.

## Move items from a SubEtha slab

The writer sets a slot for each item, and signals the notifier set once its rows are written. The wildcard `Planet*` takes a row for each item it matches, in the order the canvas draws them:

```powershell
Import-Module SubEtha
$base = Join-Path ([IO.Path]::GetTempPath()) "planets-$PID"
$null = New-Item -ItemType Directory -Path $base
$slab = New-SubEthaSlab -Path (Join-Path $base 'rows') -Capacity 2 -ElementSize 16
$set = New-SubEthaNotifierSet -Path (Join-Path $base 'notify')
$w = Show-GuiWindow -NoWait -Title 'Planets' {
    Canvas -Name Sky -Width 300 -Height 150 -SharedMemory @{ Source = $slab; Type = 'Float64'; Item = 'Planet*'; Property = 'X', 'Y'; Notifier = $set } @(
        @{ Kind = 'Ellipse'; Name = 'PlanetA'; X = 0; Y = 0; Width = 20; Height = 20; Fill = 'SteelBlue' }
        @{ Kind = 'Ellipse'; Name = 'PlanetB'; X = 0; Y = 0; Width = 12; Height = 12; Fill = 'Tomato' }
    )
}
$writer = Start-Job -ArgumentList $base {
    param($base)
    Import-Module SubEtha
    $slab = New-SubEthaSlab -Path (Join-Path $base 'rows') -Capacity 2 -ElementSize 16
    $set = New-SubEthaNotifierSet -Path (Join-Path $base 'notify')
    $row = { param([double]$x, [double]$y) [byte[]]([BitConverter]::GetBytes($x) + [BitConverter]::GetBytes($y)) }
    $slab.Set(0, (& $row 100 40))
    $slab.Set(1, (& $row 200 90))
    $null = $set.Signal()
}
$null = Wait-Job $writer
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.items)[1].x -ne 200 -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.items | ForEach-Object { '{0} at {1}, {2}' -f $_.name, $_.x, $_.y }
Close-GuiWindow -Window $w.Id
Remove-Job $writer
$slab.Dispose()
$deadline = [DateTime]::UtcNow.AddSeconds(10)
do { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction SilentlyContinue } while ((Test-Path -LiteralPath $base) -and [DateTime]::UtcNow -lt $deadline)
```

```text
PlanetA at 100, 40
PlanetB at 200, 90
```

A `SubEtha.Vec` is read the same way, each live element a row, so a writer that pushes and pops elements gives as many rows as it has elements. Each slot has one value of `Type` for each name in `Property`, and a slab or vec whose slots are another size is refused when the canvas is made. The renderer opens the structure read-only with the `Alignment` and `Tag` its object gives.

## Move items from a file another program writes

A writer outside SubEtha writes its numbers to a file: here a counter at byte 0 and three 32-bit floats from byte 8, each a bar's height. The canvas reads it every tenth of a second:

```powershell
$file = Join-Path ([IO.Path]::GetTempPath()) "levels-$PID.bin"
[IO.File]::WriteAllBytes($file, [byte[]]::new(8 + 3 * 4))
$w = Show-GuiWindow -NoWait -Title 'Levels' {
    Canvas -Name Bars -Width 300 -Height 100 -SharedMemory @{ Path = $file; Sequence = 0; Offset = 8; Length = 3; Type = 'Float32'; Item = 'Bar*'; Property = 'Height'; Interval = 0.1 } @(
        @{ Kind = 'Rect'; Name = 'Bar1'; X = 20; Y = 0; Width = 60; Height = 0; Fill = 'SeaGreen' }
        @{ Kind = 'Rect'; Name = 'Bar2'; X = 120; Y = 0; Width = 60; Height = 0; Fill = 'SeaGreen' }
        @{ Kind = 'Rect'; Name = 'Bar3'; X = 220; Y = 0; Width = 60; Height = 0; Fill = 'SeaGreen' }
    )
}
$writer = Start-Job -ArgumentList $file {
    param($file)
    $stream = [IO.File]::Open($file, 'Open', 'ReadWrite', 'ReadWrite')
    try {
        $put = { param([long]$at, [byte[]]$bytes) $stream.Position = $at; $stream.Write($bytes, 0, $bytes.Length); $stream.Flush() }
        & $put 0 ([BitConverter]::GetBytes([uint64]1))
        & $put 8 ([byte[]](([float]30, [float]80, [float]55) | ForEach-Object { [BitConverter]::GetBytes($_) }))
        & $put 0 ([BitConverter]::GetBytes([uint64]2))
    } finally {
        $stream.Dispose()
    }
}
$null = Wait-Job $writer
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.items)[2].height -ne 55 -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.items | ForEach-Object { '{0}: {1}' -f $_.name, $_.height }
Close-GuiWindow -Window $w.Id
Remove-Job $writer
Remove-Item -LiteralPath $file
```

```text
Bar1: 30
Bar2: 80
Bar3: 55
```

`Shape`, the rows and the columns, may be given beside `Length` to say how the values are laid out, and is refused when it does not match `Property`. The counter is read low byte first, as every machine PWRS-GUI runs on stores it.

## See a source the canvas cannot read

A file missing, or too short for its values and counter, is refused when the canvas is made, naming the file and the sizes; the file's path is left out here:

```powershell
$short = Join-Path ([IO.Path]::GetTempPath()) "short-$PID.bin"
[IO.File]::WriteAllBytes($short, [byte[]]::new(8))
try {
    New-GuiCanvas -SharedMemory @{ Path = $short; Sequence = 0; Offset = 8; Length = 3; Type = 'Float32'; Item = 'Bar*'; Property = 'Height'; Interval = 0.1 }
} catch {
    $_.FullyQualifiedErrorId
    $_.Exception.Message.Replace($short, '<file>')
}
Remove-Item -LiteralPath $short
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiCanvasCommand
SharedMemory: the file <file> has 8 bytes, shorter than its values and counter, which end at byte 20
```

A problem once the window is open is written as a warning by the window's command, as a problem with a ring's items is, and the numbers last read stay drawn. Such a problem is a file that shrinks, a counter the writer leaves odd for a second or more, a structure the renderer cannot open, or a row an item cannot take.
