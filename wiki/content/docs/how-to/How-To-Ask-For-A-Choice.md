---
title: How To Ask For A Choice
weight: 1
---

Ask for one of several things, or any number of them, with a default, labels of your own, and a script that tells a choice from none. Source: `crates/pwrs-gui/src/choice.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths. The clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows; a person's clicks give the same output.

## One of several strings

<!-- person: $w = Wait-Window 'Restart'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$when = Read-GuiChoice -Prompt 'Restart the server' -Choice 'now', 'tonight', 'at the weekend' -Title 'Restart'
switch ($when) {
    'now'     { 'Restarting now' }
    $null     { 'Not restarting' }
    default   { "Scheduled for $when" }
}
```

```text
Scheduled for tonight
```

A double-click or Enter on a row takes it at once, without OK.

## Nothing chosen

Cancel, Escape, the window's close button and Ctrl+C in the console write nothing. Test for `$null`, since an empty string can be a choice:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
$answer = Read-GuiChoice -Choice 'keep', 'discard'
if ($null -eq $answer) { 'Canceled, so nothing changes' }
```

```text
Canceled, so nothing changes
```

## A default

`-Default` selects a choice when the window opens, matched without regard to case, so OK is enabled at once and Enter takes the default:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Read-GuiChoice -Prompt 'Log level' -Choice 'Error', 'Warning', 'Information', 'Verbose' -Default 'information'
```

```text
Information
```

A default that is not one of the choices is refused before the window opens, as `GuiDefaultNotAChoice`.

## Any number

`-Multiple` gives each choice a check box and writes every one checked, in the order given; `-Default` may then name several:

<!-- person: $w = Wait-Window 'Features'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 0, 2, 3; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$features = Read-GuiChoice -Prompt 'Install' -Choice 'Core', 'Docs', 'Samples', 'Tools' -Default 'Core', 'Tools' -Multiple -Title 'Features'
$features -join ', '
```

```text
Core, Samples, Tools
```

## Objects, not just strings

Pipe in any objects. Each is shown as Format-Wide would show it, and the command writes back the objects themselves, so what follows works on them directly:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$culture = [System.Globalization.CultureInfo]::GetCultures('SpecificCultures') |
    Where-Object Name -in 'en-GB', 'fr-FR', 'de-DE' | Sort-Object Name | Read-GuiChoice
$culture.GetType().Name
$culture.DisplayName
```

```text
CultureInfo
English (United Kingdom)
```

## Labels of your own

`-Property` names the property to show, or gives a script block, with `$_` as each choice:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 2; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$sizes = 1, 10, 100 | ForEach-Object { [pscustomobject]@{ Megabytes = $_; Bytes = $_ * 1MB } }
$sizes | Read-GuiChoice -Prompt 'Buffer size' -Property { '{0} MB' -f $_.Megabytes } | Select-Object -ExpandProperty Bytes
```

```text
104857600
```

`-Default` is matched against the same labels.

## In a script that may run where no window can open

A command that cannot open a window fails with a terminating error, such as `GuiNoDisplay` on a Linux machine with no display, so a script can fall back to a prompt in the console:

<!-- skip: shows a fallback, which this machine with a display never takes -->
```powershell
try {
    $when = Read-GuiChoice -Choice 'now', 'later'
} catch {
    if ($_.FullyQualifiedErrorId -notlike 'GuiNoDisplay,*') { throw }
    $when = Read-Host 'now or later?'
}
```

[How To Handle Errors](How-To-Handle-Errors.md) lists the errors a command can raise.
