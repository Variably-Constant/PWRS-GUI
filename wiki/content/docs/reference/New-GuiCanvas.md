---
title: New-GuiCanvas
weight: 37
---

Describes a canvas of shapes, text and pictures in both renderers, which the commands, a SubEtha ring or shared memory another process writes change while its window is open. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiCanvas`; its items are read in `crates/pwrs-gui/src/canvas.rs` and checked in `crates/pwrs-gui-proto/src/canvas.rs`, its ring and shared memory read in `crates/pwrs-gui-proto/src/source.rs` and `crates/pwrs-gui-proto/src/shared.rs`, with the module's side of `-SharedMemory` in `crates/pwrs-gui/src/shared.rs`; the egui renderer draws it in `crates/pwrs-gui-egui/src/canvas.rs`, and an HTML window's page in `crates/pwrs-gui-web/src/runtime/runtime.js` and `canvas.js`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070 and a display at 100 percent scale. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiCanvas -Syntax
```

```text
New-GuiCanvas [[-Item] <Object[]>] [-Width <double>] [-Height <double>] [-Ring <Object>] [-SharedMemory <Object>] [-OnClick <scriptblock>] [-OnDoubleClick <scriptblock>] [-OnPointerEnter <scriptblock>] [-OnPointerLeave <scriptblock>] [-OnDragStart <scriptblock>] [-OnDragMove <scriptblock>] [-OnDragEnd <scriptblock>] [-OnPointerDown <scriptblock>] [-OnPointerUp <scriptblock>] [-OnPointerMove <scriptblock>] [-OnWheel <scriptblock>] [-OnKey <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiCanvas` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a canvas `-Width` by `-Height` points, or without them the room its place leaves across and a share of the height its column leaves. In a `-Content` block `Canvas` stands for `New-GuiCanvas`. `-Item` gives its items, hashtables drawn in the order given, the first underneath, and a mistake in one is refused here, naming the item and the key. An egui window draws the items on the GPU, every edge multisampled, and an HTML window's page draws them with Canvas 2D.

### Items

Each item is a hashtable of its `Kind`, its `Name` when an event or an update command is to find it, and its keys. Names are unique in the canvas, and they and the keys are compared without regard to case. Each kind needs the keys of its shape:

| Kind | Needs | Takes besides |
|---|---|---|
| `Rect` | `X`, `Y`, `Width`, `Height` | |
| `RoundedRect` | `X`, `Y`, `Width`, `Height`, `Radius` | |
| `Ellipse` | `X`, `Y`, `Width`, `Height` | |
| `Line` | `X1`, `Y1`, `X2`, `Y2` | |
| `Polyline`, `Polygon` | `Points` | |
| `Path` | `Data` | |
| `Text` | `X`, `Y`, `Text` | `Size` |
| `Image` | `X`, `Y`, `Width`, `Height`, and `Path` or `Bytes` | |
| `Group` | | `X`, `Y`, `Clip`, `Children` |

| Key | What it gives |
|---|---|
| `X`, `Y` | the top left of a Rect's, RoundedRect's, Ellipse's or Image's box, or of a Text, in the canvas's points; for a Group, how far its items move, 0 when not given |
| `Width`, `Height` | the box's size, 0 or more |
| `Radius` | a RoundedRect's corners, quarter circles of that radius, at most half the box's width and height |
| `X1`, `Y1`, `X2`, `Y2` | a Line's two ends |
| `Points` | one list of numbers, x then y for each point, as `0,0, 40,0, 20,30`: two points at least for a Polyline, and three for a Polygon, which closes back to its first |
| `Data` | SVG path data, as `M 0 0 C 0 -40 60 -40 60 0 Z` |
| `Text` | a Text's text, drawn in the window's UI font, a row for each of its lines |
| `Size` | a Text's height in points, above 0; 14 when not given |
| `Path`, `Bytes` | an Image's picture, a file or its bytes, as [New-GuiImage](New-GuiImage.md) takes them; the picture is stretched to the box |
| `Clip` | a Group's clip: a hashtable of a `Rect`, `RoundedRect`, `Ellipse`, `Polygon` or `Path` by its `Kind` and the keys above, in the group's own coordinates, so it moves and turns with the group |
| `Children` | a Group's items, as `-Item` takes them |

Every kind takes the style keys:

| Key | What it gives |
|---|---|
| `Fill` | a color, or a gradient; a Line is never filled |
| `Stroke` | the outline's color |
| `StrokeWidth` | the outline's width in points, 0 or more; 1 when not given |
| `Dash` | the lengths of a dash and a gap, repeated, as `6,3`; an empty list is a solid stroke |
| `Cap` | how an open outline's ends are drawn: `Butt`, `Round` or `Square` |
| `Join` | how its corners are drawn: `Miter`, `Round` or `Bevel` |
| `Opacity` | from 0 to 1; 1 when not given |
| `Rotate` | a turn in degrees, clockwise |
| `Scale` | one number for both directions, or an x,y pair |
| `Skew` | degrees along x, or an x,y pair, each less than 90 either way |
| `Origin` | the point `Rotate`, `Scale` and `Skew` act about, as fractions of the item's box, `0,0` its top left; the box's middle when not given, and for a Group the point its `X` and `Y` give |
| `Visible` | `$false` to draw the item not at all, and give it no events |
| `Order` | a whole number: among the items beside it, a higher Order is drawn above, and items of one Order in the order given |
| `Animate` | an animation, or a list of them |

A color is one of CSS's 148 names in any case, such as `Tomato`, `Transparent`, or `#rgb`, `#rrggbb` or `#rrggbbaa`. A gradient `Fill` is a hashtable, `@{ Linear = 'Tomato', 'Gold'; From = 0,0; To = 1,0 }` or `@{ Radial = 'White', 'Navy'; Center = 0.5,0.5; Radius = 0.5 }`, its points in fractions of the item's box. `From` and `To` are 0,0 and 1,0, and `Center` and `Radius` 0.5,0.5 and 0.5, when not given; its colors are spread evenly unless `Stops` gives an offset from 0 to 1 for each, none before the one ahead of it.

Each of `Fill`, `Stroke`, `StrokeWidth`, `Dash`, `Cap` and `Join` an item does not set comes from its nearest Group that sets it. An item with neither a `Fill` nor a `Stroke`, of its own or from its Groups, is outlined 1 point wide in the theme's text color, and a Text is filled in it; `Fill` alone fills an item with no outline, and `Stroke` alone outlines it with no fill. A Group's `Opacity` fades it as one layer, so its items do not show through each other, and its `Visible`, its transforms and its `Clip` act on all its items.

### Animations

`Animate` moves one of an item's numbers or colors without a call from the script for each frame: `@{ Property = 'X'; To = 120; Duration = 2 }`, with `Easing`, `Repeat` and `Reverse` when wanted.

| Key | What it gives |
|---|---|
| `Property` | the key it moves: one of the numbers its kind's keys give, as `X`, `Width`, `Radius`, `X1` or `Size`, or `Fill` when it is a color, `Stroke`, `StrokeWidth`, `Opacity`, `Rotate`, `Scale`, `Skew` or `Origin` |
| `To` | where it moves it, a value the key takes |
| `Duration` | the seconds a pass takes, above 0, as a number or a [TimeSpan] |
| `Easing` | the curve it follows: `Linear`, `Ease`, `EaseIn`, `EaseOut` or `EaseInOut`, CSS's curves, or one of Penner's, `EaseIn`, `EaseOut` or `EaseInOut` followed by `Sine`, `Quad`, `Cubic`, `Quart`, `Quint`, `Expo`, `Circ`, `Back`, `Elastic` or `Bounce`, as `EaseInOutCubic`; `Ease` when not given |
| `Repeat` | how many passes, 1 or more, or `Forever`; 1 when not given |
| `Reverse` | `$true` to play each pass forward and then back |

It starts from the value drawn when the item comes onto the canvas, and when its passes end the key stays where the last one ended: at `To`, or back at its start after a `Reverse` pass. A list of animations moves several keys, each in one of them.

### Changes while the window is open

[Update-GuiWidget](Update-GuiWidget.md) `-Item` changes one item, found by its name at any depth: the keys `-Property` gives join the item's, a `$null` taking one away, and the item is checked again as `-Item` is. Its `Kind`, `Name` and a Group's items stay as made. A key set this way ends any animation of it, and the new value is drawn at once; an `Animate` starts each of its animations from the value drawn then, and replaces only an animation of the same key, while `Animate = $null` ends them all. [Add-GuiItem](Add-GuiItem.md) adds items above the canvas's own, or with `-Parent` above a Group's own, and [Remove-GuiItem](Remove-GuiItem.md) takes items away by name, a Group with its items. `Update-GuiWidget -Property @{ Item = ... }` gives the canvas new items in place of all it had, and `Width` and `Height` change its size.

A snapshot's `Tree` gives the canvas's items as the window has them after every change, each as a dictionary of its keys in lower case.

### Events

The item handlers run for the topmost named item at the pointer, and an item without a name of its own takes events under the name of its nearest named Group. A point is on an item when it is inside a closed outline, filled or not: a Rect's, RoundedRect's, Ellipse's or Polygon's, or a Path's part that ends in `Z`. Inside an open Polyline or a Path's open part, a point counts only where its fill paints. A point on an item's outline, or within half its stroke's width or 5 points of it, whichever is more, is on it too. A Text and an Image take events across their boxes.

| Handler | Runs when |
|---|---|
| `-OnClick` | a named item is clicked: a press let go within 6 points and 0.8 seconds |
| `-OnDoubleClick` | a named item is clicked again within 0.3 seconds and 6 points of a click |
| `-OnPointerEnter`, `-OnPointerLeave` | the pointer comes over a named item, or leaves the one it was over |
| `-OnDragStart` | a press on a named item goes past 6 points, or lasts past 0.8 seconds |
| `-OnDragMove` | the pointer moves while the drag goes on, once a frame at most |
| `-OnDragEnd` | the drag's button is let go |
| `-OnPointerDown`, `-OnPointerUp`, `-OnPointerMove`, `-OnWheel` | as a [GpuSurface](New-GuiGpuSurface.md)'s handlers of those names, for the canvas itself |
| `-OnKey` | a key is pressed while the canvas has the focus, and its view's controls do not take the key |
| `-OnDrop` | files or text are dropped on the canvas |

Each handler is given the event, which in a window opened with `-NoWait` [Wait-GuiEvent](Wait-GuiEvent.md) also reads. For every event but a key and a drop, the event's `Value` gives:

| Property | What it gives |
|---|---|
| `X`, `Y` | where, in physical pixels from the canvas's top left |
| `U`, `V` | the same place as fractions of the canvas's width and height |
| `PixelsPerPoint` | the display's scale |
| `Button` | `Left`, `Right` or `Middle`; `Left` for an item's click and drag, and null for the pointer coming and going, a move with no button pressed and a wheel turn |
| `Shift`, `Ctrl`, `Alt` | the modifier keys pressed, Ctrl counting Command on macOS |
| `Notches` | for a wheel turn, the notches turned, positive away from the person |
| `Item` | the name of the topmost named item at the pointer, or for a drag the item dragged; null where no named item is |
| `SceneX`, `SceneY` | the pointer in the items' coordinates, under the canvas's view |

A key's `Value` is the key written as a shortcut, as `Ctrl+K`, and a drop's the paths dropped, or the text.

### The view

A canvas has no view until [Set-GuiCanvasView](Set-GuiCanvasView.md) gives it one: its items' coordinates are its own points, and no gesture moves them. A view puts a point of the items' coordinates at the canvas's middle at a zoom, and gives the person controls to pan and zoom with, Leaflet's or Figma's, gestures of the script's own, or none. Hit tests, `SceneX` and `SceneY` follow it. [Get-GuiCanvasView](Get-GuiCanvasView.md) reads it as the person left it, and [Remove-GuiCanvasView](Remove-GuiCanvasView.md) takes it away.

With `-OnKey`, or a view whose controls take keys, the canvas joins the window's Tab order and takes the focus when pressed, and a ring shows a focus the keyboard gave it.

### From another process

`-Ring` takes a SubEtha ring another process writes: a `SubEtha.Ring` whose writer sends items with `SendFrame`, a `SubEtha.LocaleRing` moved to `File` or `ShmFs`, or a `SubEtha.BroadcastRing`. Each item is a JSON object of the `Item` it changes, by its name, and the keys it changes, or an array of such objects, and changes the canvas as `Update-GuiWidget -Item` does; its keys may be in any case at any depth. An Image's picture comes as `Path`, the full path of its file, or as `Bytes`, its bytes written in base64. An item with a mistake is refused whole, none of its changes made, and the window's command writes a warning naming it and why. A ring made with order stamps, and a `SubEtha.CapacityRing`, are refused. [How To Feed A Widget From A SubEtha Ring](How-To-Feed-A-Widget-From-A-SubEtha-Ring.md) has the writer's side.

`-SharedMemory` takes values another process writes, which set the numbers of named items row by row: row i sets the keys of the i-th item `Item` names, column j the key `Property` names j-th. It is a hashtable of `Source`, or for a writer outside SubEtha of `Path`, `Offset`, `Length` and `Sequence` and, when wanted, `Shape`; with `Type`, `Item` and `Property`; and `Notifier`, `Interval` or both. A key missing is refused, naming it:

| Key | What it gives |
|---|---|
| `Source` | a `SubEtha.Slab` or `SubEtha.Vec`, each of its slots a row, or a `SubEtha.Cell`, its value the rows end to end, as `New-SubEthaSlab`, `New-SubEthaVec` and `New-SubEthaCell` write them; a slot has one value of `Type` for each name in `Property`, and a cell's value a whole number of such rows |
| `Path` | in place of `Source`, the file another process writes, resolved from the current location |
| `Offset` | the byte of the file its values start at |
| `Length` | how many values it has, a whole number of rows |
| `Shape` | its rows and columns, the columns one for each name in `Property` |
| `Sequence` | the byte of a 64-bit counter, low byte first, which the writer makes odd before it writes and even after; its eight bytes are outside the values |
| `Type` | the values' type: `Float16`, `Float32`, `Float64`, `Int8`, `UInt8`, `Int16`, `UInt16`, `Int32`, `UInt32`, `Int64` or `UInt64`, the element types [New-GuiGpuBuffer](New-GuiGpuBuffer.md) takes, low byte first |
| `Item` | the items the rows go to, in order: names, each taking one row whether or not the canvas has such an item, and wildcards, each taking a row for every named item it matches, in the order the canvas draws them; an item already given a row stays with its first |
| `Property` | the key each column sets, one that takes a single number: `X`, `Y`, `Width`, `Height`, `Radius`, `X1`, `Y1`, `X2`, `Y2`, `Size`, `StrokeWidth`, `Opacity`, `Rotate`, `Scale`, `Skew` or `Order` |
| `Notifier` | the `SubEtha.NotifierSet` the writer signals after each change |
| `Interval` | the seconds between reads, above 0, as a number or a [TimeSpan] |

The renderer reads the values on a thread of its own, at each signal and at least every interval, and a row changes its item only when its values change. A row past the items, or for a name the canvas does not have, changes nothing, and items past the rows are left as they are. A slab or vec is opened read-only with the `Alignment` and `Tag` its object gives or, when it gives neither, with alignment 1 and tag 0, which `New-SubEthaSlab` and `New-SubEthaVec` give when not told others; only the slots whose version moved are read again. A file's values are copied between two readings of its counter and used only when both read the same even number, so no half-written array is drawn; a counter of 0 has had no write. A file missing, or shorter than its values and counter, is refused here. A source the renderer cannot open, a file that shrinks, a counter that stays odd for a second or more, and a row an item cannot take are each written as a warning by the window's command, the values last read staying. `-Ring` and `-SharedMemory` are taken when the window opens, and `Update-GuiWidget` cannot change them.

### Tests

[Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) raises each of the canvas's events as a person would, once the canvas has been drawn. Its `-Value` is a hashtable of `Item`, the name of an item to put the pointer on, at the middle of where it was drawn, or of `U` and `V`, with any of `Button`, `Shift`, `Ctrl`, `Alt` and `Notches`. The event names the topmost named item at that point as the canvas was last drawn, and an item's event where no named item is raises nothing. So a test that changes the items or the view first takes a snapshot of the canvas, which waits for a frame that shows the change. A test event leaves the view as it is, and a key the view's controls take is refused.

## Parameters

### -Item

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: no items |
| Refused with | `GuiBadProperty` for an item the canvas cannot take |

The items, each a hashtable of its `Kind`, its `Name` and its keys, a Group's items under `Children`.

### -Width

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the room its place leaves across |
| Refused with | `GuiBadSize` when not above 0 |

The canvas's width in points.

### -Height

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: a share of the height its column leaves |
| Refused with | `GuiBadSize` when not above 0 |

The canvas's height in points.

### -Ring

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiBadProperty` for a ring the canvas cannot read |

A SubEtha ring another process writes, each item a JSON object of the `Item` it changes and its keys, or an array of them.

### -SharedMemory

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiBadProperty` for a key missing or one its value cannot be, naming it |

Shared memory another process writes, whose rows set the numbers of the items `Item` names: a hashtable of `Source`, or `Path`, `Offset`, `Length`, `Shape` and `Sequence`, with `Type`, `Item`, `Property`, and `Notifier`, `Interval` or both.

### -OnClick

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person clicks a named item, with the event as its argument.

### -OnDoubleClick

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person clicks a named item twice in quick succession, with the event as its argument.

### -OnPointerEnter

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the pointer comes over a named item, with the event as its argument.

### -OnPointerLeave

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the pointer leaves the named item it was over, with the event as its argument.

### -OnDragStart

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person starts dragging a named item, with the event as its argument; its `Value` gives where the drag began.

### -OnDragMove

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the pointer moves while a drag goes on, once a frame at most, with the event as its argument; its `Value` names the item dragged.

### -OnDragEnd

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a drag's button is let go, with the event as its argument; its `Value` names the item dragged.

### -OnPointerDown

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a pointer button is pressed over the canvas, with the event as its argument.

### -OnPointerUp

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a button pressed over the canvas is let go, wherever the pointer is, with the event as its argument.

### -OnPointerMove

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the pointer moves over the canvas, or anywhere while a button pressed over it stays down, once a frame at most, with the event as its argument.

### -OnWheel

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the wheel turns over the canvas, with the event as its argument; its `Value`'s `Notches` are positive when the wheel turns away from the person.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key is pressed while the canvas has the focus, for each key its view's controls do not take, with the event as its argument; its `Value` is the key written as a shortcut.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the canvas, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name by which the handlers' events, the update commands and the view commands find the canvas.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Canvas`, its `Name` the canvas's, and its `Properties` giving `Item`, and `Width`, `Height`, `Ring` and `SharedMemory` when they are given.

## Examples

The widget the command writes:

```powershell
New-GuiCanvas -Name Board -Width 200 -Height 100 @{ Kind = 'Rect'; X = 10; Y = 10; Width = 80; Height = 40 } | Format-List Kind, Name, Properties
```

```text
Kind       : Canvas
Name       : Board
Properties : {[Item, System.Object[]], [Width, 200], [Height, 100]}
```

<!-- powershell -->

```text
Kind       : Canvas
Name       : Board
Properties : {Item, Width, Height}
```

A picture of shapes, a gradient, text in two lines and an Image, and the items as the window has them:

```powershell
$flag = [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEklEQVR4nGP4z8CAFWEXJUsCAFpeH+EeQoQoAAAAAElFTkSuQmCC')
$w = Show-GuiWindow -NoWait -Title 'Hill' {
    Canvas -Name Board -Width 320 -Height 160 @(
        @{ Kind = 'Rect'; Name = 'Sky'; X = 0; Y = 0; Width = 320; Height = 160; Fill = @{ Linear = 'LightSkyBlue', 'White'; From = 0, 0; To = 0, 1 } }
        @{ Kind = 'Ellipse'; Name = 'Sun'; X = 250; Y = 15; Width = 50; Height = 50; Fill = 'Gold'; Stroke = 'Orange'; StrokeWidth = 3 }
        @{ Kind = 'Polygon'; Name = 'Hill'; Points = 0, 160, 140, 70, 280, 160; Fill = 'ForestGreen' }
        @{ Kind = 'Path'; Name = 'Road'; Data = 'M 110 160 C 130 130 150 110 140 70'; Stroke = 'SaddleBrown'; StrokeWidth = 6; Cap = 'Round' }
        @{ Kind = 'Image'; Name = 'Flag'; X = 140; Y = 50; Width = 24; Height = 12; Bytes = $flag }
        @{ Kind = 'Text'; Name = 'Title'; X = 12; Y = 10; Text = "A hill`nand the sun"; Size = 18 }
    )
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.items | ForEach-Object { '{0,-8} {1}' -f $_.kind, $_.name }
Close-GuiWindow -Window $w.Id
```

```text
Rect     Sky
Ellipse  Sun
Polygon  Hill
Path     Road
Image    Flag
Text     Title
```

An item changed, one added into a Group and taken away, each read back from the window:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Board' {
    Canvas -Name Board -Width 200 -Height 100 @(
        @{ Kind = 'Rect'; Name = 'Box'; X = 10; Y = 10; Width = 40; Height = 40; Fill = 'Tomato' }
        @{ Kind = 'Group'; Name = 'Layer'; X = 100; Children = @() }
    )
}
Update-GuiWidget -Window $w.Id -Name Board -Item Box -Property @{ X = 60; Fill = 'Gold' }
Add-GuiItem -Window $w.Id -Name Board -Item @{ Kind = 'Ellipse'; Name = 'Dot'; X = 0; Y = 0; Width = 20; Height = 20 } -Parent Layer
$items = (Get-GuiSnapshot -Window $w.Id).Tree.Properties.items
'{0} at {1}, {2}' -f $items[0].name, $items[0].x, $items[0].fill
'{0} contains {1}' -f $items[1].name, $items[1].children[0].name
Remove-GuiItem -Window $w.Id -Name Board -Item Dot
@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.items[1].children).Count
Close-GuiWindow -Window $w.Id
```

```text
Box at 60, Gold
Layer contains Dot
0
```

A box that slides to the right half a second after it is drawn and stays there, read from the canvas's pixels a second later:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Slide' {
    Canvas -Name Board -Width 200 -Height 50 @(
        @{ Kind = 'Rect'; X = 0; Y = 0; Width = 200; Height = 50; Fill = 'White' }
        @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 50; Height = 50; Fill = 'Red'; Animate = @{ Property = 'X'; To = 150; Duration = 0.5; Easing = 'EaseOutCubic' } }
    )
}
Start-Sleep -Seconds 1
$shot = Get-GuiSnapshot -Window $w.Id -Name Board -Pixels
$isRed = { param($x) $at = (25 * $shot.Width + $x) * 4; $shot.Pixels[$at] -gt 200 -and $shot.Pixels[$at + 1] -lt 60 }
'red at the left: {0}; red at the right: {1}' -f (& $isRed 10), (& $isRed ($shot.Width - 10))
Close-GuiWindow -Window $w.Id
```

```text
red at the left: False; red at the right: True
```

A click on an item and a press where there is no item, raised as a person's would be and read back with Wait-GuiEvent:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Pick' {
    Canvas -Name Board -Width 200 -Height 100 -OnClick {} -OnPointerDown {} @(
        @{ Kind = 'Rect'; Name = 'Box'; X = 20; Y = 20; Width = 60; Height = 60; Fill = 'Navy' }
    )
}
$null = Get-GuiSnapshot -Window $w.Id -Name Board
Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Click -Value @{ Item = 'Box' }
$click = Wait-GuiEvent -Window $w.Id -Name Board -Timeout 20
'{0} on {1} at {2}, {3}' -f $click.Kind, $click.Value.Item, $click.Value.SceneX, $click.Value.SceneY
Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.75; V = 0.5 }
$down = Wait-GuiEvent -Window $w.Id -Name Board -Timeout 20
$down.Value
Close-GuiWindow -Window $w.Id
```

```text
Click on Box at 50, 50

Name                           Value
----                           -----
Alt                            False
Button                         Left
Ctrl                           False
Item
PixelsPerPoint                 1
SceneX                         150
SceneY                         50
Shift                          False
U                              0.75
V                              0.5
X                              150
Y                              50
```

An item a canvas cannot take is refused, naming it and the key:

```powershell
try { New-GuiCanvas @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 10 } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { New-GuiCanvas @{ Kind = 'Line'; X1 = 0; Y1 = 0; X2 = 10; Y2 = 10; Radius = 4 } } catch { $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiCanvasCommand
Item: Rect Box: a Rect needs Height
Item: item 1, a Line: a Line takes no Radius; it takes X1, Y1, X2, Y2, Fill, Stroke, StrokeWidth, Dash, Cap, Join, Opacity, Rotate, Scale, Skew, Origin, Visible, Order, Animate
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | an item, `-Ring` or `-SharedMemory` the canvas cannot take; the message names the item and the key, or the ring's kind or the key of `-SharedMemory` |
| `GuiBadSize` | `-Width` or `-Height` is not above 0 |

Each is described in the [Error Reference](Error-Reference.md).
