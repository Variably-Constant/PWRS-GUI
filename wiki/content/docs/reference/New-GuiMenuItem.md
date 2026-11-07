---
title: New-GuiMenuItem
weight: 53
---

Describes a menu's item, chosen by a click or by its shortcut. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiMenuItem`; shortcuts, `Shortcut` in `crates/pwrs-gui-proto/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks and keys were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiMenuItem -Syntax
```

```text
New-GuiMenuItem [-Text] <string> [-Shortcut <string>] [-Enabled] [-OnClick <scriptblock>] [-Result <Object>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiMenuItem` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as `-Text` in its [Menu](New-GuiMenu.md) or [ContextMenu](New-GuiContextMenu.md), with `-Shortcut` beside it. In a `-Content` block `MenuItem` stands for `New-GuiMenuItem`. Choosing it raises `Click`, whose `Value` is `$null`, which `-OnClick` acts on; with `-Result` it then closes the window with that result, as a button's does. An item goes only in a Menu or a ContextMenu.

A shortcut is written `Ctrl+Shift+S`: any of Ctrl, Shift and Alt, in any order and case, joined by `+` to one key. The key is a letter, a digit, F1 to F24, Enter, Escape, Tab, Space, Backspace, Delete, Insert, Home, End, PageUp, PageDown, an arrow key, or `Plus`, `Minus` or `Equals`, the keys typing `+`, `-` and `=`. A shortcut is written back with the modifiers in that order. Ctrl is the Command key on macOS, where menus show it as Cmd. A letter or a digit is the one the key types, or, where the key types a symbol with the modifiers pressed, as Shift and 1 types `!` on a US keyboard, the one at the key's place, so `Ctrl+Shift+1` is the 1 key with Ctrl and Shift on any keyboard. Pressing the shortcut in the window acts as a click, before a shortcut of the window's own of the same keys, and only when the keys match exactly. While a text field has the focus, the keys it edits with go to the field, and in a password field Alt+F8 too, which shows the password.

A disabled item, `-Enabled:$false`, is drawn dimmed, raises nothing, and its shortcut does nothing; [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) refuses to click it. [Update-GuiWidget](Update-GuiWidget.md) sets `Text`, `Shortcut` and `Enabled`.

## Parameters

### -Text

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The item's text.

### -Shortcut

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The keys that act as a click, as `Ctrl+S`.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the item can be chosen; `-Enabled:$false` makes it one that cannot.

### -OnClick

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the item is chosen, with the event as its argument and as `$_`.

### -Result

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the item leaves the window open |

The window's result when the item is chosen, which closes the window; `-OnClick` runs first.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the item.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `MenuItem`, its `Properties` with `Text`, and `Shortcut`, as written back, and `Enabled` when they are given, and its `Result` the value of `-Result`.

## Examples

An item chosen by its shortcut, then another by a click. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Editor'; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+S'; Invoke-GuiTestEvent -Window $w.Id -Name Exit -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Editor' -HandlerOutput Pipeline {
    TextBox -Name Body -Multiline
    MenuBar {
        Menu 'File' {
            MenuItem 'Save' -Name Save -Shortcut 'ctrl+s' -OnClick { "saving, chosen by $($_.Kind)" }
            MenuItem 'Exit' -Name Exit -Result Exit
        }
    }
}
$out[0]
$out[1].Result
```

```text
saving, chosen by Click
Exit
```

A shortcut written back in order, a disabled item refused, and a shortcut no key makes:

```powershell
(New-GuiMenuItem 'Save all' -Shortcut 'shift+CTRL+s').Properties.Shortcut
$w = Show-GuiWindow -NoWait -Title 'Locked' { MenuBar { Menu 'File' { MenuItem 'Delete' -Name Delete -Enabled:$false } } }
Invoke-GuiTestEvent -Window $w.Id -Name Delete -Event Click -ErrorAction SilentlyContinue -ErrorVariable refused
$refused.FullyQualifiedErrorId
Close-GuiWindow -Window $w.Id
try { New-GuiMenuItem 'x' -Shortcut 'Ctrl+Meta+S' } catch { $_.FullyQualifiedErrorId }
```

```text
Ctrl+Shift+S
GuiRefused,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand
GuiBadShortcut,Pwrs.Modules.PWRS_GUI.NewGuiMenuItemCommand
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadShortcut` | `-Shortcut` is not written as a shortcut |
| `GuiBadProperty` | `Update-GuiWidget` sets a `Shortcut` not written as one |
| `GuiNotInMenu` | Show-GuiWindow is given an item outside a Menu or a ContextMenu |

Each is described in the [Error Reference](Error-Reference.md); [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) writes `GuiRefused` for a click on a disabled item.
