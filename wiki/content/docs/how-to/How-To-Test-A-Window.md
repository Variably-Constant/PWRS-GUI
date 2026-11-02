---
title: How To Test A Window
weight: 7
---

Drive a dialog from a test as a person would, read it back, and check what the command wrote, with a deadline on every wait. Source: `crates/pwrs-gui/src/inspect.rs` for the commands, and `crates/pwrs-gui/tests/Common.ps1`, the helpers PWRS-GUI's own Pester suites use.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each in a process of its own, with Pester 6.2.0 in both. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths.

## The pattern

A command that shows a window, such as `Read-GuiChoice`, waits in its runspace until the window closes, so the test drives the window from another runspace of the same process. PWRS-GUI has one session per renderer for the process, shared by every runspace, so the test finds and drives the window the command opened:

1. Start the command in a second runspace with `[powershell]::Create()` and `BeginInvoke()`, which work the same in both hosts.
2. Find its window by title with `Get-GuiWindow`, giving up after a deadline.
3. Act with `Invoke-GuiTestEvent` and read with `Get-GuiSnapshot`, naming widgets as `Get-GuiWindow` lists them.
4. Take the command's output with `EndInvoke`, after waiting on the result with a deadline, and stopping the command when it has not returned.

Each step that waits has a deadline, so a window that never opens or a command that never returns fails the test instead of stalling the run. Ctrl+C, which `Stop()` raises, closes the command's window.

## The helpers

```powershell
function Start-Dialog([scriptblock]$Script) {
    $ps = [powershell]::Create().AddScript($Script)
    [pscustomobject]@{ PS = $ps; Async = $ps.BeginInvoke() }
}

function Wait-Window([string]$Title, [int]$Seconds = 30) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $window = Get-GuiWindow -Title $Title
        if ($window) { return $window }
        Start-Sleep -Milliseconds 50
    }
    throw "no window titled '$Title' within $Seconds s"
}

function Complete-Dialog($Dialog, [int]$Seconds = 20) {
    if (-not $Dialog.Async.AsyncWaitHandle.WaitOne($Seconds * 1000)) {
        $Dialog.PS.Stop()
        throw "the dialog did not return within $Seconds s; it was stopped"
    }
    $output = $Dialog.PS.EndInvoke($Dialog.Async)
    $Dialog.PS.Dispose()
    $output
}

function Find-Node($Node, [string]$Name) {
    if ($Node.Name -eq $Name) { return $Node }
    foreach ($child in $Node.Children) {
        $found = Find-Node $child $Name
        if ($found) { return $found }
    }
}
```

## Drive a dialog

Choose a row and press OK; the command writes the choice:

```powershell
$dialog = Start-Dialog { Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production' -Title 'Deploy' }
$window = Wait-Window 'Deploy'
Invoke-GuiTestEvent -Window $window.Id -Name Choice -Event Select -Value 1
Invoke-GuiTestEvent -Window $window.Id -Name Ok -Event Click
Complete-Dialog $dialog
```

```text
production
```

Read the window's state before acting: OK is disabled until a row is selected, then enabled. The renderer applies a change on its next frame, so a check that follows one waits for it with a deadline:

```powershell
$dialog = Start-Dialog { Read-GuiChoice -Choice 'staging', 'production' -Title 'Deploy' }
$window = Wait-Window 'Deploy'
(Find-Node (Get-GuiSnapshot -Window $window.Id).Tree 'Ok').Properties.enabled
Invoke-GuiTestEvent -Window $window.Id -Name Choice -Event Select -Value 0
$deadline = [DateTime]::UtcNow.AddSeconds(10)
while (-not (Find-Node (Get-GuiSnapshot -Window $window.Id).Tree 'Ok').Properties.enabled -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
(Find-Node (Get-GuiSnapshot -Window $window.Id).Tree 'Ok').Properties.enabled
Invoke-GuiTestEvent -Window $window.Id -Name Cancel -Event Click
$null -eq (Complete-Dialog $dialog)
```

```text
False
True
True
```

Stop the command as Ctrl+C would; it writes nothing and its window closes:

```powershell
$dialog = Start-Dialog { Read-GuiChoice -Choice 'one', 'two' -Title 'Stopped' }
$window = Wait-Window 'Stopped'
$dialog.PS.Stop()
$dialog.PS.Dispose()
$deadline = [DateTime]::UtcNow.AddSeconds(10)
while ((Get-GuiWindow -Title 'Stopped') -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
@(Get-GuiWindow -Title 'Stopped').Count
```

```text
0
```

## In Pester

The same helpers in a Pester file, testing a script's own prompt. Pester 5 or later is needed. Windows PowerShell 5.1 ships 3.4.0; [Pester's installation guide](https://pester.dev/docs/introduction/installation) installs a current version beside it with `Install-Module -Name Pester -Force -SkipPublisherCheck`.

```powershell
@'
BeforeAll {
    Import-Module PWRS-GUI

    # The script's own command under test.
    $script:Prompt = { Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production' -Title 'Deploy' }

    function Start-Dialog([scriptblock]$Script) {
        $ps = [powershell]::Create().AddScript($Script)
        [pscustomobject]@{ PS = $ps; Async = $ps.BeginInvoke() }
    }
    function Wait-Window([string]$Title, [int]$Seconds = 30) {
        $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
        while ([DateTime]::UtcNow -lt $deadline) {
            $window = Get-GuiWindow -Title $Title
            if ($window) { return $window }
            Start-Sleep -Milliseconds 50
        }
        throw "no window titled '$Title' within $Seconds s"
    }
    function Complete-Dialog($Dialog, [int]$Seconds = 20) {
        if (-not $Dialog.Async.AsyncWaitHandle.WaitOne($Seconds * 1000)) {
            $Dialog.PS.Stop()
            throw "the dialog did not return within $Seconds s; it was stopped"
        }
        $output = $Dialog.PS.EndInvoke($Dialog.Async)
        $Dialog.PS.Dispose()
        $output
    }
}

Describe 'the deploy prompt' {
    It 'writes the target chosen' {
        $dialog = Start-Dialog $script:Prompt
        $window = Wait-Window 'Deploy'
        Invoke-GuiTestEvent -Window $window.Id -Name Choice -Event Select -Value 1
        Invoke-GuiTestEvent -Window $window.Id -Name Ok -Event Click
        Complete-Dialog $dialog | Should -Be 'production'
    }

    It 'writes nothing when canceled' {
        $dialog = Start-Dialog $script:Prompt
        $window = Wait-Window 'Deploy'
        Invoke-GuiTestEvent -Window $window.Id -Name Cancel -Event Click
        Complete-Dialog $dialog | Should -BeNullOrEmpty
    }
}
'@ | Set-Content -Path .\Deploy.Tests.ps1
```

```powershell
Import-Module Pester -MinimumVersion 5.0
$result = Invoke-Pester -Path .\Deploy.Tests.ps1 -PassThru -Output None
$result.Result
$result.Tests | Select-Object ExpandedName, Result
```

```text
Passed

ExpandedName                 Result
------------                 ------
writes the target chosen     Passed
writes nothing when canceled Passed
```

PWRS-GUI's own suites add a watchdog process that ends the PowerShell host when a whole file outlives its deadline, so even a renderer that stops responding cannot stall a CI run. Before it ends the host, it writes the thread stacks of the host's helpers, which show where each renderer stood. `crates/pwrs-gui/tests/Common.ps1` has it as `Start-Watchdog`.
