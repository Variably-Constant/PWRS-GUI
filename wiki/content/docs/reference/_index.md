---
title: Reference
weight: 4
sidebar:
  open: true
---

Look things up. Each command's page is written from its source under `crates/pwrs-gui/src/`, lists every parameter as PowerShell's binder reports it and every property of its output types, and shows output from a real run in both hosts.

## Commands

{{< cards >}}
  {{< card link="read-guichoice/" title="Read-GuiChoice" subtitle="A list of choices with OK and Cancel: what each choice is shown by, defaults, several at once, and the objects written back." >}}
  {{< card link="out-guigridview/" title="Out-GuiGridView" subtitle="A table of piped objects that sorts, filters and selects, with Out-GridView's parameters and parameter sets." >}}
  {{< card link="read-guiinput/" title="Read-GuiInput" subtitle="A form of text, password, number, date, check and choice fields, written back as one object." >}}
  {{< card link="read-guicredential/" title="Read-GuiCredential" subtitle="A user name and a password, written back as a PSCredential." >}}
  {{< card link="show-guimessage/" title="Show-GuiMessage" subtitle="The system's own message box with its icon and buttons, and the button pressed." >}}
  {{< card link="read-guifile/" title="Read-GuiFile" subtitle="The system's own dialog for files to open, with filters, and the paths chosen." >}}
  {{< card link="read-guisavepath/" title="Read-GuiSavePath" subtitle="The system's own dialog for a file to save, and the path chosen." >}}
  {{< card link="read-guifolder/" title="Read-GuiFolder" subtitle="The system's own dialog for a folder, and the path chosen." >}}
  {{< card link="show-guiprogress/" title="Show-GuiProgress" subtitle="A window of an activity's progress, fed by the script or a SubEtha ring, and its State." >}}
  {{< card link="set-guiprogress/" title="Set-GuiProgress" subtitle="A progress window's activity, status and bar changed, or the window closed." >}}
  {{< card link="get-guidialog/" title="Get-GuiDialog" subtitle="The system's dialogs showing in the session, in any runspace, for a test to close." >}}
  {{< card link="show-guiwindow/" title="Show-GuiWindow" subtitle="A window of widgets a block describes, with its handlers and result, or a page of your own from HTML or a folder, with its window.pwrs bridge." >}}
  {{< card link="wait-guievent/" title="Wait-GuiEvent" subtitle="The next event of a window opened with -NoWait, waited for until it comes or a timeout passes." >}}
  {{< card link="update-guiwidget/" title="Update-GuiWidget" subtitle="A widget's properties changed while its window is open, the widget found by its name, or one item of a canvas." >}}
  {{< card link="add-guiitem/" title="Add-GuiItem" subtitle="Items added to an open window's list, log, chart, table, tree or canvas." >}}
  {{< card link="remove-guiitem/" title="Remove-GuiItem" subtitle="Items taken from an open window's list, table, tree or canvas, by their text, their names or the objects they were made from." >}}
  {{< card link="set-guifocus/" title="Set-GuiFocus" subtitle="The keyboard focus given to a widget of an open window, found by its name, as a click or Tab gives it." >}}
  {{< card link="close-guiwindow/" title="Close-GuiWindow" subtitle="A window closed with a result, from one of its handlers or from anywhere else." >}}
  {{< card link="send-guimessage/" title="Send-GuiMessage" subtitle="A value for an open page's window.pwrs.on handlers, and what the renderer refuses." >}}
  {{< card link="new-guichannel/" title="New-GuiChannel" subtitle="A page's channel to its backend, two SubEtha rings of messages or a plain stream, with its caps and the process it ends with." >}}
  {{< card link="get-guiwindow/" title="Get-GuiWindow" subtitle="The windows the session has open, in any runspace, and the names of their widgets." >}}
  {{< card link="invoke-guitestevent/" title="Invoke-GuiTestEvent" subtitle="Any event a widget raises, a window's shortcut or a drop, raised as a person would, and the value it took." >}}
  {{< card link="get-guisnapshot/" title="Get-GuiSnapshot" subtitle="A window's tree, the state of its inputs, and its pixels or one widget's." >}}
  {{< card link="set-guioption/" title="Set-GuiOption" subtitle="Where windows are drawn, which renderer and GPU draw them, and where webviews store their data, for the next renderer." >}}
  {{< card link="get-guioption/" title="Get-GuiOption" subtitle="The options the next renderer starts with, what the running one uses, and what HTML pages draw GpuSurfaces with." >}}
  {{< card link="measure-guilatency/" title="Measure-GuiLatency" subtitle="A window with one button that times each click to the module, a handler, the pipeline and an engine-event action." >}}
  {{< card link="get-guinativeentry/" title="Get-GuiNativeEntry" subtitle="The addresses another module takes the session through, and the versions they speak." >}}
{{< /cards >}}

## GPU

The GPU layer's commands. Buffers and pipelines live in the egui renderer whichever renderer draws widget windows, and a GpuSurface draws with them in windows of either renderer.

{{< cards >}}
  {{< card link="get-guigpuadapter/" title="Get-GuiGpuAdapter" subtitle="The GPU adapters each backend finds, and the one the renderer draws with." >}}
  {{< card link="new-guigpubuffer/" title="New-GuiGpuBuffer" subtitle="A GPU buffer in the egui renderer, from an array of any numeric type or of a length." >}}
  {{< card link="read-guigpubuffer/" title="Read-GuiGpuBuffer" subtitle="A buffer, or a run of its elements, copied back as one array." >}}
  {{< card link="write-guigpubuffer/" title="Write-GuiGpuBuffer" subtitle="Elements of a buffer overwritten with new values." >}}
  {{< card link="remove-guigpubuffer/" title="Remove-GuiGpuBuffer" subtitle="A buffer freed now, rather than when its object is collected." >}}
  {{< card link="new-guigputexture/" title="New-GuiGpuTexture" subtitle="A texture in the egui renderer, zeroed or filled from an array of texels of one format." >}}
  {{< card link="import-guigpuimage/" title="Import-GuiGpuImage" subtitle="An image file loaded into a texture in the egui renderer." >}}
  {{< card link="write-guigputexture/" title="Write-GuiGpuTexture" subtitle="Every texel of a texture overwritten from an array." >}}
  {{< card link="read-guigputexture/" title="Read-GuiGpuTexture" subtitle="A texture's texels copied back as one array." >}}
  {{< card link="remove-guigputexture/" title="Remove-GuiGpuTexture" subtitle="A texture freed now, rather than when its object is collected." >}}
  {{< card link="new-guigpusampler/" title="New-GuiGpuSampler" subtitle="How a shader filters and wraps when it samples a texture." >}}
  {{< card link="invoke-guigpucompute/" title="Invoke-GuiGpuCompute" subtitle="A compute shader run once over arrays or GPU buffers, as Invoke-WgpuCompute runs one." >}}
  {{< card link="new-guigpurenderpipeline/" title="New-GuiGpuRenderPipeline" subtitle="A render pipeline for a GpuSurface's draws, as New-WgpuRenderPipeline builds one." >}}
  {{< card link="set-guigpusurface/" title="Set-GuiGpuSurface" subtitle="What a GpuSurface draws each frame, and the compute passes run before its draws." >}}
  {{< card link="save-guigpusurface/" title="Save-GuiGpuSurface" subtitle="A GpuSurface written to a PNG file." >}}
  {{< card link="set-guigpucamera/" title="Set-GuiGpuCamera" subtitle="A GpuSurface's camera: where it looks from, its projection, and the controls that move it." >}}
  {{< card link="get-guigpucamera/" title="Get-GuiGpuCamera" subtitle="A GpuSurface's camera, where the person has moved it." >}}
  {{< card link="remove-guigpucamera/" title="Remove-GuiGpuCamera" subtitle="A GpuSurface's camera taken away." >}}
  {{< card link="reset-guigpudevice/" title="Reset-GuiGpuDevice" subtitle="The egui renderer's GPU device destroyed, as a driver reset loses one, and the renderer on a new one." >}}
{{< /cards >}}

## Canvas

A Canvas's view: which part of its items it shows, how near, and the gestures and keys that pan and zoom it.

{{< cards >}}
  {{< card link="set-guicanvasview/" title="Set-GuiCanvasView" subtitle="A Canvas's view: its center and zoom, a fit to its items, and the controls the person pans and zooms with." >}}
  {{< card link="get-guicanvasview/" title="Get-GuiCanvasView" subtitle="A Canvas's view as it is drawn, where the person has moved it." >}}
  {{< card link="remove-guicanvasview/" title="Remove-GuiCanvasView" subtitle="A Canvas's view taken away." >}}
{{< /cards >}}

## Widgets

The widgets a `Show-GuiWindow -Content` block writes. Each is a `New-Gui<Widget>` command, and inside the block its short name, such as `Button` for `New-GuiButton`, stands for it.

### Layout

{{< cards >}}
  {{< card link="new-guicolumn/" title="Column" subtitle="Widgets stacked top to bottom." >}}
  {{< card link="new-guirow/" title="Row" subtitle="Widgets side by side." >}}
  {{< card link="new-guigrid/" title="Grid" subtitle="Widgets in a grid of columns, filled a row at a time." >}}
  {{< card link="new-guigroup/" title="Group" subtitle="Widgets in a frame, under a title." >}}
  {{< card link="new-guitabs/" title="Tabs" subtitle="Tabs, one shown at a time, and the tab chosen." >}}
  {{< card link="new-guitab/" title="Tab" subtitle="One tab: its title and its widgets." >}}
  {{< card link="new-guisplit/" title="Split" subtitle="Two widgets with a divider between them that a person drags." >}}
  {{< card link="new-guiscroll/" title="Scroll" subtitle="Widgets in an area that scrolls." >}}
  {{< card link="new-guispacer/" title="Spacer" subtitle="Empty space between widgets." >}}
  {{< card link="new-guiseparator/" title="Separator" subtitle="A rule between widgets, or between a menu's items." >}}
{{< /cards >}}

### Display

{{< cards >}}
  {{< card link="new-guitext/" title="Text" subtitle="A line of text in a style, or a link that runs a handler." >}}
  {{< card link="new-guimarkdown/" title="Markdown" subtitle="A Markdown document from text or a file, and the links in it." >}}
  {{< card link="new-guiimage/" title="Image" subtitle="A picture from a file or from bytes." >}}
  {{< card link="new-guiprogressbar/" title="ProgressBar" subtitle="How far work has gone, with text on the bar, set by updates or read from a SubEtha ring." >}}
  {{< card link="new-guispinner/" title="Spinner" subtitle="A spinner for work of unknown length." >}}
  {{< card link="new-guilogview/" title="LogView" subtitle="Lines that stay scrolled to the newest, added by a command or read from a SubEtha ring." >}}
  {{< card link="new-guichart/" title="Chart" subtitle="Series drawn as lines, bars or points." >}}
  {{< card link="new-guicanvas/" title="Canvas" subtitle="Shapes, text and pictures with events, animations and a view that pans and zooms, changed by commands, a SubEtha ring or shared memory." >}}
  {{< card link="new-guigpusurface/" title="GpuSurface" subtitle="A surface your WGSL draws on in either renderer, with pointer events and a camera." >}}
{{< /cards >}}

### Input

{{< cards >}}
  {{< card link="new-guibutton/" title="Button" subtitle="A button that runs a handler, closes the window with a result, or both." >}}
  {{< card link="new-guicheckbox/" title="CheckBox" subtitle="A check box with its text, checked or not." >}}
  {{< card link="new-guitextbox/" title="TextBox" subtitle="A text field of one line or several, or a password field." >}}
  {{< card link="new-guinumberbox/" title="NumberBox" subtitle="A number field limited to a range, which a person drags, steps or types into." >}}
  {{< card link="new-guislider/" title="Slider" subtitle="A slider from a minimum to a maximum, moving in steps." >}}
  {{< card link="new-guicombobox/" title="ComboBox" subtitle="A drop-down of choices, or a text field that offers them." >}}
  {{< card link="new-guiradiogroup/" title="RadioGroup" subtitle="Option buttons, one per choice, of which one is chosen." >}}
  {{< card link="new-guicolorpicker/" title="ColorPicker" subtitle="A color chooser." >}}
  {{< card link="new-guidatepicker/" title="DatePicker" subtitle="A date and time to the minute, limited to a range." >}}
  {{< card link="new-guifilepicker/" title="FilePicker" subtitle="A file or folder, chosen in the system's dialog." >}}
  {{< card link="new-guilistbox/" title="ListBox" subtitle="A list to select one item from, or any number." >}}
{{< /cards >}}

### Data

{{< cards >}}
  {{< card link="new-guitable/" title="Table" subtitle="Objects in rows that sort, filter and select, with rows added and taken away while the window is open." >}}
  {{< card link="new-guitreeview/" title="TreeView" subtitle="Nodes that open to show the nodes inside them, given up front or found when a node opens." >}}
{{< /cards >}}

### Window

{{< cards >}}
  {{< card link="new-guimenubar/" title="MenuBar" subtitle="A bar of menus along the top of the window." >}}
  {{< card link="new-guimenu/" title="Menu" subtitle="A menu's title, and its items, menus and rules." >}}
  {{< card link="new-guimenuitem/" title="MenuItem" subtitle="A menu's item, chosen by a click or by its shortcut." >}}
  {{< card link="new-guicontextmenu/" title="ContextMenu" subtitle="A menu that opens where a person right-clicks a widget." >}}
  {{< card link="new-guistatusbar/" title="StatusBar" subtitle="A bar along the bottom of the window, with widgets side by side." >}}
  {{< card link="new-guibuttonbar/" title="ButtonBar" subtitle="The window's buttons along its bottom, at the right, in the order the system's dialogs put them." >}}
{{< /cards >}}

## Other reference

{{< cards >}}
  {{< card link="native-entry-functions/" title="The Native Entry's Functions" subtitle="The four functions' contract, version 1: types, statuses, waits, the wake at a session's end, and how a taker reaches the renderer." >}}
  {{< card link="environment-variables/" title="Environment Variables" subtitle="Every variable the module, the session and the renderer read." >}}
  {{< card link="error-reference/" title="Error Reference" subtitle="Every error id, its category, whether it ends the command, and when it is raised." >}}
  {{< card link="tested-platforms/" title="Tested Platforms" subtitle="The machines, systems and hosts, the builds, what each run showed, and the measurements." >}}
{{< /cards >}}
