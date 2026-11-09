---
title: Windows PowerShell 5.1
weight: 9
---

PWRS-GUI runs the same commands in Windows PowerShell 5.1 and in PowerShell 7.4 or later. This page lists each place the two hosts differ in what a script gets, with the page that spells it out. A page that gains a note about Windows PowerShell 5.1 gains a line here.

## Writing a file another program reads

Windows PowerShell 5.1's `Add-Content` and `Set-Content` cannot write to a file another program has open, even only to read it. A Windows PowerShell 5.1 script writing a log with `Add-Content` therefore fails at the moment a reader such as LogLens reads it. `Out-File -Append`, PowerShell 7's `Add-Content` and .NET's `StreamWriter` share the file and never fail so. See [Following A Log](Following-A-Log.md) and [Show-GuiWindow](Show-GuiWindow.md#-watchpath)'s `-WatchPath`.

## A GPU buffer read as memory

`Read-GuiGpuBuffer -AsMemory` writes a `Memory<T>` over the values. Windows PowerShell 5.1 has no `Memory<T>`, so there it writes the array. See [Read-GuiGpuBuffer](Read-GuiGpuBuffer.md).

## Installing the WebView2 add-on

Windows PowerShell 5.1's own PowerShellGet 1.0.0.1 cannot install a package whose license a person accepts at install, which the WebView2 add-on is. A newer PowerShellGet installed beside it can. See [The WebView2 Add-On](The-WebView2-Add-On.md).

## Testing a window

Windows PowerShell 5.1 ships Pester 3.4.0, and the test helpers need Pester 5 or later, installed beside it. See [How To Test A Window](How-To-Test-A-Window.md) and [How To Run The Tests](How-To-Run-The-Tests.md).

## Output formatted differently

PowerShell formats some objects differently in the two hosts, and the pages show Windows PowerShell 5.1's output beside PowerShell 7's where it differs:

- A `[double]` in a table: PowerShell 7 writes it with two decimal places, Windows PowerShell 5.1 without. See [Read-GuiInput](Read-GuiInput.md), [Read-GuiGpuBuffer](Read-GuiGpuBuffer.md) and [How To Feed A Widget From A SubEtha Ring](How-To-Feed-A-Widget-From-A-SubEtha-Ring.md).
- A dictionary in a list, such as a widget's `Properties`: Windows PowerShell 5.1 shows it by its keys. See [Get-GuiSnapshot](Get-GuiSnapshot.md), [New-GuiCanvas](New-GuiCanvas.md) and [New-GuiGpuSurface](New-GuiGpuSurface.md).
- A process: Windows PowerShell 5.1's table view of one has columns of its own, and its own process is named `powershell`. See [Out-GuiGridView](Out-GuiGridView.md) and [Read-GuiChoice](Read-GuiChoice.md).
