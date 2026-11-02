---
title: Error Reference
weight: 62
---

Every error id the module raises. Source: `gui_error`, `html_session` and `closed_error` in `crates/pwrs-gui/src/session.rs`, with `GuiTimeout`'s words in `crates/pwrs-gui/src/inspect.rs`, which turn the session's `GuiError` (`crates/pwrs-gui-client/src/lib.rs`) and a window's close into error records. The rest come from `crates/pwrs-gui/src/options.rs`, `choice.rs`, `grid.rs`, `latency.rs`, `page.rs`, `dialogs.rs`, `input.rs`, `progress.rs`, `media.rs` and `gpu.rs`, with the GPU layer's own failures in `crates/pwrs-gui-gpu` and the egui renderer's in `crates/pwrs-gui-egui/src/gpu_thread.rs`. What the session's messages say comes from `check_display` and `helper_ended` in `crates/pwrs-gui-client/src/platform.rs`.

The id is the first part of the record's `FullyQualifiedErrorId`; the second part names the command's class, as in `GuiRefused,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand`. A terminating error ends the command, and in a script with `-ErrorAction Stop` or inside `try` it is caught as an exception; a non-terminating one is written to the error stream and the command goes on, so `-ErrorVariable` and `-ErrorAction` apply to it. [How To Handle Errors](How-To-Handle-Errors.md) shows both.

## The session's errors

Any command that starts a renderer or talks to one can raise these.

| Id | Category | Ends the command | Raised when |
|---|---|---|---|
| `GuiNoDisplay` | ResourceUnavailable | yes | No window can open: on Linux and FreeBSD neither `DISPLAY` nor `WAYLAND_DISPLAY` is set; on Windows the process's window station is not visible, as in a service. The message begins `no window can open:` and gives the reason. |
| `GuiStartFailed` | ResourceUnavailable | yes | The renderer could not start: the helper could not be found or launched, its rings or job object could not be set up, it ended before saying it had started, or, drawing in-process, the renderer's thread could not take it; or `-Adapter` names no adapter, or `PWRS_GUI_BACKEND` no backend. In `Auto` mode on Windows a helper that cannot start is not this error: the session draws in-process instead, and the Verbose stream says why. The message begins `the GUI renderer could not start:`. |
| `GuiBuildMismatch` | InvalidOperation | yes | The helper speaks another protocol than the module's library, so the two come from different builds; the message names both. |
| `GuiCanceled` | OperationStopped | yes | Ctrl+C ended a wait for the renderer to start. PowerShell does not show an error once its pipeline stops, so only a caller of the session that ends the wait itself receives this. |
| `GuiRendererGone` | ResourceUnavailable | yes | The renderer went away while a command waited on it or made a request of it: a helper exited or was killed, or the in-process run ended. For a helper the message says how it ended and quotes the last 20 lines of its log, with the log's path. The next command starts a new renderer. |
| `GuiRefused` | InvalidArgument | no | The renderer could not do what the command requested: no window with that number, no widget with that name (the message lists the window's widgets), an event the widget does not raise, a value the event cannot take, such as a number outside a field's range or a row past a list's end, a click, change, Enter or key on a widget that is disabled, or pixels of a window or widget that is minimized, covered or on a tab not shown and so not drawn, or that closed before its capture; for `Set-GuiFocus`, a widget that is disabled, on a tab not shown, or a Canvas that takes no keys; for `Send-GuiMessage`, a window that shows widgets rather than a page of its own, or a value that is not JSON; for `Set-GuiMediaPlayer`, any call on a clip that could not be played, or a rate other than 1 where GStreamer has no `scaletempo` element; for `Invoke-GuiTestEvent -Dialog`, a value the dialog cannot take, a button the message does not show, more than one path for a dialog of one, or no path, after which the dialog stays open. For `Invoke-GuiTestEvent -Input`, a point outside the window's inside, a press of a button the pointer holds or a release of one it does not, a window still taking another test's input, not yet drawn or showing a page of the caller's own, or one that closed before every step was given. |
| `GuiNoNativeDialog` | ResourceUnavailable | yes | The system has no dialog of the kind, and no renderer drew one in its place. The egui renderer draws its own where the system has none, and an HTML session's dialog goes to it, so this is not expected. |
| `GuiTransport` | ConnectionError | yes | A ring in shared memory, or an in-process channel, failed. |
| `GuiWaitNotArmed` | InvalidOperation | yes | The command could not register with PowerShell the wake that lets Ctrl+C end its wait. |
| `GuiNoSession` | ObjectNotFound | no | `Invoke-GuiTestEvent`, `Get-GuiSnapshot` or `Send-GuiMessage` ran before any window of that renderer had opened in the process, so there is no renderer for the command to reach. `Get-GuiWindow` writes nothing instead. |
| `GuiOptions` | InvalidData | yes | A command that starts a renderer could not read the options: `PWRS_GUI_MODE` names no mode or `PWRS_GUI_WEBVIEW` no web data choice, or either cannot be read. |
| `GuiNoWebview` | ResourceUnavailable | yes | A page of your own cannot open because the HTML renderer could not start. The message gives the renderer's reason, and names what to install when the engine is missing: Microsoft's WebView2 Runtime on Windows, and on Linux and FreeBSD the system's WebKitGTK 4.1 or `Install-Module PWRS-GUI.WebKit.<rid>`. A widget window meant for the HTML renderer draws with egui instead, and the Verbose stream says why. |
| `GuiPageFailed` | ResourceUnavailable | yes | A window's page could not be shown, so the window closed: its webview could not be made, or the engine drawing the page ended, the page was loaded again, and it ended again before it could draw. The message says why. When WebView2 refused the webview because the browser process on the shared data folder was started with other options, such as another `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, it names `Set-GuiOption -WebView PerProcess` and `-WebViewDataFolder`, either of which puts the process on a data folder of its own; runtimes from 154.0.4258.62 join such a page to the running browser process instead, without its options, and no error is written. When WebView2 refused it because the shared data folder is in use by a browser process this process cannot join, such as one in another sign-in of the same user, it names the same two. A page whose engine ends once is loaded again with no error, and comes back as it was. |

## The commands' errors

| Id | Category | Ends the command | Commands | Raised when |
|---|---|---|---|---|
| `GuiModeUnknown` | InvalidData | no | `Get-GuiOption` | `PWRS_GUI_MODE` names no mode; the message gives the value and the three modes. |
| `GuiModeUnreadable` | InvalidData | no | `Get-GuiOption` | `PWRS_GUI_MODE` cannot be read, as when it is not valid Unicode. |
| `GuiWebViewUnknown` | InvalidData | no | `Get-GuiOption` | `PWRS_GUI_WEBVIEW` names neither `Shared` nor `PerProcess`; the message gives the value. |
| `GuiWebViewUnreadable` | InvalidData | no | `Get-GuiOption` | `PWRS_GUI_WEBVIEW` cannot be read. |
| `GuiNoFolder` | InvalidArgument | yes | `Set-GuiOption` | `-WebViewDataFolder` resolves to no path. |
| `GuiNoPage` | ObjectNotFound, InvalidArgument | yes | `Show-GuiWindow` | `-Path` resolves to nothing, names a folder with no `index.html`, or names a file in no folder. |
| `GuiJson` | InvalidResult | yes | `Show-GuiWindow`, `Send-GuiMessage` | `ConvertTo-Json` wrote nothing for what `-OnMessage` returned or what `-Message` gives. This is not expected. |
| `GuiNoChoices` | InvalidArgument | yes | `Read-GuiChoice` | No choice arrived, from `-Choice` or the pipeline. |
| `GuiDefaultNotAChoice` | InvalidArgument | yes | `Read-GuiChoice` | A `-Default` matches no choice's text; the message names it. |
| `GuiTooManyDefaults` | InvalidArgument | yes | `Read-GuiChoice` | More than one `-Default` matched without `-Multiple`. |
| `GuiPropertyInvalid` | InvalidArgument | yes | `Read-GuiChoice` | `-Property` is neither a string nor a script block; the message names its type. |
| `GuiLabelCount` | InvalidResult | yes | `Read-GuiChoice` | Showing the choices gave a different number of labels than there are choices. This is not expected. |
| `GuiBadSelection` | InvalidData | yes | `Read-GuiChoice`, `Out-GuiGridView` | The window reported a selection that is not a row number of its own. This is not expected. |
| `GuiNoColumns` | InvalidData | yes | `Out-GuiGridView` | The first object has no properties to show. |
| `GuiRowCount` | InvalidResult | yes | `Out-GuiGridView`, `Show-GuiWindow` | Evaluating the columns of a grid view or a Table gave a different number of rows than objects sent. This is not expected. |
| `GuiHandlerSilent` | InvalidResult | yes | `Measure-GuiLatency` | The handler script block wrote no clock reading. This is not expected. |
| `GuiThread` | ResourceUnavailable | yes | `Measure-GuiLatency -NoWait`, `Show-GuiWindow -NoWait` | The thread that reads the window's events could not start. |
| `GuiNoDialog` | ObjectNotFound | yes | `Invoke-GuiTestEvent -Dialog` | No dialog showing has the number given; `Get-GuiDialog` lists the ones showing. |
| `GuiBadAnswer` | InvalidArgument, InvalidResult | yes | `Invoke-GuiTestEvent -Dialog`, `Show-GuiMessage`, `Read-GuiFile`, `Read-GuiSavePath`, `Read-GuiFolder` | As InvalidArgument, a `-Value` for a message that names no button: `Ok`, `Cancel`, `Yes` or `No`. As InvalidResult, a dialog that closed with the other kind's result, a message with paths or a file dialog with a button, which is not expected. |
| `GuiBadFilter` | InvalidArgument | yes | `Read-GuiFile`, `Read-GuiSavePath` | A `-Filter` that names no kind of file, as a string without `\|`, or a plain hashtable, whose keys come in no fixed order; the message says why. |
| `GuiBadField` | InvalidArgument | yes | `Read-GuiInput` | A field the form cannot take: none at all, a key other than `Name`, `Label`, `Type`, `Default` and `Choice`, no `Name`, a `Type` none of the six, a `Default` its type cannot take or a `Password` field's, `Choice` on another type or none on a `Choice` field, a field named `Ok` or `Cancel`, the buttons' names, or two fields of one name. The message names the field by its place. |
| `GuiBadInput` | InvalidData | yes | `Read-GuiInput`, `Read-GuiCredential` | The window reported a value its field cannot take. This is not expected. |
| `GuiBadPercent` | InvalidArgument | yes | `Show-GuiProgress`, `Set-GuiProgress` | `-PercentComplete` is outside 0 to 100. |

## The errors of windows described in PowerShell

`Show-GuiWindow -Content`, the `New-Gui<Widget>` commands and the commands that change an open window raise these. Each widget's reference page lists the ones it can raise.

| Id | Category | Ends the command | Commands | Raised when |
|---|---|---|---|---|
| `GuiNoWidgets` | InvalidArgument | yes | `Show-GuiWindow` | The `-Content` block wrote no widget. |
| `GuiNotAWidget` | InvalidArgument | yes | `Show-GuiWindow`, the `New-Gui<Widget>` commands that take widgets | A block wrote something other than a widget; the message names its type. `$null` is passed over. |
| `GuiNameTwice` | InvalidArgument | yes | `Show-GuiWindow` | Two widgets of the window have one name, so `State` and the update commands could not tell them apart. |
| `GuiUnknownWidget` | InvalidArgument | yes | `Show-GuiWindow` | A `PwrsGui.Widget` names a `Kind` that is none of the widgets; the message lists them. |
| `GuiUnknownEvent` | InvalidArgument | yes | `Show-GuiWindow` | A `PwrsGui.Widget`'s handlers name an event its kind does not raise. |
| `GuiUnknownProperty` | InvalidArgument | yes | `Update-GuiWidget`, `Show-GuiWindow` | A property the widget does not take; the message lists those it takes. |
| `GuiBadProperty` | InvalidArgument | yes | the `New-Gui<Widget>` commands, `Show-GuiWindow`, `Update-GuiWidget`, `Invoke-GuiMedia` | A value the widget refuses: a `-Selected` that names no choice, item or tab, a color not written `#rrggbb`, two properties that exclude each other, a `-Ring` no widget reads, a canvas's item, naming it and the key, or a key of its `-SharedMemory`, and the others each widget's page lists. From `Update-GuiWidget`, also any value a property cannot be read as, where its command raises an id of its own: a shortcut not written as one, a picture in no format an Image takes, a split's position outside 0 to 1, a number that is not finite. The message names the property and why. |
| `GuiOpeningOnly` | InvalidArgument | yes | `Update-GuiWidget` | A property taken when the window opens: a TextBox's `Password`, a ComboBox's `Editable`, a FilePicker's `Save`, `Folder` and `Multiple`, a widget's `Ring`, a Canvas's `SharedMemory`, a Table's `Item` and `Property`, a TreeView's `Node`, `Text` and `HasChildren`. |
| `GuiBadNumber` | InvalidArgument | yes | `New-GuiNumberBox`, `New-GuiSlider`, `New-GuiProgressBar`, `New-GuiSplit`, `New-GuiMediaPlayer`, `Invoke-GuiMedia`, `Update-GuiWidget` | A number the widget does not take: one that is not finite, a minimum above a maximum, a step not above 0, a value outside its range; for `New-GuiSplit`, a position outside 0 to 1; for a MediaPlayer, a volume outside 0 to 1 or a rate outside 0.25 to 4. |
| `GuiBadDate` | InvalidArgument | yes | `New-GuiDatePicker`, `Update-GuiWidget` | A date outside the chooser's range, or a range that ends before it starts. |
| `GuiBadSize` | InvalidArgument | yes | `New-GuiScroll`, `New-GuiLogView`, `New-GuiSpacer`, `New-GuiImage`, `New-GuiChart`, `New-GuiGpuSurface`, `New-GuiCanvas`, `New-GuiMediaPlayer`, `Update-GuiWidget` | A size below 0 points for a Scroll's height, a LogView's width or height, or a Spacer's size, or not above 0 for an Image's sides, a Chart's height, or a GpuSurface's, a Canvas's or a MediaPlayer's sides. |
| `GuiBadPath` | InvalidArgument | yes | `New-GuiFilePicker` | A path that is no path, or a relative one where the current location is no folder of files. |
| `GuiNoFile` | ObjectNotFound | yes | `New-GuiMarkdown`, `New-GuiImage`, `New-GuiMediaPlayer`, `Invoke-GuiMedia` | `-Path` names no file that can be read, or several. |
| `GuiBadImage` | InvalidData | yes | `New-GuiImage` | The picture is none of the formats an Image takes; the message names what it found. |
| `GuiNoDocument`, `GuiNoPicture` | InvalidArgument | yes | `New-GuiMarkdown`, `New-GuiImage` | Neither `-Text` nor `-Path`, or neither `-Bytes` nor `-Path`, was given. The parameter sets make this not expected. |
| `GuiTooManySelected` | InvalidArgument | yes | `New-GuiListBox` | `-Selected` names several items without `-Multiple`. |
| `GuiNotATab` | InvalidArgument | yes | `New-GuiTabs`, `Show-GuiWindow` | A Tabs has a widget in it other than a Tab. |
| `GuiTabOutsideTabs` | InvalidArgument | yes | `Show-GuiWindow` | A Tab is anywhere but in a Tabs. |
| `GuiNotTwoWidgets` | InvalidArgument | yes | `New-GuiSplit`, `Show-GuiWindow` | A Split has other than two widgets in it. |
| `GuiNotAtTop` | InvalidArgument | yes | `Show-GuiWindow` | A MenuBar, a StatusBar or a ButtonBar is inside another widget rather than among the window's own. |
| `GuiNotAButton` | InvalidArgument | yes | `Show-GuiWindow` | A ButtonBar has a widget in it other than a Button; the message names its kind. |
| `GuiButtonRole` | InvalidArgument | yes | `New-GuiButton`, `Show-GuiWindow` | A button is both the `-Default` and the `-Cancel` button; or a window has two ButtonBars, two `-Default` buttons or two `-Cancel` buttons, and the message names the first two found. |
| `GuiNotInMenu` | InvalidArgument | yes | `Show-GuiWindow` | A Menu is outside a MenuBar, a ContextMenu or another Menu, a MenuItem is outside a Menu or a ContextMenu, a MenuBar has a widget in it other than a Menu, or a menu has one other than an item, a menu or a rule. |
| `GuiNotInTree` | InvalidArgument | yes | `Show-GuiWindow` | A tree's item is outside a TreeView, or a TreeView has another widget in it. |
| `GuiBadTreeNode` | InvalidArgument, InvalidResult | yes | `Show-GuiWindow`, `Add-GuiItem` | A tree's node could not be read, such as a hashtable without `Text`; the message says why. As InvalidResult, the script that describes the nodes returned something other than what was requested, which is not expected. |
| `GuiBadShortcut` | InvalidArgument | yes | `New-GuiMenuItem`, `Show-GuiWindow`, `Invoke-GuiTestEvent` | A shortcut not written as one, as `Ctrl+Shift+S`; for `-OnShortcut`, also a shortcut given twice or a value that is not a script block. |
| `GuiBadStep` | InvalidArgument | yes | `Invoke-GuiTestEvent -Input` | A step of a person's input that is not one: no hashtable, none or more than one of `Press`, `Release`, `Move`, `Key` and `Wait`, a key no step takes, or a key given twice. Also a button other than `Left`, `Right` or `Middle`, a point without a finite `X` and `Y`, a key not written as a shortcut or `Menu`, or a wait below 0 seconds. The message names the step by its place; `-Input` with no steps is refused too. |
| `GuiTimeout` | OperationTimeout | no | `Invoke-GuiTestEvent -Input` | A step of the input was not taken and seen within its time: 10 seconds, or `-Timeout`'s, from when it began, a wait's own seconds besides. The message names the step and its time, as `step 1 of 2, a press of the Right button at 40, 12, was not taken and seen within 10 seconds`, and the window drops the steps left. |
| `GuiBadWatch` | InvalidArgument | yes | `Show-GuiWindow` | `-WatchPath` names no file or a file twice, `-WatchInterval` is not seconds above 0 or a `[TimeSpan]`, or `-WatchInterval` or `-OnFileChange` is given without `-WatchPath`. |
| `GuiNoWatch` | ResourceUnavailable | yes | `Show-GuiWindow` | A file `-WatchPath` names cannot be watched: its folder does not exist, or the system's change notifications cannot start; a window already opened for it is closed. |
| `GuiNoWidget` | ObjectNotFound, InvalidArgument | yes | `Update-GuiWidget`, `Set-GuiFocus`, `Add-GuiItem`, `Remove-GuiItem`, `Set-GuiMediaPlayer`, `Get-GuiMediaPlayer`, `Show-GuiWindow` | The window has no widget of that name; the message lists the names it has. For `Show-GuiWindow`, a ContextMenu's `-For` names no widget of the window. |
| `GuiNoWindow` | ObjectNotFound, InvalidArgument | yes, but no for `Close-GuiWindow` given a number | `Update-GuiWidget`, `Set-GuiFocus`, `Add-GuiItem`, `Remove-GuiItem`, `Close-GuiWindow`, `Wait-GuiEvent`, `Set-GuiProgress`, `Set-GuiMediaPlayer`, `Get-GuiMediaPlayer` | No window described in PowerShell is open with that number, or `-Window` was left out outside a handler, where it would act on the handler's window; for `Set-GuiProgress`, a `-Window` that is neither a number nor an object with an `Id`. For `Close-GuiWindow`, no window of any kind is open with the number given, whether or not a renderer is running, so a list piped in goes on past it. |
| `GuiNoItems` | InvalidArgument | yes | `Add-GuiItem`, `Remove-GuiItem` | The widget has no items to add or take away: it is none of a ListBox, ComboBox, RadioGroup, LogView, Chart, Table, TreeView or Canvas; or, for `Remove-GuiItem`, a LogView's lines or a Chart's points, which go only all at once through `Update-GuiWidget`. |
| `GuiNoRow`, `GuiTwoRows` | ObjectNotFound, InvalidArgument | yes | `Remove-GuiItem` | No row of the table was made from the object, or several were, as from the same string twice; the message names the rows, so one can be named by the object it was made from. |
| `GuiNoItem` | ObjectNotFound | yes | `Remove-GuiItem`, `Update-GuiWidget -Item`, `Add-GuiItem -Parent` | The list has no item with that text, and the message lists its items; or the canvas has no item of the name given. |
| `GuiBadItem` | InvalidArgument | yes | `Update-GuiWidget -Item`, `Add-GuiItem` | A canvas's item or change the canvas cannot take: a key its kind does not take, a value a key cannot take, a name the canvas already has, a change that sets `Kind`, `Name` or `Children`, or a `-Parent` naming an item that is no Group. The message names the canvas, the item and the key, and nothing changes. |
| `GuiNotACanvas` | InvalidArgument | yes | `Update-GuiWidget -Item`, `Set-GuiCanvasView`, `Get-GuiCanvasView`, `Remove-GuiCanvasView` | The widget of that name is not a Canvas; the message names its kind. |
| `GuiNotAMediaPlayer` | InvalidArgument | yes | `Set-GuiMediaPlayer`, `Get-GuiMediaPlayer` | The widget of that name is not a MediaPlayer; the message names its kind. |
| `GuiBadMedia` | InvalidArgument | yes | `Set-GuiMediaPlayer` | `-Play` with `-Pause`, a `-Position` that is neither seconds nor a `[TimeSpan]` or is below 0, a `-Rate` outside 0.25 to 4, or a `-Volume` outside 0 to 1; no call is made. |
| `GuiMediaUnplayable` | InvalidData | yes | `Invoke-GuiMedia` | The player cannot play the clip's format, or the system has no player. The message is the player's `Error` event's: the clip, why, the format its headers name, and the remedy where there is one, which is the FFmpeg add-on's `Install-Module` line in an egui window, `-Renderer Egui` in an HTML window, or what to install. |
| `GuiMediaFailed` | ResourceUnavailable | yes | `Invoke-GuiMedia` | The clip cannot be played for another cause: its file or address cannot be read, reading it was stopped, or its picture or sound cannot be decoded. The message is the player's `Error` event's, naming the clip and why. |
| `GuiNotFocusable` | InvalidArgument | yes | `Set-GuiFocus` | The widget's kind takes no keyboard focus: a Text without `-Style Link`, a GpuSurface without `-OnKey`, or a widget that is not an input, a button, a list, a table or a tree; the message names its kind. |
| `GuiNoNode`, `GuiTwoNodes` | ObjectNotFound, InvalidArgument | yes | `Add-GuiItem -Parent`, `Remove-GuiItem` | No node of the tree was made from the object, or several were; the message names them, so one can be named by the object it was made from. |
| `GuiNoParent` | InvalidArgument | yes | `Add-GuiItem` | `-Parent` was given for a widget other than a TreeView or a Canvas. |
| `GuiBadPoint` | InvalidArgument | yes | `Add-GuiItem` | A chart's point is not a hashtable of `Series`, `Y` and `X` the chart takes, such as a date on a chart of numbers. |
| `GuiNotQueued` | InvalidOperation | yes | `Wait-GuiEvent` | The window was not opened with `-NoWait`, so the command that opened it reads its events. |
| `GuiBadTimeout` | InvalidArgument | yes | `Wait-GuiEvent`, `Invoke-GuiTestEvent -Input` | `-Timeout` is below 0, or not a number, or one no clock counts. |
| `GuiNotAWidgetEvent` | InvalidArgument | yes | `Invoke-GuiTestEvent` | `-Event Close` or `-Event FileChange`, the window's own events, which no widget raises; `Close-GuiWindow` closes a window, and a write to a file `Show-GuiWindow -WatchPath` names raises `FileChange`. |
| `GuiNoNodeNumber` | LimitsExceeded | yes | `Show-GuiWindow`, `Add-GuiItem` | The window has used all 4,294,967,295 node numbers. |
| `GuiBadState` | InvalidData | yes | `Update-GuiWidget`, `Add-GuiItem`, `Remove-GuiItem` | The module's own record of a widget could not be read. This is not expected. |
| `GuiNoShape` | InvalidResult | no | `New-GuiFilePicker`, `Update-GuiWidget` | Reading what kind of value a property was given returned nothing. This is not expected. |

## The GPU commands' errors

The GPU commands and a Canvas's view commands raise these, and the GPU commands also pass on the Wgpu module's own ids for what its code refuses. Among them are `WgpuOutOfRange` and `WgpuNotNumeric` for a value that does not convert to a buffer's or texture's type, `WgpuShaderInvalid` for a shader that does not validate, with its line and column, and `WgpuBlend` and `WgpuBlendConstant` for a blend a pipeline cannot draw. `WgpuDepthTest`, `WgpuStencil` and `WgpuDepthBias` refuse a depth test, a stencil test or a depth bias the depth format or the topology cannot take, and `WgpuStencilReference` a stencil reference no stencil test reads. An HTML window's page checks a surface as the egui renderer does, and refuses what it refuses under the same ids. A command that acts on a window raises the window errors above too, such as `GuiNoWindow` and `GuiNoWidget`. Only the errors the table marks end the command; the others are non-terminating.

| Id | Category | Ends the command | Commands | Raised when |
|---|---|---|---|---|
| `GuiGpuNoLayer` | ResourceUnavailable | no | every GPU command | The renderer the command reached has no GPU layer: the HTML renderer, or an egui renderer whose GPU thread could not start, and the message says why. |
| `GuiGpuEnded` | ResourceUnavailable | no | every GPU command | The renderer's GPU thread, or the renderer, ended before it replied. |
| `GuiGpuOtherSession` | InvalidArgument | no | the commands that take a buffer, texture, sampler or pipeline | The object lives in a renderer that has since ended, so the renderer running now does not have it. |
| `GuiGpuRemoved` | ObjectNotFound | no | `Read-GuiGpuBuffer`, `Write-GuiGpuBuffer`, `Read-GuiGpuTexture`, `Write-GuiGpuTexture`, and the commands that bind them | `Remove-GuiGpuBuffer` or `Remove-GuiGpuTexture` freed the object. |
| `GuiGpuNoBuffer`, `GuiGpuNoTexture`, `GuiGpuNoSampler`, `GuiGpuNoPipeline` | InvalidArgument, ObjectNotFound | no | the commands that take one | As InvalidArgument, the command was given none. As ObjectNotFound, the renderer has no such object, which is not expected. |
| `GuiGpuNoData` | InvalidArgument | no | `Write-GuiGpuBuffer`, `Write-GuiGpuTexture` | The buffer or texture, or the data, is missing. |
| `GuiGpuEmpty` | InvalidArgument | no | `New-GuiGpuBuffer`, `Invoke-GuiGpuCompute`, `Set-GuiGpuSurface` | A buffer of no elements, or an empty array for a binding or a vertex input. |
| `GuiGpuBufferTooLarge` | InvalidArgument | no | `New-GuiGpuBuffer` | `-Length` elements of the type are more bytes than a buffer can address. |
| `GuiGpuRange` | InvalidArgument, InvalidOperation | no | `Read-GuiGpuBuffer`, `Write-GuiGpuBuffer` | Elements before the offset or past the buffer's end, as InvalidArgument; as InvalidOperation, a range the renderer found outside the buffer, which is not expected. |
| `GuiGpuTextureSize` | InvalidArgument | no | `New-GuiGpuTexture`, `Write-GuiGpuTexture` | A side of 0 texels, or values other than every texel's; the message gives the bytes and values the texture takes. |
| `GuiGpuImage` | ObjectNotFound, ReadError, InvalidArgument, InvalidData | no | `Import-GuiGpuImage` | A path that names no file, as ObjectNotFound; a file that cannot be read, as ReadError; neither `-Path` nor `-LiteralPath`, as InvalidArgument; a file the renderer cannot decode as an image, as InvalidData, the message naming the file. |
| `GuiGpuImageFormat` | InvalidArgument | no | `Import-GuiGpuImage` | A `-Format` other than `Rgba8Unorm` and `Rgba8UnormSrgb`, or `-Srgb` with one other than `Rgba8UnormSrgb`. |
| `GuiGpuNoShader` | InvalidArgument | no | `Invoke-GuiGpuCompute`, `New-GuiGpuRenderPipeline`, `Set-GuiGpuSurface` | No shader was given, or a draw or a pass was described without one, or a draw's shader is neither WGSL text nor a `PwrsGui.GpuRenderPipeline`. |
| `GuiGpuShaderLanguage` | InvalidArgument | no | `New-GuiGpuRenderPipeline`, `Set-GuiGpuSurface` | A render pipeline's shader is GLSL, which has one stage and so cannot draw. |
| `GuiGpuEntry` | InvalidArgument, InvalidOperation | no | `Invoke-GuiGpuCompute`, `Set-GuiGpuSurface -Compute` | A GLSL compute shader's entry point named other than `main`. |
| `GuiGpuBinding` | InvalidArgument, InvalidOperation | no | `Invoke-GuiGpuCompute`, `Set-GuiGpuSurface` | A binding named that the shader does not use, a key that is neither a name nor `group:binding`, a binding given twice, a value a binding cannot take, or a binding in the frame uniform's place at `@group(1) @binding(0)`; the message lists the bindings the shader uses. |
| `GuiGpuBindingCount` | InvalidArgument | no | `Invoke-GuiGpuCompute` | Resources given in order, other in number than the shader's bindings. |
| `GuiGpuStruct` | InvalidArgument | no | `Invoke-GuiGpuCompute` | A hashtable for one binding; a struct's members are packed into a buffer to bind them. |
| `GuiGpuDispatchSize` | InvalidArgument | no | `Invoke-GuiGpuCompute`, `Set-GuiGpuSurface -Compute` | More than one of `-Workgroups`, `-Invocations` and `-Indirect`, or other than one to three counts. |
| `GuiGpuVertex` | InvalidArgument, InvalidOperation | no | `Set-GuiGpuSurface` | A vertex input given a hashtable, a texture or a sampler, or a value that is no array of numbers. |
| `GuiGpuVertexInput` | InvalidOperation | no | `Set-GuiGpuSurface` | Vertex inputs given other in number than the shader's, or one given twice. |
| `GuiGpuVertexCount` | InvalidOperation | no | `Set-GuiGpuSurface` | A draw with no `-VertexCount`, no index buffer and no values for a first vertex input, or values for more vertices than a draw takes. |
| `GuiGpuIndexType`, `GuiGpuIndexCount` | InvalidArgument | no | `Set-GuiGpuSurface` | An index buffer of elements other than `UInt16` and `UInt32`, or more indices than a draw takes. |
| `GuiGpuPipelineOptions` | InvalidArgument | no | `Set-GuiGpuSurface` | Pipeline parameters beside a `PwrsGui.GpuRenderPipeline`, which draws as it was built. |
| `GuiGpuDrawKey`, `GuiGpuDrawValue` | InvalidArgument | no | `Set-GuiGpuSurface -Draw` | A draw that is not a hashtable or has a key other than the draw parameters, or a key's value its parameter cannot take; the message names the draw by its place. |
| `GuiGpuPassKey`, `GuiGpuPassValue` | InvalidArgument | no | `Set-GuiGpuSurface -Compute` | A pass that is not a hashtable or has a key other than `Invoke-GuiGpuCompute`'s parameters and `Every`, or a value its key cannot take, such as an `Every` other than `Frame` and `Once`; the message names the pass by its place. |
| `GuiGpuBlendConstant`, `GuiGpuClearColor` | InvalidArgument | no | `Set-GuiGpuSurface` | A blend constant other than four numbers, or a clear color other than three or four. |
| `GuiGpuClearDepth`, `GuiGpuClearStencil` | InvalidArgument, InvalidOperation | no | `Set-GuiGpuSurface` | A `-ClearDepth` outside 0 to 1, as InvalidArgument. As InvalidOperation, a `-ClearDepth` or `-ClearStencil` for draws whose depth format holds no depth or no stencil, or for draws with no depth attachment. |
| `GuiGpuUniform` | InvalidArgument, InvalidOperation | no | `Set-GuiGpuSurface` | `-Uniform` is no array of numbers, as InvalidArgument; as InvalidOperation, no pass or draw reads the frame uniform it would fill. |
| `GuiGpuSurfaceTargets` | InvalidOperation | no | `Set-GuiGpuSurface` | Draws whose pipelines take different sample counts or depth formats, where a surface's draws share its attachments. |
| `GuiGpuDraw` | InvalidOperation | no | `Set-GuiGpuSurface` | The surface's passes and draws do not record on the device; the message quotes the device's validation error. In an HTML window, the page's WebGPU refuses a shader, a pass or a draw, or WebGL 2 does not compile, link or draw one, or a draw's WGSL cannot be written as GLSL ES 3.00 for it; the message quotes why. |
| `GuiGpuNoWebGpu` | ResourceUnavailable | no | `Set-GuiGpuSurface`, `Save-GuiGpuSurface` | An HTML window's engine has no WebGPU, and the surface needs it: a compute pass, a storage buffer or texture, a blend with a second source, a texture format WebGL 2 takes none of, or a clamped depth bias where the engine's WebGL 2 lacks the EXT_polygon_offset_clamp extension. Or the engine has neither WebGPU nor WebGL 2, or gives the surface no device or context. The message names the engine; a window opened with `-Renderer Egui` draws the surface. |
| `GuiGpuNoCopy` | ObjectNotFound | no | `Set-GuiGpuSurface` | The HTML renderer or the window's page has no copy of a buffer, texture or sampler the surface draws from, or a copy of another size. This is not expected: the module sends each object's copy before the surface. |
| `GuiGpuNoSurface` | ObjectNotFound | no | `Set-GuiGpuSurface`, `Save-GuiGpuSurface`, the camera commands | The window or the surface left the renderer before the command got its reply. |
| `GuiNotASurface` | InvalidArgument | yes | `Set-GuiGpuSurface`, `Save-GuiGpuSurface`, the camera commands | The widget of that name is not a GpuSurface; the message names its kind. |
| `GuiGpuCamera` | InvalidArgument, InvalidOperation | no | `Set-GuiGpuCamera` | A `-Target` other than three numbers, a gesture or move `-Bindings` cannot read, two keys naming one gesture, `-Controls Custom` without `-Bindings` or other controls with them, or a pose or projection a camera cannot take, such as a distance not above 0; the camera is left as it was. |
| `GuiCanvasView` | InvalidArgument | no | `Set-GuiCanvasView` | A `-Center` other than two numbers, a `-Bindings` gesture or move it cannot read or a gesture given a move it cannot make, two keys naming one gesture, a key the window's zoom takes, `-Controls Custom` without `-Bindings` or other controls with them, `-Fit` with `-FitItem`, a fit to a name no item has, `-Easing` without `-Duration`, a duration of 0 or less, or a zoom outside the view's limits; the view is left as it was. |
| `GuiNoCanvas` | ObjectNotFound | no | `Set-GuiCanvasView`, `Get-GuiCanvasView`, `Remove-GuiCanvasView` | The window or the canvas left the renderer before the command got its reply. |
| `GuiGpuNotDrawn` | InvalidOperation | no | `Save-GuiGpuSurface` | The window drew the surface in no frame before the save's deadline, as a surface on a tab not shown or in a minimized window is not drawn; in an HTML window, the page shows the surface on no tab shown, or gives it no room. |
| `GuiGpuSaveFormat` | InvalidArgument | yes | `Save-GuiGpuSurface` | The path does not end in `.png`; nothing is written. |
| `GuiGpuPng`, `GuiGpuSave` | InvalidData, WriteError, InvalidOperation | yes, but not as InvalidOperation | `Save-GuiGpuSurface` | The texels could not be written as a PNG, as InvalidData, or the file could not be written, as WriteError. As InvalidOperation, an HTML window's page could not read the surface back, or returned texels that do not fill its size, which is not expected. |
| `GuiGpuNotLost` | InvalidOperation | no | `Reset-GuiGpuDevice` | The destroyed device did not report itself lost in time. |
| `GuiGpuAnswer`, `GuiGpuHeld` | ProtocolError, InvalidOperation | `GuiGpuAnswer` yes | every GPU command | The renderer replied to a call with another kind of reply, a call reached the renderer's GPU thread that its UI thread handles, or a call for an HTML window's page and the copies it draws from reached the egui renderer. This is not expected. |

PowerShell's binder refuses some arguments before a command runs, with its own ids: `CannotConvertArgumentNoMessage` for a `-Mode`, `-Renderer`, `-WebView`, `-Event` or `-OutputMode` value that names none of the enumeration's members, and `ParameterArgumentValidationError` for a `-Count` outside 1 to 100000, or a `-Width` or `-Height` outside 100 to 10000.

## The messages

`GuiNoDisplay`'s message is `no window can open: ` and the reason: `neither DISPLAY nor WAYLAND_DISPLAY is set` on Linux and FreeBSD, and `this session's window station is not visible, as in a service` on Windows. The others give what the renderer or the operating system said. An `-Adapter` that matches nothing, in a new PowerShell process of the same host, drawing in the helper only:

```powershell
& (Get-Process -Id $PID).Path -NoProfile -Command {
    Set-GuiOption -Mode Helper -Adapter 'NoSuchGpu'
    try { Get-GuiGpuAdapter } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
}
```

```text
GuiStartFailed,Pwrs.Modules.PWRS_GUI.GetGuiGpuAdapterCommand
the GUI renderer could not start: WGPU error: Adapter selection failed: no adapter's name contains NoSuchGpu; the adapters are: NVIDIA GeForce RTX 5070 (Vulkan); AMD Radeon(TM) Graphics (Vulkan); NVIDIA GeForce RTX 5070 (Dx12); AMD Radeon(TM) Graphics (Dx12); Microsoft Basic Render Driver (Dx12); NVIDIA GeForce RTX 5070/PCIe/SSE2 (Gl)
```

The command reference pages show each command's own errors raised in a real run, with their ids and messages.
