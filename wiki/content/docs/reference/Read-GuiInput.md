---
title: Read-GuiInput
weight: 2
---

Shows a form of fields with OK and Cancel and writes what was entered as one object. Source: `crates/pwrs-gui/src/input.rs`, `ReadGuiInput`, `field_of` and `input_form`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md), in a second runspace of the same process, filled the fields and pressed OK. Its events reach the window as a person's typing and clicks do.

## Syntax

```powershell
Get-Command Read-GuiInput -Syntax
```

```text
Read-GuiInput [[-Prompt] <string>] [-Field] <Object[]> [-Title <string>] [<CommonParameters>]
```

## Description

`Read-GuiInput` opens a window titled `Input`, or `-Title`, and waits. The window shows `-Prompt` as its main line, then a label and an input for each field, in the order given, and OK and Cancel at the bottom right in the system's order: OK, Cancel on Windows, and Cancel, OK on macOS, Linux and the BSDs. It fits its fields, from 320 to 548 points across and up to 756 points down. The first field has the keyboard as the window opens, so a person types into it at once. OK writes one `PSCustomObject`, of the type name `PwrsGui.Input`, with a property per field under its name, in the order given. Enter in a text field takes the form as OK does. Cancel, the window's close button, Escape, and Ctrl+C in the console write nothing; Ctrl+C also closes the window.

Each `-Field` is a hashtable of these keys, and any other key is refused:

| Key | What it gives | When none is given |
|---|---|---|
| `Name` | the property the field's value goes under, and the input's widget name | refused: every field needs one |
| `Label` | the text beside the input | the `Name` |
| `Type` | `Text`, `Password`, `Number`, `Date`, `Check` or `Choice` | `Text` |
| `Default` | the value the field opens with; a `Password` field takes none, and a `Choice` field's names one of its choices, without regard to case | empty, 0, no date, unchecked, or no choice |
| `Choice` | a `Choice` field's choices, which only it takes | refused for a `Choice` field |

Each property is the field's value, of the kind its `Type` gives:

| Type | The input | The property |
|---|---|---|
| `Text` | a text box | `System.String` |
| `Password` | a password box, whose eye shows the password while it is held, as [New-GuiTextBox](New-GuiTextBox.md) describes | `System.Security.SecureString` |
| `Number` | a number box | `System.Double` |
| `Date` | a date and time chooser, to the minute | `System.DateTime`, or `$null` when none was picked |
| `Check` | a check box | `System.Boolean` |
| `Choice` | a drop-down list | `System.String`, the choice, or `$null` when none was picked |

The wait has no time limit. The session's renderer draws the window, as [Set-GuiOption](Set-GuiOption.md) says.

## Parameters

### -Prompt

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: no text above the fields |

The text above the fields.

### -Field

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: mandatory, position 1 |
| Pipeline input | no |
| Refused with | `GuiBadField`, naming the field, for a key, a type or a default it cannot take, a field named Ok or Cancel, which are the buttons' names, and two fields of one name |

The fields, each a hashtable as the description says.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Input` |

The window's title, which [Get-GuiWindow](Get-GuiWindow.md) matches its `-Title` against.

## Inputs

None.

## Outputs

`System.Management.Automation.PSCustomObject`, of the type name `PwrsGui.Input`, a property per field. Nothing when the window is canceled, closed or stopped.

## Examples

The examples from the command's help.

A user's login and password; the password arrives as a `SecureString`:

<!-- focus: its pictures show the first field with the keyboard focus as the form opens -->
<!-- keep: read-guiinput-login.png images/read-guiinput-login.png -->
<!-- person: $w = Wait-Window 'Input'; Invoke-GuiTestEvent -Window $w.Id -Name Login -Event Change -Value 'ada'; Invoke-GuiTestEvent -Window $w.Id -Name Password -Event Change -Value 'correct horse'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'read-guiinput-login.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Read-GuiInput -Prompt 'New user' -Field @{ Name = 'Login' }, @{ Name = 'Password'; Type = 'Password' }
```

```text
Login                     Password
-----                     --------
ada   System.Security.SecureString
```

The form just before OK was pressed, as [Get-GuiSnapshot](Get-GuiSnapshot.md) `-Pixels` captured it in that run:

![The Input window with the prompt "New user", a Login field showing ada, a Password field showing dots, and OK and Cancel at the bottom right](/images/read-guiinput-login.png)

A number that opens at 3 and a date picked in the form:

<!-- keep: read-guiinput-count-due.png images/read-guiinput-count-due.png -->
<!-- person: $w = Wait-Window 'Input'; Invoke-GuiTestEvent -Window $w.Id -Name Due -Event Change -Value '2026-10-20T09:30'; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'read-guiinput-count-due.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Read-GuiInput -Field @{ Name = 'Count'; Type = 'Number'; Default = 3 }, @{ Name = 'Due'; Type = 'Date' }
```

```text
Count Due
----- ---
 3.00 10/20/2026 9:30:00 AM
```

<!-- powershell -->

```text
Count Due
----- ---
    3 10/20/2026 9:30:00 AM
```

The form with the date picked:

![The Input window with a Count number field at 3 and a Due date field naming the date picked, its hour and minute beside it, all in one field look, and OK and Cancel at the bottom right](/images/read-guiinput-count-due.png)

A choice that opens on its default, taken as it is:

<!-- keep: read-guiinput-choice.png images/read-guiinput-choice.png -->
<!-- person: $w = Wait-Window 'Input'; Wait-Enabled $w Ok; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'read-guiinput-choice.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Read-GuiInput -Field @{ Name = 'Env'; Label = 'Deploy to'; Type = 'Choice'; Choice = 'staging', 'production'; Default = 'staging' }
```

```text
Env
---
staging
```

The form as it opened, the drop-down on its default:

![The Input window with a Deploy to drop-down showing staging, and OK and Cancel at the bottom right](/images/read-guiinput-choice.png)

## Errors

Every error of its own is terminating, raised before any window opens:

```powershell
foreach ($attempt in { Read-GuiInput -Field @{ Name = 'Ok' } },
                     { Read-GuiInput -Field @{ Name = 'Pin'; Type = 'Password'; Default = '1234' } },
                     { Read-GuiInput -Field @{ Name = 'Env'; Type = 'Choice'; Choice = 'a', 'b'; Default = 'c' } },
                     { Read-GuiInput -Field @{ Name = 'Size'; Color = 'red' } }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
```

```text
GuiBadField,Pwrs.Modules.PWRS_GUI.ReadGuiInputCommand: -Field 1: Ok names one of the form's buttons, Ok and Cancel
GuiBadField,Pwrs.Modules.PWRS_GUI.ReadGuiInputCommand: -Field 1: Pin is a Password field, which takes no Default; the person types it
GuiBadField,Pwrs.Modules.PWRS_GUI.ReadGuiInputCommand: -Field 1: Env's Default 'c' is not one of its choices
GuiBadField,Pwrs.Modules.PWRS_GUI.ReadGuiInputCommand: -Field 1: Color is none of Name, Label, Type, Default and Choice
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed` and `GuiRendererGone`.
