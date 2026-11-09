# Read-GuiChoice, driven as a person would through Invoke-GuiTestEvent, in
# every mode the platform has and with each renderer. See Common.ps1 for
# the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'Read-GuiChoice in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
    }

    It 'writes the choice selected when OK is pressed' {
        $job = Start-Background "Read-GuiChoice -Prompt 'Pick one' -Choice 'alpha','beta','gamma' -Title 'PWRS-GUI test choice'"
        $w = Wait-GuiWindow 'PWRS-GUI test choice' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1
        Wait-Until { (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Ok').Properties.enabled } 'OK becoming enabled'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -Be @('beta')
    }

    It 'leaves OK disabled until a choice is selected' {
        $job = Start-Background "Read-GuiChoice -Choice 'alpha','beta' -Title 'PWRS-GUI test enable'"
        $w = Wait-GuiWindow 'PWRS-GUI test enable' $job
        (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Ok').Properties.enabled | Should -BeFalse
        Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 0
        Wait-Until { (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Ok').Properties.enabled } 'OK becoming enabled'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        (Complete-Background $job).Output | Should -Be @('alpha')
    }

    It 'writes nothing when Cancel is pressed' {
        $job = Start-Background "Read-GuiChoice -Choice 'one','two' -Title 'PWRS-GUI test cancel'"
        $w = Wait-GuiWindow 'PWRS-GUI test cancel' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 0
        Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -BeNullOrEmpty
    }

    It 'writes nothing and closes its window when stopped as Ctrl+C stops it' {
        $job = Start-Background "Read-GuiChoice -Choice 'one','two' -Title 'PWRS-GUI test stop'"
        $w = Wait-GuiWindow 'PWRS-GUI test stop' $job
        $done = Stop-Background $job
        $done.Output | Should -BeNullOrEmpty
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI test stop') } 'the stopped command''s window closing'
    }

    It 'with -Multiple, writes every choice checked, in the order given' {
        $job = Start-Background "Read-GuiChoice -Choice 'alpha','beta','gamma' -Multiple -Title 'PWRS-GUI test multiple'"
        $w = Wait-GuiWindow 'PWRS-GUI test multiple' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 2, 0
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        (Complete-Background $job).Output | Should -Be @('alpha', 'gamma')
    }

    It 'opens with -Default selected, matched without regard to case, and OK enabled' {
        $job = Start-Background "Read-GuiChoice -Choice 'alpha','beta' -Default 'BETA' -Title 'PWRS-GUI test default'"
        $w = Wait-GuiWindow 'PWRS-GUI test default' $job
        $shot = Get-GuiSnapshot -Window $w.Id
        @($shot.State.Choice) | Should -Be @(1)
        (Find-GuiNode $shot.Tree 'Ok').Properties.enabled | Should -BeTrue
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        (Complete-Background $job).Output | Should -Be @('beta')
    }

    It 'takes a choice activated with a double-click or Enter' {
        $job = Start-Background "Read-GuiChoice -Choice 'alpha','beta','gamma' -Title 'PWRS-GUI test activate'"
        $w = Wait-GuiWindow 'PWRS-GUI test activate' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Activate -Value 2
        (Complete-Background $job).Output | Should -Be @('gamma')
    }

    It 'shows each piped object as Format-Wide shows it' {
        $file = Join-Path ([System.IO.Path]::GetTempPath()) 'pwrs-gui-label-test.txt'
        Set-Content -LiteralPath $file -Value 'label'
        $job = Start-Background "@((Get-Process -Id `$PID), (Get-Item -LiteralPath '$file'), [pscustomobject]@{ Id = 42; Size = 7 }, 'plain') | Read-GuiChoice -Title 'PWRS-GUI test labels'"
        $w = Wait-GuiWindow 'PWRS-GUI test labels' $job
        $items = (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Choice').Properties.items
        $items | Should -Be @((Get-Process -Id $PID).ProcessName, 'pwrs-gui-label-test.txt', '42', 'plain')
        Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1
        Wait-Until { (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Ok').Properties.enabled } 'OK becoming enabled'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $out = (Complete-Background $job).Output
        $out.Count | Should -Be 1
        $out[0].FullName | Should -Be (Get-Item -LiteralPath $file).FullName
        Remove-Item -LiteralPath $file
    }

    It 'shows each choice by the property -Property names' {
        $job = Start-Background "Get-Process -Id `$PID | Read-GuiChoice -Property Id -Title 'PWRS-GUI test property'"
        $w = Wait-GuiWindow 'PWRS-GUI test property' $job
        (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Choice').Properties.items | Should -Be @("$PID")
        Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
        (Complete-Background $job).Output | Should -BeNullOrEmpty
    }

    It 'shows each choice by a -Property script block, matches -Default against it, and writes the objects given' {
        $job = Start-Background "'alpha', 'beta' | Read-GuiChoice -Property { `$_.ToUpper() } -Default 'BETA' -Title 'PWRS-GUI test script label'"
        $w = Wait-GuiWindow 'PWRS-GUI test script label' $job
        $shot = Get-GuiSnapshot -Window $w.Id
        (Find-GuiNode $shot.Tree 'Choice').Properties.items | Should -Be @('ALPHA', 'BETA')
        @($shot.State.Choice) | Should -Be @(1)
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        (Complete-Background $job).Output | Should -Be @('beta')
    }

    It 'refuses a -Property that is neither a property name nor a script block' {
        { Read-GuiChoice -Choice 'alpha' -Property 5 } | Should -Throw '*property name or a script block*'
    }

    It 'writes the piped objects themselves, shown by their Name property' {
        $job = Start-Background "[pscustomobject]@{ Name = 'first' } , [pscustomobject]@{ Name = 'second' } | ForEach-Object { `$_ | Add-Member -PassThru ScriptMethod ToString { `$this.Name } -Force } | Read-GuiChoice -Title 'PWRS-GUI test piped'"
        $w = Wait-GuiWindow 'PWRS-GUI test piped' $job
        (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Choice').Properties.items | Should -Be @('first', 'second')
        Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1
        Wait-Until { (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Ok').Properties.enabled } 'OK becoming enabled'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $out = (Complete-Background $job).Output
        $out.Count | Should -Be 1
        $out[0].Name | Should -Be 'second'
    }

    It 'refuses a default that is not one of the choices, naming it' {
        { Read-GuiChoice -Choice 'alpha' -Default 'omega' } | Should -Throw '*omega*'
    }

    It 'refuses several defaults without -Multiple' {
        { Read-GuiChoice -Choice 'alpha', 'beta' -Default 'alpha', 'beta' } | Should -Throw '*-Multiple*'
    }
}
