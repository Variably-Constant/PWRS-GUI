---
title: Close-GuiWindow
weight: 17
---

Closes a window, giving the command that waits on it a result. Source: `crates/pwrs-gui/src/window.rs`, `CloseGuiWindow`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be.

## Syntax

```powershell
Get-Command Close-GuiWindow -Syntax
```

```text
Close-GuiWindow [[-Window] <ulong>] [-Result <Object>] [<CommonParameters>]
```

## Description

`Close-GuiWindow` closes the window `-Window` names, as a person closing it would, and the command that opened it finishes: a blocking [Show-GuiWindow](Show-GuiWindow.md) writes its result, and a `-NoWait` window's `Close` event waits for [Wait-GuiEvent](Wait-GuiEvent.md). A window described in PowerShell takes `-Result` as its result's `Result`; without it, the result is `Closed`, as when a person closes the window.

Inside a handler, `-Window` may be left out: the handler's window closes, which is how a handler ends its window with a result of its own, as a button's `-Result` does.

A number no open window has is a non-terminating `GuiNoWindow` error, whether or not a renderer is running, so a list of windows piped in goes on past it, and `-ErrorAction Ignore` makes a close that may come second silent. A window `Show-GuiWindow -NoWait` opened from a `-Content` block counts as open until [Wait-GuiEvent](Wait-GuiEvent.md) takes its `Close`. For any other window the command sends the close to its renderer, which refuses it when it has no window of that number open, a window it has not drawn yet counting as open.

## Parameters

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: position 0 |
| Pipeline input | yes, by property name, as `Window` or `Id` |
| Default | inside a handler, the handler's window |

The window's number, as `Show-GuiWindow -NoWait` and [Get-GuiWindow](Get-GuiWindow.md) give it.

### -Result

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: `Closed` |

The value the window's result gives as its `Result`.

## Inputs

An object with a `Window` or `Id` property, such as what `Get-GuiWindow` writes.

## Outputs

None.

## Examples

A handler that ends its window with a result of its own:

<!-- person: $w = Wait-Window 'Save'; Invoke-GuiTestEvent -Window $w.Id -Name Save -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Save' {
    TextBox 'notes.txt' -Name File
    ButtonBar { Button 'Save' -Name Save -Default -OnClick { Close-GuiWindow -Result "saved $($_.State.File)" } }
}
$result.Result
```

```text
saved notes.txt
```

Closing a window found by its title:

```powershell
$null = Show-GuiWindow -NoWait -Title 'Report' { Text 'Done' }
Get-GuiWindow -Title Report | Close-GuiWindow
@(Get-GuiWindow -Title Report).Count
```

```text
0
```

A number no open window has is a non-terminating error, so the command goes on:

```powershell
Close-GuiWindow -Window 4242 -ErrorVariable missed -ErrorAction SilentlyContinue
$missed[0].FullyQualifiedErrorId
$missed[0].Exception.Message
```

```text
GuiNoWindow,Pwrs.Modules.PWRS_GUI.CloseGuiWindowCommand
no window is open with the number 4242
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoWindow` | `-Window` is left out outside a handler, which ends the command; or no window is open with the number `-Window` gives, which is written as a non-terminating error |

Described in the [Error Reference](Error-Reference.md).
