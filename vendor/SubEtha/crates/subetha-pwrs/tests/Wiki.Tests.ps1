# Every PowerShell program the SubEtha Guide shows, run as a reader runs it.
#
# A powershell block that names a file (```powershell
# {filename="producer.ps1"}) is a program. A page's PowerShell programs run
# in page order from one fresh folder, with the temporary directory pointed
# at it as well, each as a process of the host running this suite, which
# imports the module this build produced by its name. Each runs to its end
# before the next starts unless it is marked alongside="true", and none
# runs longer than 60 seconds. A program marked for one operating system
# (os="linux") runs there only. A program passes when it exits
# successfully and prints what the Output block after it shows.

BeforeDiscovery {
    . (Join-Path $PSScriptRoot 'Wiki.ps1')
    $wikiPages = @(foreach ($found in Get-SEWikiPages) {
            $programs = @($found.Programs | Where-Object Surface -CEQ 'PowerShell')
            if ($programs.Count -gt 0) { @{ Page = $found.Page; Programs = $programs } }
        })
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    . (Join-Path $PSScriptRoot 'Wiki.ps1')
    $module = $env:PWRS_MODULE
    if (-not $module) { throw 'PWRS_MODULE is not set' }
    if ((Split-Path -Leaf $module) -cne 'SubEtha') { throw "PWRS_MODULE $module is not a folder named SubEtha, which Import-Module SubEtha finds" }
    $script:moduleRoot = Split-Path -Parent $module
    $script:exe = [System.Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
    $script:os = if ($IsLinux) { 'linux' } elseif ($IsMacOS) { 'macos' } elseif ($PSVersionTable.Platform -eq 'Unix') { 'freebsd' } else { 'windows' }
}

Describe 'The Guide''s PowerShell programs' {
    It 'finds PowerShell programs in the Guide' {
        @(Get-SEWikiPages | ForEach-Object { $_.Programs } | Where-Object Surface -CEQ 'PowerShell').Count | Should -BeGreaterThan 0
    }

    It 'runs every PowerShell program on <Page> and each prints what it shows' -ForEach $wikiPages {
        $folder = New-SubEthaScratch 'wiki'
        $failures = [System.Collections.Generic.List[string]]::new()
        try {
            $alongside = [System.Collections.Generic.List[object]]::new()
            foreach ($program in $Programs) {
                if ($null -ne $program.Os -and $program.Os -cne $script:os) {
                    Write-Host "for another operating system: $($program.Page):$($program.Line) ($($program.Os))"
                    continue
                }
                $started = Start-SEWikiProgram -Program $program -Folder $folder -Exe $script:exe -ModuleRoot $script:moduleRoot
                if ($program.Alongside) {
                    $alongside.Add($started)
                    continue
                }
                $failure = Test-SEWikiProgram -Started $started -DeadlineSeconds 60
                if ($null -ne $failure) { $failures.Add($failure) }
            }
            foreach ($started in $alongside) {
                $failure = Test-SEWikiProgram -Started $started -DeadlineSeconds 60
                if ($null -ne $failure) { $failures.Add($failure) }
            }
        } finally {
            Remove-SEWikiFolder $folder
        }
        $failures -join "`n`n" | Should -BeNullOrEmpty
    }
}
