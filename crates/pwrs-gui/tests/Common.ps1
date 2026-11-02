# What every PWRS-GUI suite shares. No test can stall the run: a command
# that blocks runs in a background runspace and is stopped, failing its
# test, when it has not returned in time; every wait for a window has a
# deadline; and a watchdog process ends this host when a file outlives its
# own deadline, so a renderer that stops responding fails the run instead
# of stalling it. PWRS_MODULE points at the built module folder, and the
# stand-in module is built beside it.

$ModuleFolder = $env:PWRS_MODULE
if (-not $ModuleFolder) { throw 'PWRS_MODULE is not set' }
$script:GuiModule = Join-Path $ModuleFolder 'PWRS-GUI.psd1'
$script:StandinModule = Join-Path (Join-Path (Split-Path $ModuleFolder) 'PwrsGuiStandin') 'PwrsGuiStandin.psd1'
if ($PSVersionTable.PSVersion.Major -ge 7) { $PSStyle.OutputRendering = 'PlainText' }

# How long a blocking command may take to return once driven.
$script:ReturnSeconds = 20
# How long a window may take to appear, a renderer's start included.
$script:WindowSeconds = 30

# Every background command Start-Background started and Complete-Background
# has not yet disposed of, so Reset-GuiSession can close their runspaces.
$script:Background = [System.Collections.Generic.List[object]]::new()

# Ends this host after $Seconds unless stopped first. Before it does, it
# writes the thread stacks of the helpers this host started
# (Write-HelperStacks), so a suite that outlives its deadline shows where
# each renderer stood. The watchdog loads this file for them, and finds the
# module folder in the environment it inherits.
function Start-Watchdog([int]$Seconds = 600) {
    $hostPath = (Get-Process -Id $PID).Path
    $common = (Join-Path $PSScriptRoot 'Common.ps1') -replace "'", "''"
    $lines = @(
        "Start-Sleep -Seconds $Seconds"
        "Write-Host 'PWRS-GUI test watchdog: the suite outlived ${Seconds}s; the threads of its helpers follow, then its host is ended'"
        ". '$common'"
        "foreach (`$helper in @(Get-OwnHelpers $PID)) { Write-Host `"helper `$(`$helper.Id):`"; Write-HelperStacks `$helper.Id }"
        "Stop-Process -Id $PID -Force"
    )
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($lines -join "`n"))
    $arguments = @('-NoProfile', '-NonInteractive', '-EncodedCommand', $encoded)
    if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
        Start-Process -FilePath $hostPath -ArgumentList $arguments -WindowStyle Hidden -PassThru
    } else {
        Start-Process -FilePath $hostPath -ArgumentList $arguments -PassThru
    }
}

function Stop-Watchdog($Watchdog) {
    if ($Watchdog -and -not $Watchdog.HasExited) { Stop-Process -Id $Watchdog.Id -Force }
}

# The helper processes of both renderers that host $Parent started, this
# host unless named: on Windows from the process table's parent ids,
# elsewhere from ps's pid, ppid and comm columns. PowerShell's
# Process.Parent is null on FreeBSD, which
# mounts no procfs, and BSD ps takes everything after a first `=` in one -o
# as that column's header, so each column has an -o of its own. ps is run
# by the full path found as this file loads, since a suite may take folders
# off PATH: the stand-in dialogs' suite takes off zenity's, which on Ubuntu
# contains ps as well. A helper that exits between its listing and its lookup
# is left out, since nothing of it remains to wait for.
if (-not ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop')) {
    $script:PsPath = (Get-Command ps -CommandType Application -TotalCount 1 -ErrorAction Stop).Source
}

# The tool that writes a hung helper's thread stacks, found as this file
# loads for the same reason as ps: sample on macOS, lldb on FreeBSD and gdb
# on Linux, none where it is not installed. Windows has no such tool in the
# box.
$script:StackTool = if ($IsMacOS) {
    (Get-Command sample -CommandType Application -TotalCount 1 -ErrorAction Ignore).Source
} elseif ($PSVersionTable.OS -match 'FreeBSD') {
    (Get-Command lldb -CommandType Application -TotalCount 1 -ErrorAction Ignore).Source
} elseif ($IsLinux) {
    (Get-Command gdb -CommandType Application -TotalCount 1 -ErrorAction Ignore).Source
}

# Writes every thread's stack of the helper $Id to the host, through
# $script:StackTool; macOS's sample watches the helper for one second.
function Write-HelperStacks([int]$Id) {
    if (-not $script:StackTool) {
        Write-Host "  no tool here writes another process's thread stacks"
        return
    }
    switch ([IO.Path]::GetFileName($script:StackTool)) {
        'sample' {
            $file = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-helper-$Id.sample.txt"
            & $script:StackTool $Id 1 -mayDie -file $file 2>&1 | Out-Host
            if (Test-Path -LiteralPath $file) { Get-Content -LiteralPath $file | Out-Host }
        }
        'lldb' { & $script:StackTool --batch -p $Id -o 'thread backtrace all' 2>&1 | Out-Host }
        'gdb' {
            # Yama's ptrace_scope 1, Ubuntu's default, lets gdb attach to its
            # own descendants alone, so each thread's name and the kernel
            # call it waits in come first, from /proc, which needs no attach.
            foreach ($task in Get-ChildItem -LiteralPath "/proc/$Id/task" -Directory -ErrorAction Ignore) {
                $name = Get-Content -LiteralPath (Join-Path $task.FullName 'comm') -ErrorAction Ignore
                $wait = Get-Content -LiteralPath (Join-Path $task.FullName 'wchan') -ErrorAction Ignore
                Write-Host "  thread $($task.Name) $name waits in $wait"
            }
            & $script:StackTool -batch -p $Id -ex 'thread apply all bt' 2>&1 | Out-Host
        }
    }
}
function Get-OwnHelpers([int]$Parent = $PID) {
    if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
        Get-CimInstance Win32_Process -Filter "ParentProcessId = $Parent AND (Name = 'pwrs-gui-egui.exe' OR Name = 'pwrs-gui-web.exe')" | ForEach-Object { Get-Process -Id $_.ProcessId -ErrorAction Ignore }
    } else {
        foreach ($line in (& $script:PsPath -ax -o 'pid=' -o 'ppid=' -o 'comm=')) {
            $fields = -split $line
            if ($fields.Count -ge 3 -and [int]$fields[1] -eq $Parent -and [IO.Path]::GetFileName($fields[2]) -in 'pwrs-gui-egui', 'pwrs-gui-web') { Get-Process -Id ([int]$fields[0]) -ErrorAction Ignore }
        }
    }
}

# Ends the module's sessions and imports the module again, so the next
# command starts a renderer in $Mode, and widget windows draw with
# $Renderer. The sessions end once no runspace has the module imported, so
# every background runspace a test left open is stopped and closed first.
# Removal unloads nothing, so the import finds the same library, with no
# session in its slots. The ended sessions' helpers are waited for, so none
# is still exiting when a test counts the helpers of the next. A helper
# still running 20 s after the removal has its threads' stacks written and
# is ended before the reset fails, so the next suite starts clean and one
# hang fails one suite.
function Reset-GuiSession([string]$Mode, [string]$Renderer = 'Egui') {
    $ending = @(Get-OwnHelpers)
    foreach ($job in @($script:Background)) {
        if (-not $job.Async.IsCompleted) { $job.PS.Stop() }
        $job.PS.Runspace.Dispose()
        $job.PS.Dispose()
    }
    $script:Background.Clear()
    if (Get-Module PwrsGuiStandin) { Remove-Module PwrsGuiStandin }
    if (Get-Module PWRS-GUI) { Remove-Module PWRS-GUI }
    $hung = @($ending | Where-Object { -not $_.WaitForExit(20000) })
    foreach ($h in $hung) {
        Write-Host "helper $($h.Id) of the ended session was still running 20 s after the module's removal; its threads:"
        Write-HelperStacks $h.Id
        if (-not $h.HasExited) { $h.Kill() }
        if (-not $h.WaitForExit(5000)) { throw "helper $($h.Id) of the ended session hung and could not be ended" }
    }
    if ($hung) {
        $ids = ($hung | ForEach-Object Id) -join ', '
        $what = if ($hung.Count -eq 1) { "helper $ids of the ended session was" } else { "helpers $ids of the ended session were" }
        throw "$what still running 20 s after the module's removal; the threads' stacks are above, and each was ended"
    }
    Import-Module $script:GuiModule
    Set-GuiOption -Mode $Mode -Renderer $Renderer
}

# Runs $Script in a runspace of this process with the modules imported
# here and the same options, so it uses this process's sessions.
function Start-Background([string]$Script) {
    $now = Get-GuiOption
    $imports = "Import-Module '$script:GuiModule'; Set-GuiOption -Mode $($now.Mode) -Renderer $($now.Renderer); "
    if (Get-Module PwrsGuiStandin) { $imports += "Import-Module '$script:StandinModule'; " }
    $runspace = [runspacefactory]::CreateRunspace()
    $runspace.Open()
    $ps = [powershell]::Create()
    $ps.Runspace = $runspace
    [void]$ps.AddScript($imports + $Script)
    $output = [System.Management.Automation.PSDataCollection[psobject]]::new()
    $async = $ps.BeginInvoke([System.Management.Automation.PSDataCollection[psobject]]$null, $output)
    $job = [pscustomobject]@{ PS = $ps; Async = $async; Output = $output; What = $Script }
    $script:Background.Add($job)
    $job
}

# The window titled $Title, once it has opened; fails, naming what the
# background command reported, when none appears in time.
function Wait-GuiWindow([string]$Title, $Job) {
    $deadline = [DateTime]::UtcNow.AddSeconds($script:WindowSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $window = Get-GuiWindow -Title $Title
        if ($window) { return $window }
        if ($Job -and $Job.Async.IsCompleted) { break }
        Start-Sleep -Milliseconds 50
    }
    $why = ''
    if ($Job) {
        $why = " The background command is $($Job.PS.InvocationStateInfo.State): $($Job.PS.InvocationStateInfo.Reason) $(@($Job.PS.Streams.Error) -join '; ')"
        if (-not $Job.Async.IsCompleted) { $Job.PS.Stop() }
    }
    throw "No window titled '$Title' within $($script:WindowSeconds) s.$why"
}

# The output, errors and warnings' messages of a background command, once
# it returns; stops it and fails when it does not return in time. A command
# that was stopped returns what it wrote before the stop.
function Complete-Background($Job, [switch]$Stopped) {
    if (-not $Job.Async.AsyncWaitHandle.WaitOne($script:ReturnSeconds * 1000)) {
        $Job.PS.Stop()
        throw "'$($Job.What)' did not return within $($script:ReturnSeconds) s; it was stopped"
    }
    try {
        # The output went to the job's own collection, so this returns null.
        $null = $Job.PS.EndInvoke($Job.Async)
    } catch [System.Management.Automation.MethodInvocationException] {
        if (-not ($Stopped -and $_.Exception.InnerException -is [System.Management.Automation.PipelineStoppedException])) { throw }
    }
    $errors = @($Job.PS.Streams.Error)
    $warnings = @($Job.PS.Streams.Warning | ForEach-Object Message)
    $output = @($Job.Output)
    $Job.PS.Runspace.Dispose()
    $Job.PS.Dispose()
    [void]$script:Background.Remove($Job)
    [pscustomobject]@{ Output = $output; Errors = $errors; Warnings = $warnings }
}

# Stops a background command as Ctrl+C would, and returns what it wrote.
function Stop-Background($Job) {
    $Job.PS.Stop()
    Complete-Background $Job -Stopped
}

# The node of a snapshot's tree named $Name.
function Find-GuiNode($Node, [string]$Name) {
    if ($Node.Name -eq $Name) { return $Node }
    foreach ($child in $Node.Children) {
        $found = Find-GuiNode $child $Name
        if ($found) { return $found }
    }
}

# Waits until $Condition is true, failing with $What when it is not within
# $Seconds.
function Wait-Until([scriptblock]$Condition, [string]$What, [int]$Seconds = 20) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not (& $Condition)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "$What did not happen within $Seconds s" }
        Start-Sleep -Milliseconds 50
    }
}

# The system clipboard's text as it is, line endings included, and text put
# there. Windows and macOS go through Get-Clipboard and Set-Clipboard. Linux
# and the BSDs go through xclip itself: PowerShell runs xclip on Linux, but
# on FreeBSD its commands keep no clipboard at all, so Set-Clipboard does
# nothing and Get-Clipboard gives nothing. Each reads or writes xclip's
# stream whole, since PowerShell splits a command's output into lines, and
# fails when xclip does, an empty clipboard included, with xclip's own
# words: from its error stream here, and on the host's own when writing.
function Get-SystemClipboard {
    if ($IsWindows -or $IsMacOS -or $PSVersionTable.PSEdition -eq 'Desktop') { return Get-Clipboard -Raw }
    $xclip = [System.Diagnostics.Process]::Start([System.Diagnostics.ProcessStartInfo]@{ FileName = 'xclip'; Arguments = '-selection clipboard -o'; RedirectStandardOutput = $true; RedirectStandardError = $true; UseShellExecute = $false })
    $text = $xclip.StandardOutput.ReadToEnd()
    $why = $xclip.StandardError.ReadToEnd()
    $xclip.WaitForExit()
    if ($xclip.ExitCode -ne 0) { throw "xclip -o exited $($xclip.ExitCode): $why" }
    $text
}

# xclip, given the text, goes on serving it from a process of its own once
# the one started here exits, as PowerShell's Set-Clipboard leaves it, and
# that process keeps the streams it was given until the X server ends. A
# pipe of the host's would keep the host's reader waiting to the end of the
# run, and on FreeBSD a pipe of this process's own keeps pwsh from exiting.
# So xclip writes its output to /dev/null and its errors to a file, read
# when it fails, which it does before it forks.
function Set-SystemClipboard([string]$Text) {
    if ($IsWindows -or $IsMacOS -or $PSVersionTable.PSEdition -eq 'Desktop') { return Set-Clipboard -Value $Text }
    $why = [IO.Path]::GetTempFileName()
    try {
        $start = [System.Diagnostics.ProcessStartInfo]::new('/bin/sh')
        foreach ($argument in '-c', 'exec xclip -selection clipboard -i >/dev/null 2>"$1"', 'sh', $why) { $start.ArgumentList.Add($argument) }
        $start.RedirectStandardInput = $true
        $start.UseShellExecute = $false
        $xclip = [System.Diagnostics.Process]::Start($start)
        # xclip refuses before it reads, as without a display, closing the
        # pipe under the write; its own message then says why.
        $unwritten = $null
        try {
            $xclip.StandardInput.Write($Text)
            $xclip.StandardInput.Close()
        } catch [System.IO.IOException] {
            $unwritten = $_
        }
        $xclip.WaitForExit()
        if ($xclip.ExitCode -ne 0) { throw "xclip -i exited $($xclip.ExitCode): $("$(Get-Content -Raw -LiteralPath $why)".Trim())" }
        if ($unwritten) { throw $unwritten }
    } finally {
        Remove-Item -LiteralPath $why
    }
}
