# Read-GuiInput and Read-GuiCredential, driven as a person would through
# Invoke-GuiTestEvent, in every mode the platform has and with each
# renderer, and Read-GuiInput's checks of its fields, which open no window.
# See Common.ps1 for the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
    # A SecureString's text, as the person typed it.
    function Get-Plain($secure) { [System.Net.NetworkCredential]::new('', $secure).Password }
    function Test-OkEnabled($id) { (Find-GuiNode (Get-GuiSnapshot -Window $id).Tree 'Ok').Properties.enabled }
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'Read-GuiInput fields' {
    BeforeAll {
        Import-Module $script:GuiModule
    }

    It 'refuses <Why>, naming what is wrong' -ForEach @(
        @{ Why = 'a key no field takes'; Fields = @(@{ Name = 'A'; Size = 3 }); Says = 'Size is none of Name, Label, Type, Default and Choice' }
        @{ Why = 'a field without a Name'; Fields = @(@{ Label = 'Server' }); Says = 'needs a Name' }
        @{ Why = 'a Type of no field'; Fields = @(@{ Name = 'A'; Type = 'Slider' }); Says = 'Type Slider is none of' }
        @{ Why = 'a Default for a Password field'; Fields = @(@{ Name = 'P'; Type = 'Password'; Default = 'x' }); Says = 'takes no Default' }
        @{ Why = 'a Choice field without choices'; Fields = @(@{ Name = 'C'; Type = 'Choice' }); Says = 'needs Choice' }
        @{ Why = 'a Default that is no choice'; Fields = @(@{ Name = 'C'; Type = 'Choice'; Choice = 'a', 'b'; Default = 'z' }); Says = "Default 'z' is not one of its choices" }
        @{ Why = 'choices for a Text field'; Fields = @(@{ Name = 'T'; Choice = 'a' }); Says = 'only a Choice field takes Choice' }
        @{ Why = 'a Number Default that is not finite'; Fields = @(@{ Name = 'N'; Type = 'Number'; Default = [double]::NaN }); Says = 'finite' }
        @{ Why = 'a field that is not a hashtable'; Fields = @('Server'); Says = 'a field is a hashtable' }
        @{ Why = 'two fields of one name'; Fields = @(@{ Name = 'Host' }, @{ Name = 'host' }); Says = 'two fields are named host' }
        @{ Why = 'a field named for a button'; Fields = @(@{ Name = 'ok' }); Says = "names one of the form's buttons" }
    ) {
        { Read-GuiInput -Field $Fields } | Should -Throw -ErrorId 'GuiBadField*' -ExpectedMessage "*$Says*"
    }
}

Describe 'Read-GuiInput in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
        $script:Fields = "@{ Name = 'Login'; Default = 'ada' }, @{ Name = 'Secret'; Type = 'Password' }, @{ Name = 'Count'; Type = 'Number'; Default = 3 }, @{ Name = 'Due'; Type = 'Date' }, @{ Name = 'Agree'; Type = 'Check' }, @{ Name = 'Env'; Label = 'Deploy to'; Type = 'Choice'; Choice = 'staging', 'production'; Default = 'staging' }"
    }

    It 'writes each field under its name, in order, as its type gives it' {
        $job = Start-Background "Read-GuiInput -Prompt 'Fill in' -Field $Fields -Title 'PWRS-GUI test input'"
        $w = Wait-GuiWindow 'PWRS-GUI test input' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Login -Event Change -Value 'grace'
        Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Change -Value 'hunter2'
        Invoke-GuiTestEvent -Window $w.Id -Name Count -Event Change -Value 7
        Invoke-GuiTestEvent -Window $w.Id -Name Due -Event Change -Value ([datetime]'2026-11-02T09:30')
        Invoke-GuiTestEvent -Window $w.Id -Name Agree -Event Change -Value $true
        Invoke-GuiTestEvent -Window $w.Id -Name Env -Event Change -Value 1
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $read = @($done.Output)
        $read.Count | Should -Be 1
        $read[0].PSObject.Properties.Name | Should -Be @('Login', 'Secret', 'Count', 'Due', 'Agree', 'Env')
        $read[0].Login | Should -Be 'grace'
        $read[0].Secret | Should -BeOfType [securestring]
        Get-Plain $read[0].Secret | Should -Be 'hunter2'
        $read[0].Count | Should -BeOfType [double]
        $read[0].Count | Should -Be 7
        $read[0].Due | Should -BeOfType [datetime]
        $read[0].Due | Should -Be ([datetime]'2026-11-02T09:30')
        $read[0].Agree | Should -BeTrue
        $read[0].Env | Should -Be 'production'
    }

    It 'opens each field with its Default, which OK writes unchanged' {
        $job = Start-Background "Read-GuiInput -Field $Fields -Title 'PWRS-GUI test input defaults'"
        $w = Wait-GuiWindow 'PWRS-GUI test input defaults' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $read = @($done.Output)[0]
        $read.Login | Should -Be 'ada'
        Get-Plain $read.Secret | Should -Be ''
        $read.Count | Should -Be 3
        $read.Due | Should -BeNullOrEmpty
        $read.Agree | Should -BeFalse
        $read.Env | Should -Be 'staging'
    }

    It 'gives its first field the keyboard as it opens' {
        $job = Start-Background "Read-GuiInput -Field $Fields -Title 'PWRS-GUI test input focus'"
        $w = Wait-GuiWindow 'PWRS-GUI test input focus' $job
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Focused -eq 'Login' } 'the first field taking the keyboard'
        Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
        (Complete-Background $job).Errors | Should -BeNullOrEmpty
    }

    It 'takes the form when Enter is pressed in a text field' {
        $job = Start-Background "Read-GuiInput -Field @{ Name = 'Login' }, @{ Name = 'Count'; Type = 'Number' } -Title 'PWRS-GUI test input enter'"
        $w = Wait-GuiWindow 'PWRS-GUI test input enter' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Login -Event Submit -Value 'kay'
        $read = @((Complete-Background $job).Output)
        $read.Count | Should -Be 1
        $read[0].Login | Should -Be 'kay'
        $read[0].Count | Should -Be 0
    }

    It 'writes nothing when Cancel is pressed' {
        $job = Start-Background "Read-GuiInput -Field @{ Name = 'Login' } -Title 'PWRS-GUI test input cancel'"
        $w = Wait-GuiWindow 'PWRS-GUI test input cancel' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Login -Event Change -Value 'typed'
        Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -BeNullOrEmpty
    }

    It 'writes nothing and closes its window when stopped as Ctrl+C stops it' {
        $job = Start-Background "Read-GuiInput -Field @{ Name = 'Login' } -Title 'PWRS-GUI test input stop'"
        $null = Wait-GuiWindow 'PWRS-GUI test input stop' $job
        $done = Stop-Background $job
        $done.Output | Should -BeNullOrEmpty
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI test input stop') } 'the stopped command''s window closing'
    }
}

Describe 'Read-GuiCredential in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
    }

    It 'leaves OK disabled until a user name is entered, then writes a PSCredential' {
        $job = Start-Background "Read-GuiCredential -Message 'Sign in' -Title 'PWRS-GUI test credential'"
        $w = Wait-GuiWindow 'PWRS-GUI test credential' $job
        Test-OkEnabled $w.Id | Should -BeFalse
        Invoke-GuiTestEvent -Window $w.Id -Name UserName -Event Change -Value 'deploy'
        Wait-Until { Test-OkEnabled $w.Id } 'OK becoming enabled'
        Invoke-GuiTestEvent -Window $w.Id -Name Password -Event Change -Value 's3cret'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $read = @($done.Output)
        $read.Count | Should -Be 1
        $read[0] | Should -BeOfType [pscredential]
        $read[0].UserName | Should -Be 'deploy'
        $read[0].GetNetworkCredential().Password | Should -Be 's3cret'
    }

    It 'opens with -UserName and OK enabled, and Enter in the password field takes it' {
        $job = Start-Background "Read-GuiCredential -UserName 'ops' -Title 'PWRS-GUI test credential user'"
        $w = Wait-GuiWindow 'PWRS-GUI test credential user' $job
        Test-OkEnabled $w.Id | Should -BeTrue
        Invoke-GuiTestEvent -Window $w.Id -Name Password -Event Submit -Value 'pw'
        $read = @((Complete-Background $job).Output)
        $read.Count | Should -Be 1
        $read[0].UserName | Should -Be 'ops'
        $read[0].GetNetworkCredential().Password | Should -Be 'pw'
    }

    It 'gives the user name field the keyboard as it opens, and the password field when -UserName fills the name' -ForEach @(
        @{ Given = ''; Field = 'UserName' }
        @{ Given = "-UserName 'ops'"; Field = 'Password' }
    ) {
        $job = Start-Background "Read-GuiCredential $Given -Title 'PWRS-GUI test credential focus $Field'"
        $w = Wait-GuiWindow "PWRS-GUI test credential focus $Field" $job
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Focused -eq $Field } "the $Field field taking the keyboard"
        Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
        (Complete-Background $job).Errors | Should -BeNullOrEmpty
    }

    It 'writes nothing when Cancel is pressed' {
        $job = Start-Background "Read-GuiCredential -UserName 'ops' -Title 'PWRS-GUI test credential cancel'"
        $w = Wait-GuiWindow 'PWRS-GUI test credential cancel' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -BeNullOrEmpty
    }

    It 'writes nothing and closes its window when stopped as Ctrl+C stops it' {
        $job = Start-Background "Read-GuiCredential -Title 'PWRS-GUI test credential stop'"
        $null = Wait-GuiWindow 'PWRS-GUI test credential stop' $job
        $done = Stop-Background $job
        $done.Output | Should -BeNullOrEmpty
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI test credential stop') } 'the stopped command''s window closing'
    }
}
