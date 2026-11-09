---
title: New-GuiText
weight: 30
---

Describes a line of text, or a link that runs a handler when clicked. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiText`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiText -Syntax
```

```text
New-GuiText [-Text] <string> [-Style <TextStyle>] [-OnClick <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiText` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as `-Text` in the style `-Style` names:

- `Body`, the window's text;
- `Heading`, larger and bolder;
- `Instruction`, a dialog's main line, 15 points in the strong text's color, as the ready-made dialogs show their prompt;
- `Mono`, in the monospaced font, with its spaces as written;
- `Link`, drawn as a link.

In a `-Content` block `Text` stands for `New-GuiText`.

A link raises `Click` when a person clicks it, which `-OnClick` acts on. Nothing opens on its own: the handler does whatever following the link should do, such as `Start-Process` with an address. The event's `Value` is `$null`; its `Name` is the widget's. `-OnClick` is refused on any other style. [Update-GuiWidget](Update-GuiWidget.md) sets `Text` and `Style`.

## Parameters

### -Text

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none; an empty string is allowed |

The text shown.

### -Style

| | |
|---|---|
| Type | `PwrsGui.TextStyle` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Body` |

`Body`, `Heading`, `Instruction`, `Mono` or `Link`.

### -OnClick

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

With `-Style Link`, runs when the link is clicked, with the event as its argument and as `$_`.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the text, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the text.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Text` and its `Properties` with `Text`, and `Style` when it is given.

## Examples

Four styles, one of them changed while the window is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Styles' {
    Text 'Report' -Style Heading
    Text 'Copy finished' -Style Instruction
    Text 'Three files copied.' -Style Body
    Text 'C:\Logs\today.log' -Style Mono -Name Where
}
Update-GuiWidget -Window $w.Id -Name Where -Property @{ Text = 'C:\Logs\yesterday.log' }
(Get-GuiSnapshot -Window $w.Id).Tree.Children | ForEach-Object { '{0}: {1}' -f $_.Properties.style, $_.Properties.text }
Close-GuiWindow -Window $w.Id
```

```text
heading: Report
instruction: Copy finished
body: Three files copied.
mono: C:\Logs\yesterday.log
```

A link whose handler says what it would open, then the window closed by its button. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Help'; Invoke-GuiTestEvent -Window $w.Id -Name Docs -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Done -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Help' -HandlerOutput Pipeline {
    Text 'Read the documentation' -Style Link -Name Docs -OnClick { "opening the documentation from $($_.Name)" }
    ButtonBar { Button 'Done' -Name Done -Default -Result Done }
}
$out[0]
$out[1].Result
```

```text
opening the documentation from Docs
Done
```

Only a link takes a click handler:

```powershell
try { New-GuiText 'Plain' -OnClick { } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiTextCommand
-OnClick runs when a link is clicked, and only a Text with -Style Link is one
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | `-OnClick` is given without `-Style Link`, or `Update-GuiWidget` sets a `Style` that is not one of the five |

Described in the [Error Reference](Error-Reference.md).
