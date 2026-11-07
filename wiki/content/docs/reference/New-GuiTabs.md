---
title: New-GuiTabs
weight: 24
---

Describes tabs, one shown at a time: the Tab widgets its block writes, in order. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiTabs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The tab changes and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be.

## Syntax

```powershell
Get-Command New-GuiTabs -Syntax
```

```text
New-GuiTabs [[-Content] <scriptblock>] [-Selected <string>] [-OnChange <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiTabs` writes a `PwrsGui.Widget` that contains the [Tab](New-GuiTab.md) widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws as a row of titles over the widgets of the tab chosen, the chosen title in the strong text color with a line under it in the selection's color. In a `-Content` block `Tabs` stands for `New-GuiTabs`. The first tab is shown when the window opens, or the one `-Selected` names by its title.

Tabs is an input as well as a layout. Choosing a tab raises `Change` with its title, and the `State` of the window's result and of the handlers' events gives, under the widget's name, the title of the tab shown. [Get-GuiSnapshot](Get-GuiSnapshot.md) gives the tab's number instead, from 0, as it gives a list's rows, and [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) chooses a tab by that number. [Update-GuiWidget](Update-GuiWidget.md) sets `Selected` to a title to show that tab.

The inputs on the tabs not shown do not lose their values, and `State` has them too. A widget on a tab not shown is not drawn, so `Get-GuiSnapshot -Name -Pixels` refuses its pixels.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none |

The block that writes the tabs, each a Tab.

### -Selected

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the first tab |

The title of the tab shown when the window opens, compared without regard to case.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person chooses another tab, with the event as its argument and as `$_`; its `Value` is the tab's title.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the tabs where no widget inside them takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the tabs.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Tabs`, its `Properties` with `Selected` when it is given, and its `Children` the tabs.

## Examples

Tabs that open on the second, then a person choosing the first and pressing Done. With `-HandlerOutput Pipeline` the handler's output comes before the window's result:

<!-- person: $w = Wait-Window 'Settings'; Invoke-GuiTestEvent -Window $w.Id -Name Pages -Event Change -Value 0; Invoke-GuiTestEvent -Window $w.Id -Name Done -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Settings' -HandlerOutput Pipeline {
    Tabs -Name Pages -Selected Advanced -OnChange { "now showing $($_.Value)" } {
        Tab 'General' { TextBox 'db01' -Name Server }
        Tab 'Advanced' { CheckBox 'Verbose' -Name Verbose }
    }
    ButtonBar { Button 'Done' -Name Done -Default -Result Done }
}
$out[0]
$out[1].Result
$out[1].State.Pages
```

```text
now showing General
Done
General
```

The tab shown, by number, before and after `Update-GuiWidget` shows another:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Pages' {
    Tabs -Name Pages { Tab 'General' { Text 'first' }; Tab 'Advanced' { Text 'second' } }
}
(Get-GuiSnapshot -Window $w.Id).State.Pages
Update-GuiWidget -Window $w.Id -Name Pages -Property @{ Selected = 'Advanced' }
(Get-GuiSnapshot -Window $w.Id).State.Pages
Close-GuiWindow -Window $w.Id
```

```text
0
1
```

A Tabs contains tabs alone:

```powershell
try { New-GuiTabs { New-GuiText 'not a tab' } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNotATab,Pwrs.Modules.PWRS_GUI.NewGuiTabsCommand
a Tabs contains only Tab widgets, and its block wrote a Text
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotATab` | the block writes a widget other than a Tab |
| `GuiBadProperty` | `-Selected` names no tab |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
