---
title: Get-GuiSnapshot
weight: 7
---

Gets a window's tree, the state of its inputs, the widget with the keyboard focus and, with `-Pixels`, the pixels it last drew. Source: `crates/pwrs-gui/src/inspect.rs`, `GetGuiSnapshot` and `GuiSnapshot`, and `from_node` and `from_value` in `crates/pwrs-gui/src/values.rs`; the renderers' sides are `UiState::snapshot` and `deliver_shots` in `crates/pwrs-gui-egui/src/lib.rs` and `snapshot`, `page_peeks`, `page_copied` and `page`, which sends the snapshots asked for before the page reported its window open, in `crates/pwrs-gui-web/src/lib.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a display at 100% scale. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and its names for unsigned integer types, `uint32` and `uint64` where PowerShell 7 writes `uint` and `ulong`, except where its output is shown too.

## Syntax

```powershell
Get-Command Get-GuiSnapshot -Syntax
```

```text
Get-GuiSnapshot [-Window] <ulong> [-Pixels] [-Name <string>] [<CommonParameters>]
```

## Description

`Get-GuiSnapshot` reads a window the session has open, whichever runspace opened it. It gives the window's tree of widgets, as nested ordered dictionaries, the current value of each input, by the input's name, where each named widget was last drawn, the widget with the keyboard focus, and in a window the HTML renderer draws, the engine drawing it. It also gives the text the window last put on the clipboard, such as a table's rows copied with Ctrl+C, and the move of its window a page of your own last started under a test's input. With `-Pixels` it also takes the pixels the window draws next, as RGBA bytes, rows top to bottom; with `-Name` too, the pixels of that widget alone, cropped from the window at the scale the window is drawn at.

A window that is minimized or covered is not drawn, so it has no pixels to take, and a request for them is a `GuiRefused` error, as is a window or a widget that does not exist. Pixels that no frame of the window has taken 30 seconds after the request are a `GuiRefused` error naming the window. The tree and the state are read whatever the window's state. A window not yet shown is read once it is shown, so its boxes and pixels describe the window a person sees.

## Parameters

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Aliases | `Id` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | by property name (`Window`, `Id`) |

The window's number, as [Get-GuiWindow](Get-GuiWindow.md) writes it in `Id`.

### -Pixels

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |

Takes the window's pixels, or the widget's with `-Name`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |

The widget whose pixels to take, compared without regard to case. Naming one takes its pixels, with or without `-Pixels`.

## Inputs

Any object with a `Window` or `Id` property, such as the `PwrsGui.Window` that `Get-GuiWindow` writes, by property name to `-Window`.

## Outputs

`PwrsGui.Snapshot`, one:

| Property | Type | What it gives |
|---|---|---|
| `Window` | `System.UInt64` | the window's number |
| `Tree` | `System.Object` whose value is a `System.Collections.Specialized.OrderedDictionary` | the window's tree: `Id`, `Kind`, `Name`, `Properties` (an ordered dictionary, by name) and `Children` (an array of the same), in that order, as PowerShell's `[ordered]` orders keys. For a window described in PowerShell, the root is the one widget its block wrote, or a Column with the widgets in it when it wrote several |
| `State` | `System.Collections.IDictionary` | each input's current value under its name, or under its number when it has none |
| `Width` | `System.Nullable[System.UInt32]` | the width of `Pixels`; null without them |
| `Height` | `System.Nullable[System.UInt32]` | the height of `Pixels`; null without them |
| `Pixels` | `System.Byte[]` | RGBA, four bytes a pixel, rows top to bottom; null without them |
| `Bounds` | `System.Object` whose value is a `System.Collections.Specialized.OrderedDictionary` | where each named widget was last drawn, under its name, in tree order: an ordered dictionary of `X`, `Y`, `Width` and `Height`, in points from the top left of the window's inside. A widget not drawn, as one on a tab not shown, is absent |
| `Engine` | `System.Object` whose value is a `System.Collections.Specialized.OrderedDictionary` | in a window the HTML renderer draws, its engine: `Name`, `Version`, the `Folder` it was loaded from, and `Settings`; null in a window egui draws |
| `Focused` | `System.String` | the name of the widget with the keyboard focus, such as one [Set-GuiFocus](Set-GuiFocus.md) gave it, or its number when it has no name; null while no widget of the window has it |
| `Copied` | `System.String` | the text the window last put on the clipboard: a table's rows copied with Ctrl+C, as [New-GuiTable](New-GuiTable.md) describes, or text selected and copied or cut; null until the window has put any there |
| `Moving` | `System.String` | the move of its window that a page of your own last started while [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) `-Input` gave it input: `drag`, or `resize` and the edge, as `resize n`. The system makes no such move under a test, since it would follow the person's own pointer, so this says the page asked for it. Null when the page started none under the last test's input, and in a window of widgets |

A password field's node has `peek` among its `Properties`: true while the field offers the eye that shows its password, as [New-GuiTextBox](New-GuiTextBox.md) describes, and false while it does not.

`Engine.Settings` has each setting of the engine's chrome the HTML renderer makes, as the engine reports it now. WebView2 reports `AreBrowserAcceleratorKeysEnabled`, `IsZoomControlEnabled`, `IsPinchZoomEnabled`, `IsSwipeNavigationEnabled`, `IsStatusBarEnabled`, `AreDefaultContextMenusEnabled` and `AreDevToolsEnabled`; WebKit on macOS `allowsMagnification` and `allowsBackForwardNavigationGestures`; WebKitGTK `enable-back-forward-navigation-gestures`, `enable-developer-extras` and `default-context-menu`. A page shown with `-DevTools` has the developer keys and menus on, and every other setting off.

A dialog started in a second runspace, with a default selected, and found by its title once it has opened:

```powershell
$dialog = [powershell]::Create().AddScript({
    Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production', 'lab' -Default 'production' -Title 'Deploy'
})
$running = $dialog.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($window = Get-GuiWindow -Title 'Deploy') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
$shot = Get-GuiSnapshot -Window $window.Id
$shot
```

```text
Window  : 1
Tree    : {[Id, 1], [Kind, Column], [Name, ], [Properties, System.Collections.Specialized.OrderedDictionary]…}
State   : {[Choice, System.Object[]]}
Width   :
Height  :
Pixels  :
Bounds  : {[Choice, System.Collections.Specialized.OrderedDictionary], [Ok,
          System.Collections.Specialized.OrderedDictionary], [Cancel, System.Collections.Specialized.OrderedDictionary]}
Engine  :
Focused : Choice
Copied  :
Moving  :
```

Windows PowerShell 5.1 shows a dictionary in a list by its keys:

<!-- powershell -->
```text
Window  : 1
Tree    : {Id, Kind, Name, Properties...}
State   : {Choice}
Width   :
Height  :
Pixels  :
Bounds  : {Choice, Ok, Cancel}
Engine  :
Focused : Choice
Copied  :
Moving  :
```

```powershell
$shot | Get-Member -MemberType Properties
```

```text
   TypeName: PwrsGui.Snapshot

Name    MemberType Definition
----    ---------- ----------
Bounds  Property   System.Object Bounds {get;set;}
Copied  Property   string Copied {get;set;}
Engine  Property   System.Object Engine {get;set;}
Focused Property   string Focused {get;set;}
Height  Property   System.Nullable[uint] Height {get;set;}
Moving  Property   string Moving {get;set;}
Pixels  Property   byte[] Pixels {get;set;}
State   Property   System.Collections.IDictionary State {get;set;}
Tree    Property   System.Object Tree {get;set;}
Width   Property   System.Nullable[uint] Width {get;set;}
Window  Property   ulong Window {get;set;}
```

The tree, one line per widget:

```powershell
function Show-Node($Node, $Depth = 0) {
    '{0}{1} {2} {3}' -f ('  ' * $Depth), $Node.Id, $Node.Kind, $Node.Name
    foreach ($child in $Node.Children) { Show-Node $child ($Depth + 1) }
}
Show-Node $shot.Tree
```

```text
1 Column
  2 Text
  3 Spacer
  4 ListBox Choice
  7 ButtonBar
    5 Button Ok
    6 Button Cancel
```

A widget's properties, by name:

```powershell
($shot.Tree.Children | Where-Object Name -eq 'Choice').Properties
```

```text
Name                           Value
----                           -----
items                          {staging, production, lab}
multiple                       False
selected                       {1}
```

## Examples

The examples from the command's help.

The rows selected in the list:

```powershell
(Get-GuiSnapshot -Window $window.Id).State.Choice
```

```text
1
```

The pixels of the OK button:

```powershell
$shot = Get-GuiSnapshot -Window $window.Id -Name Ok -Pixels
'{0} x {1} pixels, {2} bytes' -f $shot.Width, $shot.Height, $shot.Pixels.Length
```

```text
80 x 24 pixels, 7680 bytes
```

The pixels of the whole window:

```powershell
$whole = Get-GuiSnapshot -Window $window.Id -Pixels
'{0} x {1} pixels, {2} bytes' -f $whole.Width, $whole.Height, $whole.Pixels.Length
```

```text
320 x 168 pixels, 215040 bytes
```

[How To Take A Screenshot](How-To-Take-A-Screenshot.md) saves them as a PNG file.

## Errors

A widget the window does not have is `GuiRefused`, naming the widgets it has:

```powershell
Get-GuiSnapshot -Window $window.Id -Name Missing -ErrorVariable failed -ErrorAction SilentlyContinue
$failed.FullyQualifiedErrorId
$failed.Exception.Message
Invoke-GuiTestEvent -Window $window.Id -Name Cancel -Event Click
$dialog.EndInvoke($running)
$dialog.Dispose()
```

```text
GuiRefused,Pwrs.Modules.PWRS_GUI.GetGuiSnapshotCommand
the GUI renderer refused: window 1 has no widget named Missing; its widgets are Choice, Ok, Cancel
```

Before any window has opened in the process there is no renderer for the command to reach, which is `GuiNoSession`. Both are non-terminating; the [Error Reference](Error-Reference.md) describes each error.
