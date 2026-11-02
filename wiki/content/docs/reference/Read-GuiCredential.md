---
title: Read-GuiCredential
weight: 2
---

Shows a form for a user name and a password and writes them as a `PSCredential`. Source: `crates/pwrs-gui/src/input.rs`, `ReadGuiCredential` and `input_form`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md), in a second runspace of the same process, filled the fields and pressed OK. Its events reach the window as a person's typing and clicks do.

## Syntax

```powershell
Get-Command Read-GuiCredential -Syntax
```

```text
Read-GuiCredential [[-Message] <string>] [-UserName <string>] [-Title <string>] [<CommonParameters>]
```

## Description

`Read-GuiCredential` opens a window titled `Credential`, or `-Title`, and waits. The window shows `-Message` as its main line, then a user name field, filled with `-UserName` when it is given, and a password field, and OK and Cancel at the bottom right in the system's order: OK, Cancel on Windows, and Cancel, OK on macOS, Linux and the BSDs. It fits its fields, from 320 to 548 points across. The user name field has the keyboard as the window opens, or the password field when `-UserName` fills the name, as the system's own prompts do. OK is enabled once a user name is entered, and Enter in either field then takes the form as OK does. Once the password field has the focus and is typed into, an eye at its right end shows the password while it is held, or Alt+F8 is, as [New-GuiTextBox](New-GuiTextBox.md) describes.

OK writes a `PSCredential` of the user name and the password. The password comes from the renderer as text, over the session's rings, which only the current user can open, and the module turns it into a `SecureString` for the credential. Cancel, the window's close button, Escape, and Ctrl+C in the console write nothing; Ctrl+C also closes the window.

The inputs are named `UserName` and `Password`, the buttons `Ok` and `Cancel`, which [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) takes as `-Name`. The wait has no time limit.

## Parameters

### -Message

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: no text above the fields |

The text above the fields.

### -UserName

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: an empty user name, and OK disabled |

The user name the form opens with.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Credential` |

The window's title, which [Get-GuiWindow](Get-GuiWindow.md) matches its `-Title` against.

## Inputs

None.

## Outputs

`System.Management.Automation.PSCredential`. Nothing when the window is canceled, closed or stopped.

## Examples

The examples from the command's help.

Sign in, with a message above the fields:

<!-- focus: its pictures show the field with the keyboard focus as the form opens -->
<!-- keep: read-guicredential-sign-in.png images/read-guicredential-sign-in.png -->
<!-- person: $w = Wait-Window 'Credential'; Invoke-GuiTestEvent -Window $w.Id -Name UserName -Event Change -Value 'builder'; Invoke-GuiTestEvent -Window $w.Id -Name Password -Event Change -Value 'correct horse'; Wait-Enabled $w Ok; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'read-guicredential-sign-in.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Read-GuiCredential -Message 'Sign in to the build server'
```

```text
UserName                     Password
--------                     --------
builder  System.Security.SecureString
```

The form just before OK was pressed, as [Get-GuiSnapshot](Get-GuiSnapshot.md) `-Pixels` captured it in that run:

![The Credential window with the message "Sign in to the build server", a user name field showing builder, a password field showing dots, and OK and Cancel at the bottom right](/images/read-guicredential-sign-in.png)

A user name given up front, so OK is enabled at once and only the password is typed:

<!-- keep: read-guicredential-deploy.png images/read-guicredential-deploy.png -->
<!-- person: $w = Wait-Window 'Deploy'; Wait-Enabled $w Ok; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'read-guicredential-deploy.png'); Invoke-GuiTestEvent -Window $w.Id -Name Password -Event Change -Value 'correct horse'; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$credential = Read-GuiCredential -UserName 'deploy' -Title 'Deploy'
$credential.UserName
$credential.Password.Length
```

```text
deploy
13
```

The form as it opened, with the user name given and OK already enabled:

![The Deploy window with a user name field showing deploy, an empty password field, and OK and Cancel at the bottom right](/images/read-guicredential-deploy.png)
