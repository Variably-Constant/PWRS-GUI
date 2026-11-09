---
title: Read-GuiChoice
weight: 1
---

Shows a list of choices in a window and writes the one chosen, or every one checked. Source: `crates/pwrs-gui/src/choice.rs`, `ReadGuiChoice`, `LABELS` and `preset_rows`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks came from `Invoke-GuiTestEvent` in a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows. They reach the window as a person's clicks do.

## Syntax

```powershell
Get-Command Read-GuiChoice -Syntax
```

```text
Read-GuiChoice [[-Prompt] <string>] [-Choice] <Object[]> [-Default <Object[]>] [-Multiple] [-Title <string>] [-Property <Object>] [<CommonParameters>]
```

## Description

`Read-GuiChoice` opens a window titled `Choose`, or `-Title`, with `-Prompt` as its main line above a list of the choices, and waits. OK and Cancel are at the bottom right in the system's order: OK, Cancel on Windows, and Cancel, OK on macOS, Linux and the BSDs. Choices come from `-Choice` and from the pipeline, in the order given, and the command opens its window once its input has ended.

The window fits its choices, from 320 to 548 points across and up to 756 points down, past which the list scrolls. A choice too long for the widest window is cut short.

The list has the keyboard as the window opens, its first choice outlined, or the first choice `-Default` selects. Without `-Multiple` one choice is selected at a time, by a click or the arrow keys, and OK is enabled once one is. Double-clicking a choice, or pressing Enter on a selected one, takes it at once. With `-Multiple` each choice has a check box: the arrows move without checking, Space checks or clears the choice, and Enter presses OK, which takes every one checked. OK writes the chosen objects themselves, in the order given, not their text. Cancel, the window's close button, Escape, and Ctrl+C in the console write nothing; Ctrl+C also closes the window.

Each choice is shown by what Format-Wide shows for it:

1. a string or a value type, as itself;
2. else the entry of the first wide view `Get-FormatData` gives one of its type names, preferring an entry selected by one of its type names to the entry selected by none;
3. else its type's `DefaultDisplayProperty`;
4. else its first property named like `name`, `id`, `key`, `*key`, `*name` or `*id`, tried in that order;
5. else its `ToString()`.

`-Property` names a property or gives a script block instead. `-Default` is matched against the same text, without regard to case.

The wait has no time limit. The session's renderer draws the window, and the first window opened in the process starts that renderer, as [Set-GuiOption](Set-GuiOption.md) says.

## Parameters

### -Prompt

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: no text above the list |

The text above the choices.

### -Choice

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: mandatory, position 1 |
| Pipeline input | by value |
| Refused with | `GuiNoChoices` when no choice arrives |

The choices. Objects piped to the command are added to them in the order piped.

### -Default

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: nothing selected, and without `-Multiple` OK disabled |
| Refused with | `GuiDefaultNotAChoice` for one that matches no choice; `GuiTooManyDefaults` for more than one without `-Multiple` |

The choice, or with `-Multiple` the choices, selected when the window opens, matched against the text each choice is shown by, without regard to case. An object given here is shown by the same rule first.

### -Multiple

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |

Lets any number of choices be checked, and writes every one checked.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Choose` |

The window's title, which [Get-GuiWindow](Get-GuiWindow.md) matches its `-Title` against.

### -Property

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: what Format-Wide shows |
| Refused with | `GuiPropertyInvalid` for anything but a string or a script block |

What each choice is shown by: a property name, or a script block run with `$_` as the choice, as Format-Wide's `-Property` takes. A choice without the property is shown empty.

## Inputs

Any object, by value to `-Choice`.

## Outputs

The chosen objects, as given: `System.Object`, of whatever type each choice is. Nothing when the window is canceled, closed or stopped.

Three services, a process, a file and a custom object, each shown as Format-Wide shows it; the list's items are read from a second runspace while the window is open:

```powershell
'notes' | Set-Content -Path .\notes.txt
$dialog = [powershell]::Create().AddScript({
    param($File)
    @((Get-Service -Name Dhcp, Dnscache, W32Time) + (Get-Process -Id $PID) + (Get-Item $File) + [pscustomobject]@{ Id = 42; Size = 7 }) |
        Read-GuiChoice -Title 'What it shows'
}).AddArgument((Resolve-Path .\notes.txt).Path)
$running = $dialog.BeginInvoke()
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($window = Get-GuiWindow -Title 'What it shows') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
$list = (Get-GuiSnapshot -Window $window.Id).Tree.Children | Where-Object Name -eq Choice
$list.Properties.items
```

```text
Dhcp
Dnscache
W32Time
pwsh
notes.txt
42
```

In Windows PowerShell 5.1, whose process is `powershell`:

<!-- powershell -->
```text
Dhcp
Dnscache
W32Time
powershell
notes.txt
42
```

The services read by their names, here through the `name` rule; the process by `ProcessName`, from its wide view; the file by `Name`, from its wide view; and the custom object by `Id`, through the `id` rule. Choosing the file writes the `FileInfo` itself:

```powershell
Invoke-GuiTestEvent -Window $window.Id -Name Choice -Event Select -Value 4
Invoke-GuiTestEvent -Window $window.Id -Name Ok -Event Click
$chosen = $dialog.EndInvoke($running)
$dialog.Dispose()
$chosen[0].GetType().FullName
$chosen[0].Name
```

```text
System.IO.FileInfo
notes.txt
```

## Examples

The examples from the command's help.

Choose one of two strings:

<!-- focus: its pictures show the list with the keyboard focus as the dialog opens -->
<!-- keep: read-guichoice-one.png images/read-guichoice-one.png -->
<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1; Wait-Enabled $w Ok; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'read-guichoice-one.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production'
```

```text
production
```

The window just before OK was pressed, as [Get-GuiSnapshot](Get-GuiSnapshot.md) `-Pixels` captured it in that run:

![The Choose window with the prompt "Deploy to" above the choices staging and production, production selected, and OK and Cancel at the bottom right](/images/read-guichoice-one.png)

Check any number of piped services; here the first and the last are checked, and the command writes the services themselves:

<!-- keep: read-guichoice-multiple.png images/read-guichoice-multiple.png -->
<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 0, 2; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'read-guichoice-multiple.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Get-Service -Name Dhcp, Dnscache, W32Time | Read-GuiChoice -Prompt 'Restart which?' -Multiple
```

```text
Status   Name               DisplayName
------   ----               -----------
Running  Dhcp               DHCP Client
Stopped  W32Time            Windows Time
```

With `-Multiple`, each choice has a check box:

![The Choose window with the prompt "Restart which?" above three check boxes, Dhcp and W32Time checked and Dnscache clear, and OK and Cancel at the bottom right](/images/read-guichoice-multiple.png)

Show the services by their display names instead, and take one by double-clicking it:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Activate -Value 1 -->
```powershell
Get-Service -Name Dhcp, Dnscache, W32Time | Read-GuiChoice -Property DisplayName | Select-Object Name, DisplayName
```

```text
Name     DisplayName
----     -----------
Dnscache DNS Client
```

Open with a default selected, matched without regard to case, so OK is enabled at once:

<!-- person: $w = Wait-Window 'Environment'; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Read-GuiChoice -Choice 'Staging', 'Production', 'Lab' -Default 'LAB' -Title 'Environment'
```

```text
Lab
```

A script block as `-Property`:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 0; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
1..3 | ForEach-Object { [pscustomobject]@{ Number = $_; Square = $_ * $_ } } |
    Read-GuiChoice -Property { '{0} squared is {1}' -f $_.Number, $_.Square }
```

```text
Number Square
------ ------
     1      1
```

Cancel writes nothing:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
$choice = Read-GuiChoice -Choice 'yes', 'no'
$null -eq $choice
```

```text
True
```

## Errors

Every error of its own is terminating, since it leaves nothing to choose from:

```powershell
foreach ($attempt in { @() | Read-GuiChoice },
                     { Read-GuiChoice -Choice 'alpha' -Default 'omega' },
                     { Read-GuiChoice -Choice 'alpha', 'beta' -Default 'alpha', 'beta' },
                     { Read-GuiChoice -Choice 'alpha' -Property 5 }) {
    try { & $attempt } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
}
```

```text
GuiNoChoices,Pwrs.Modules.PWRS_GUI.ReadGuiChoiceCommand: Read-GuiChoice was given no choices
GuiDefaultNotAChoice,Pwrs.Modules.PWRS_GUI.ReadGuiChoiceCommand: the default 'omega' is not one of the choices
GuiTooManyDefaults,Pwrs.Modules.PWRS_GUI.ReadGuiChoiceCommand: 2 defaults were given for one choice; -Multiple takes several
GuiPropertyInvalid,Pwrs.Modules.PWRS_GUI.ReadGuiChoiceCommand: -Property takes a property name or a script block, and was given a System.Int32
```

The [Error Reference](Error-Reference.md) describes these and the session's errors, such as `GuiStartFailed` and `GuiRendererGone`.
