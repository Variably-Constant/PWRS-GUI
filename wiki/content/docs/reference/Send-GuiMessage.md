---
title: Send-GuiMessage
weight: 4
---

Sends a value to the page of an HTML window, for the handlers it registered with `window.pwrs.on`. Source: `crates/pwrs-gui/src/page.rs`, `SendGuiMessage`; the renderer's side is `deliver_to_page` in `crates/pwrs-gui-web/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Send-GuiMessage -Syntax
```

```text
Send-GuiMessage [-Window] <ulong> [-Message] <Object> [<CommonParameters>]
```

## Description

`Send-GuiMessage` converts `-Message` to JSON as [Show-GuiWindow](Show-GuiWindow.md) converts a handler's output, as plain data to a depth of 100, sends it to the window's page, and waits for the renderer to take it. Each function the page registered with `window.pwrs.on` is called with the value. A value sent before the page has loaded waits for it.

The renderer refuses a window that is not open, a window that shows widgets rather than a page of its own, and text that is not JSON, and the command writes a non-terminating `GuiRefused`, so a pipeline of several windows goes on with the rest. The window is found by its number, as [Get-GuiWindow](Get-GuiWindow.md) and [Show-GuiWindow](Show-GuiWindow.md) `-NoWait` write it, in any runspace of the process.

## Parameters

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | by property name, as `Window` or `Id` |
| Aliases | `Id` |

The window's number.

### -Message

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: mandatory, position 1 |
| Pipeline input | no |

The value to send. It crosses as JSON, so a hashtable arrives as a plain object and a date as a string.

## Inputs

Objects with an `Id` or `Window` property, such as those `Get-GuiWindow` and `Show-GuiWindow -NoWait` write.

## Outputs

None.

## Examples

The examples from the command's help, on a page that shows each value it gets and closes on `bye`:

<!-- varies: the window's number -->
```powershell
$page = '<p id="got"></p><script>window.pwrs.on(function (v) { if (v === "bye") { window.pwrs.close(); } else { document.getElementById("got").textContent = JSON.stringify(v); } });</script>'
$w = Show-GuiWindow -Html $page -Title 'Report' -NoWait
Send-GuiMessage -Window $w.Id -Message @{ Progress = 50 }
Get-GuiWindow -Title Report | Send-GuiMessage -Message 'refresh'
Send-GuiMessage -Window $w.Id -Message 'bye'
```

## Errors

A window of widgets takes no value for a page. `Out-GuiGridView` returns before its window has opened, and [Get-GuiWindow](Get-GuiWindow.md) lists a window once it has, so the example waits for it:

```powershell
1..2 | Out-GuiGridView -Title 'Two rows'
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($grid = Get-GuiWindow -Title 'Two rows') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
Send-GuiMessage -Window $grid.Id -Message 'hello' -ErrorVariable refused -ErrorAction SilentlyContinue
$refused.FullyQualifiedErrorId
Close-GuiWindow -Window $grid.Id
```

```text
GuiRefused,Pwrs.Modules.PWRS_GUI.SendGuiMessageCommand
```

`GuiNoSession` when no window of the HTML renderer has opened in the process. The [Error Reference](Error-Reference.md) lists every error the module raises.
