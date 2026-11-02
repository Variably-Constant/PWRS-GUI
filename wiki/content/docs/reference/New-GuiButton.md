---
title: New-GuiButton
weight: 38
---

Describes a button, which runs a handler when pressed, closes the window with a result, or both. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiButton`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiButton -Syntax
```

```text
New-GuiButton [-Text] <string> [-Result <Object>] [-Enabled] [-Default] [-Cancel] [-OnClick <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiButton` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a button labeled `-Text`. In a `-Content` block `Button` stands for `New-GuiButton`. Pressing it raises `Click`, whose `Value` is `$null`, which `-OnClick` acts on. With `-Result` the press then closes the window, and Show-GuiWindow's result gives that value as its `Result`, so a dialog's buttons are usually just `-Result`s, in a [ButtonBar](New-GuiButtonBar.md) along the window's bottom.

`-Default` makes the button the window's default: Enter pressed alone presses it, unless the widget with the focus uses Enter itself, and it is drawn in the selection's colors while it is enabled; a disabled default button looks like any disabled button, as Windows and macOS draw one. `-Cancel` makes it the window's cancel button, which Escape pressed alone presses. A window has one default button and one cancel button at most, and a button is not both. In a ButtonBar the two roles also set the button's place, as the system's own dialogs order their buttons.

A button with `-Enabled:$false` is drawn dimmed and takes no press, from a key either, and [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) refuses to press it, as a person cannot. A button that has the focus uses Space and Enter itself, to press it; `-OnKey` gets the other keys. [Update-GuiWidget](Update-GuiWidget.md) sets `Text` and `Enabled`.

## Parameters

### -Text

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The button's text.

### -Result

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the button leaves the window open |

The window's result when the button is pressed, which closes the window; `-OnClick` runs first.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the button can be pressed; `-Enabled:$false` makes it one that cannot.

### -Default

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | not the default button |

Makes the button the window's default, which Enter presses and the selection's colors draw while it is enabled.

### -Cancel

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | not the cancel button |

Makes the button the window's cancel button, which Escape presses.

### -OnClick

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the button is pressed, with the event as its argument and as `$_`.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the button, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the button does not use itself is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the button.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Button`, its `Properties` with `Text`, `Enabled` when it is given, and `Default` or `Cancel` when it is set, and its `Result` the value of `-Result`.

## Examples

A dialog's two buttons, Deploy the default and Cancel the cancel button, Deploy pressed:

<!-- person: $w = Wait-Window 'Deploy'; Invoke-GuiTestEvent -Window $w.Id -Name Deploy -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Deploy' {
    Text 'Deploy to production?' -Style Instruction
    ButtonBar {
        Button 'Deploy' -Name Deploy -Default -Result Deploy
        Button 'Cancel' -Name Cancel -Cancel -Result Cancel
    }
}
$result.Result
```

```text
Deploy
```

A button whose handler relabels it, pressed before the window is closed by another. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Counter'; Invoke-GuiTestEvent -Window $w.Id -Name Count -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Done -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Counter' -HandlerOutput Pipeline {
    Button 'Count' -Name Count -OnClick { Update-GuiWidget Count @{ Text = 'Counted' }; "pressed $($_.Name)" }
    ButtonBar { Button 'Done' -Name Done -Default -Result Done }
}
$out[0]
$out[1].Result
```

```text
pressed Count
Done
```

A disabled button takes no press, from a test either:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Locked' { Button 'Deploy' -Name Deploy -Enabled:$false }
Invoke-GuiTestEvent -Window $w.Id -Name Deploy -Event Click -ErrorAction SilentlyContinue -ErrorVariable refused
$refused.FullyQualifiedErrorId
Close-GuiWindow -Window $w.Id
```

```text
GuiRefused,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand
```

A button is not both the default and the cancel button:

```powershell
try { New-GuiButton 'OK' -Default -Cancel } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiButtonRole,Pwrs.Modules.PWRS_GUI.NewGuiButtonCommand
the button OK is the default or the cancel button, not both
```

## Errors

| Error ID | When |
|---|---|
| `GuiButtonRole` | the button is given both `-Default` and `-Cancel`; Show-GuiWindow writes it for a window with two default or two cancel buttons |

Described in the [Error Reference](Error-Reference.md). [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) writes `GuiRefused` for a press on a disabled button.
