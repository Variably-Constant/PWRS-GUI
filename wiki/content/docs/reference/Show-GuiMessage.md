---
title: Show-GuiMessage
weight: 2
---

Shows a message in the system's own message box and writes the button pressed. Source: `crates/pwrs-gui/src/dialogs.rs`, `ShowGuiMessage`, and the crate `pwrs-gui-dialogs`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same, except where its output is shown too. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) `-Dialog`, in a second runspace of the same process, pressed each box's button. It closes a box as a person's click does. The pictures are the boxes as Windows drew them in those runs.

## Syntax

```powershell
Get-Command Show-GuiMessage -Syntax
```

```text
Show-GuiMessage [-Message] <string> [[-Title] <string>] [-Icon <MessageIcon>] [-Buttons <MessageButtons>] [<CommonParameters>]
```

## Description

`Show-GuiMessage` shows `-Message` in the system's own message box and waits for a button to be pressed. The box is titled `PWRS-GUI`, or `-Title`, with the icon `-Icon` names and the buttons `-Buttons` names:

| System | The box |
|---|---|
| Windows | `MessageBoxW`, with each of its four icons |
| macOS | an alert the system's notification agent draws, the title as its heading; a question shows the informational alert, since macOS alerts have no question style |
| Linux and the BSDs | zenity's box, its icon given with the flag zenity's version takes; where zenity is not installed, the renderer draws its own box, as [Get-GuiDialog](Get-GuiDialog.md) says |

It writes the button pressed, as `PwrsGui.MessageResult`. A box closed without a button, as by its close button or Escape where the system allows it, writes `Cancel` where the box shows Cancel, `No` for Yes and No alone, and `Ok` for OK alone. Ctrl+C in the console closes the box and writes nothing.

The box is the session's. [Get-GuiDialog](Get-GuiDialog.md) lists it while it is open, from any runspace of the process, and `Invoke-GuiTestEvent -Dialog` presses one of its buttons as a person would. In an HTML session, the egui renderer draws a box the system cannot show, and starts first when it is not running. The wait has no time limit.

## Parameters

### -Message

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |

The message.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | `PWRS-GUI` |

The box's title, which [Get-GuiDialog](Get-GuiDialog.md) matches its `-Title` against.

### -Icon

| | |
|---|---|
| Type | `PwrsGui.MessageIcon`: `Info`, `Warning`, `Error`, `Question` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Info` |

The icon beside the message.

### -Buttons

| | |
|---|---|
| Type | `PwrsGui.MessageButtons`: `Ok`, `OkCancel`, `YesNo`, `YesNoCancel` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Ok` |

The buttons the box shows.

## Inputs

None.

## Outputs

`PwrsGui.MessageResult`: `Ok`, `Cancel`, `Yes` or `No`. Nothing when the wait was stopped.

<!-- keep: show-guimessage-disk.png images/show-guimessage-disk.png -->
<!-- person: $d = Wait-Dialog 'Disk'; Save-DialogPng 'Disk' (Join-Path $PWD 'show-guimessage-disk.png'); Invoke-GuiTestEvent -Dialog $d.Id -Value Cancel -->
```powershell
$pressed = Show-GuiMessage 'The disk is full.' -Title 'Disk' -Icon Error -Buttons OkCancel
$pressed.GetType().FullName
$pressed
```

```text
PwrsGui.MessageResult
Cancel
```

![A Windows message box titled Disk with the error icon, the message "The disk is full." and the buttons OK and Cancel](/images/show-guimessage-disk.png)

## Examples

The examples from the command's help.

Tell the person something:

<!-- keep: show-guimessage-info.png images/show-guimessage-info.png -->
<!-- person: $d = Wait-Dialog 'PWRS-GUI'; Save-DialogPng 'PWRS-GUI' (Join-Path $PWD 'show-guimessage-info.png'); Invoke-GuiTestEvent -Dialog $d.Id -Value Ok -->
```powershell
Show-GuiMessage 'The copy finished.'
```

```text
Ok
```

![A Windows message box titled PWRS-GUI with the information icon, the message "The copy finished." and an OK button](/images/show-guimessage-info.png)

Check with the person before acting; here they press No, so nothing is removed:

<!-- keep: show-guimessage-question.png images/show-guimessage-question.png -->
<!-- person: $d = Wait-Dialog 'PWRS-GUI'; Save-DialogPng 'PWRS-GUI' (Join-Path $PWD 'show-guimessage-question.png'); Invoke-GuiTestEvent -Dialog $d.Id -Value No -->
```powershell
if ((Show-GuiMessage 'Delete the old logs?' -Icon Question -Buttons YesNo) -eq 'Yes') { Remove-Item *.log }
```

![A Windows message box titled PWRS-GUI with the question icon, the message "Delete the old logs?" and the buttons Yes and No](/images/show-guimessage-question.png)

A box of Yes and No closed without a button writes No:

<!-- keep: show-guimessage-yesno.png images/show-guimessage-yesno.png -->
<!-- person: $d = Wait-Dialog 'Keep'; Save-DialogPng 'Keep' (Join-Path $PWD 'show-guimessage-yesno.png'); Invoke-GuiTestEvent -Dialog $d.Id -->
```powershell
Show-GuiMessage 'Keep the settings?' -Title 'Keep' -Buttons YesNo
```

```text
No
```

![A Windows message box titled Keep with the information icon, the message "Keep the settings?" and the buttons Yes and No](/images/show-guimessage-yesno.png)

## Errors

A test that presses a button the box does not show is refused, and the box stays open. Here a second runspace tries two buttons the box refuses, then presses OK:

<!-- varies: the dialog's number -->
```powershell
$runspace = [powershell]::Create().AddScript({
    $d = $null
    while (-not $d) { $d = Get-GuiDialog -Title 'Refused'; Start-Sleep -Milliseconds 50 }
    try { Invoke-GuiTestEvent -Dialog $d.Id -Value Yes -ErrorAction Stop } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
    try { Invoke-GuiTestEvent -Dialog $d.Id -Value Maybe } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
    Invoke-GuiTestEvent -Dialog $d.Id -Value Ok
})
$refusing = $runspace.BeginInvoke()
Show-GuiMessage 'Go on?' -Title 'Refused' -Buttons OkCancel
$runspace.EndInvoke($refusing)
$runspace.Dispose()
```

```text
Ok
GuiRefused,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand: the GUI renderer refused: dialog 24, Message "Refused": it shows [Ok, Cancel], not Yes
GuiBadAnswer,Pwrs.Modules.PWRS_GUI.InvokeGuiTestEventCommand: a message is answered with Ok, Cancel, Yes or No, not Maybe
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed` and `GuiRendererGone`.
