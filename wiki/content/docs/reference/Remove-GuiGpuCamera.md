---
title: Remove-GuiGpuCamera
weight: 9
---

Takes the camera from a GpuSurface. Source: `crates/pwrs-gui/src/gpu.rs`, `RemoveGuiGpuCamera`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Remove-GuiGpuCamera -Syntax
```

```text
Remove-GuiGpuCamera [-Name] <string> [-Window <ulong>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

## Description

`Remove-GuiGpuCamera` takes the camera [Set-GuiGpuCamera](Set-GuiGpuCamera.md) gave the [GpuSurface](New-GuiGpuSurface.md) named `-Name`, so the view and projection of the surface's frame uniform are identity matrices again and the person's pointer moves nothing. A surface without a camera is left as it is. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotASurface` for a widget of another kind |

The surface's name, compared without regard to case.

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | yes, by property name, as `Window` or `Id` |
| Default | inside a handler, the handler's window |
| Refused with | `GuiNoWindow` outside a handler when not given, or when no window described in PowerShell is open with the number it gives |

The window's number.

### -WhatIf and -Confirm

The common risk-mitigation parameters: `-WhatIf` says what the command would do without doing it, and `-Confirm` waits for the person to confirm first.

## Inputs

An object with a `Window` or `Id` property, such as the window `Show-GuiWindow -NoWait` writes.

## Outputs

None.

## Examples

What the command would take away, and then taking it:

<!-- varies: the window's number -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'View' { GpuSurface View -Width 64 -Height 64 }
Set-GuiGpuCamera View -Window $w.Id
Remove-GuiGpuCamera View -Window $w.Id -WhatIf
Remove-GuiGpuCamera View -Window $w.Id
'{0} cameras left' -f @(Get-GuiGpuCamera View -Window $w.Id).Count
Close-GuiWindow -Window $w.Id
```

```text
What if: Performing the operation "Remove the camera" on target "GpuSurface View in window 1".
0 cameras left
```
