---
title: Why PWRS-GUI
weight: 1
---

What PowerShell offers a script that needs to show a window, and what this module adds. Sources: Microsoft's [Out-GridView reference](https://learn.microsoft.com/powershell/module/microsoft.powershell.utility/out-gridview?view=powershell-7.6), and the module's code, `crates/`.

## What PowerShell has

- **Out-GridView** shows objects in a table and returns the rows picked. Microsoft's reference says it "is only available on Windows systems that support the Windows Desktop", and points to the ConsoleGuiTools module, which draws a grid in the terminal, for other systems. It is in Windows PowerShell 5.1 and was reintroduced in PowerShell 7.
- **Console prompts**: `Read-Host`, `$Host.UI.PromptForChoice` and the `-Confirm` questions appear in the console window, as text.
- **Windows Forms and WPF**, loaded with `Add-Type`, build any window a script can describe in .NET code, on Windows only, where the script creates the window on a thread in a single-threaded apartment and runs its message loop.

None of them comes with commands a test can use to drive or read the windows it shows, and none draws the same window on Windows, Linux, FreeBSD and macOS.

## What PWRS-GUI adds

- **The same commands in both hosts.** Windows PowerShell 5.1 and PowerShell 7.4 or later take the same module and run the same commands; [Tested Platforms](Tested-Platforms.md) lists the systems each has run on.
- **Ready-made windows as cmdlets.** `Read-GuiChoice` and `Out-GuiGridView` take objects from the pipeline and write the chosen objects back, with Out-GridView's parameters on the grid.
- **Windows a test can drive.** `Get-GuiWindow`, `Invoke-GuiTestEvent` and `Get-GuiSnapshot` find any window the session has open, act on it as a person would, and read its widgets, their state and its pixels, which is how this module's own Pester suites test it in both hosts ([How To Test A Window](How-To-Test-A-Window.md)).
- **Drawing on the GPU.** Windows are drawn by egui on wgpu: DirectX 12, Vulkan or OpenGL on Windows, Metal on macOS, Vulkan or OpenGL on Linux and FreeBSD, and Microsoft's WARP or Mesa's software renderers where there is no GPU.
- **A process boundary when wanted.** The renderer runs in a helper process by default, so a window cannot stall PowerShell's own threads, and on Windows it can run in PowerShell's process instead, which is how windows open where policy refuses the helper ([Where Windows Are Drawn](Where-Windows-Are-Drawn.md)).
- **Waits that cost nothing.** A command waiting on its window sleeps until an event, Ctrl+C or the renderer's end wakes it ([Events And Waiting](Events-And-Waiting.md)).
- **One session per renderer.** Every runspace shares each renderer's session, and another Rust module takes the same session through a native entry rather than starting a renderer of its own ([Sharing The Renderers](Sharing-The-Renderers.md)).
- **Measured, on your machine.** `Measure-GuiLatency` times how long a click takes to reach the module, a handler and an engine-event action, on the installed build ([How To Measure Latency](How-To-Measure-Latency.md)).
