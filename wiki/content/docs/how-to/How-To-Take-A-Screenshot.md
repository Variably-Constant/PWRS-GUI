---
title: How To Take A Screenshot
weight: 8
---

Save a window, or one widget of it, as a PNG file. Source: `GetGuiSnapshot` in `crates/pwrs-gui/src/inspect.rs`, and `deliver_shots` in `crates/pwrs-gui-egui/src/lib.rs`, which crops a widget from the window.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths.

## The pixels

`Get-GuiSnapshot -Pixels` gives the pixels a window draws next as `Width`, `Height` and `Pixels`: four bytes a pixel, red, green, blue and alpha, rows top to bottom. `-Name` crops them to one widget. A window that is minimized or covered is not drawn, and a request for its pixels is an error, so a window being captured must be visible.

## Save them as PNG on Windows

System.Drawing, which both Windows hosts load, writes PNG files. It stores pixels as blue, green, red and alpha, so the function swaps the first and third byte of each pixel:

```powershell
function Save-SnapshotPng($Shot, [string]$Path) {
    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::new([int]$Shot.Width, [int]$Shot.Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $bounds = [System.Drawing.Rectangle]::new(0, 0, $bitmap.Width, $bitmap.Height)
    $locked = $bitmap.LockBits($bounds, [System.Drawing.Imaging.ImageLockMode]::WriteOnly, $bitmap.PixelFormat)
    $bgra = [byte[]]::new($Shot.Pixels.Length)
    for ($i = 0; $i -lt $bgra.Length; $i += 4) {
        $bgra[$i] = $Shot.Pixels[$i + 2]
        $bgra[$i + 1] = $Shot.Pixels[$i + 1]
        $bgra[$i + 2] = $Shot.Pixels[$i]
        $bgra[$i + 3] = $Shot.Pixels[$i + 3]
    }
    [System.Runtime.InteropServices.Marshal]::Copy($bgra, 0, $locked.Scan0, $bgra.Length)
    $bitmap.UnlockBits($locked)
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
}
```

A dialog shown from a second runspace, as a test would show it, then the whole window and its OK button saved:

<!-- keep: deploy.png images/how-to-screenshot-window.png -->
<!-- keep: ok.png images/how-to-screenshot-ok.png -->
<!-- varies: file sizes -->
```powershell
$dialog = [powershell]::Create().AddScript({
    Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production' -Default 'staging' -Title 'Deploy'
})
$running = $dialog.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($window = Get-GuiWindow -Title 'Deploy') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
Save-SnapshotPng (Get-GuiSnapshot -Window $window.Id -Pixels) (Join-Path $PWD 'deploy.png')
Save-SnapshotPng (Get-GuiSnapshot -Window $window.Id -Name Ok) (Join-Path $PWD 'ok.png')
Get-Item .\deploy.png, .\ok.png | Select-Object Name, Length
```

```text
Name       Length
----       ------
deploy.png   6498
ok.png        812
```

The two files, as the run above wrote them:

![The Deploy window: the prompt "Deploy to", the choices staging and production with staging selected, and OK and Cancel](/images/how-to-screenshot-window.png)

![The OK button alone](/images/how-to-screenshot-ok.png)

Close the dialog when done:

```powershell
Invoke-GuiTestEvent -Window $window.Id -Name Cancel -Event Click
$dialog.EndInvoke($running)
$dialog.Dispose()
```

## On Linux, FreeBSD and macOS

`Get-GuiSnapshot -Pixels` works the same there, but .NET's System.Drawing runs on Windows alone, so the function above does not. The bytes are plain RGBA rows that any PNG encoder takes as they are.

## Compare against a reference

A test can save a PNG as its reference and compare a fresh capture with it byte for byte, or within a tolerance per channel, reading the reference back through `[System.Drawing.Bitmap]::new($path)` on Windows. A capture is in the window's physical pixels, so its size follows the display's scale: the 420-point window above is 420 pixels wide at 100%, and a reference taken at one scale does not match a capture at another.
