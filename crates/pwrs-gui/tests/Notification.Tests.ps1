# Notifications: Show-, Update-, Remove- and Get-GuiNotification, their
# answers and handlers, and the identities Register- and
# Unregister-GuiNotificationApp manage, in every mode the platform has.
# Answers come from Invoke-GuiTestEvent -Notification, as a person's do.
# On Windows a toast shows on the desktop: every toast is silent but one,
# and PWRS-GUI's own identity is left as it was found. On Linux and FreeBSD
# the stand-in module's test bus is the notification server: the suite
# checks what reached it, and answers through its signals as well. On macOS
# every notification is -Quiet, which requests provisional permission, given
# without a prompt. Where macOS refuses PWRS-GUI's notifications even so, as
# GitHub's macOS runners do, each case that shows one is skipped with the
# refusal as its reason, and one case checks the refusal. See Common.ps1 for
# the deadlines.

BeforeDiscovery {
    $windows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
    $overBus = -not $windows -and -not $IsMacOS
    $modes = if ($windows) { 'Helper', 'InProcess' } else { 'Helper' }
    $noBus = $overBus -and -not $env:DBUS_SESSION_BUS_ADDRESS
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
    $script:Windows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
    $script:OverBus = -not $script:Windows -and -not $IsMacOS
    # What every notification is shown with: silent, and on macOS quiet.
    $script:Show = @{ Audio = @{ Silent = $true } }
    if ($IsMacOS) { $script:Show.Quiet = $true }
    $script:Quiet = if ($IsMacOS) { ' -Quiet' } else { '' }
    $script:OwnKey = 'HKCU:\Software\Classes\AppUserModelId\PWRS-GUI'
    $script:OwnWasThere = $script:Windows -and (Test-Path -LiteralPath $script:OwnKey)
    $script:TestApp = 'PWRS-GUI.Tests'

    # The last Notify the test bus took for the summary $Summary.
    function Get-BusNotify([string]$Summary) {
        @(Get-GuiTestBusCall | Where-Object { $_.Call -eq 'Notify' -and $_.Summary -eq $Summary }) | Select-Object -Last 1
    }

    # Skips the running case when macOS refused the suite's first
    # notification, with the refusal as the reason.
    function Skip-WhenRefused {
        if ($script:Refused) { Set-ItResult -Skipped -Because "macOS refuses PWRS-GUI's notifications on this host: $script:Refused" }
    }
}

AfterAll {
    Get-GuiNotification | Remove-GuiNotification
    if ($script:Windows) {
        if (Test-Path -LiteralPath "HKCU:\Software\Classes\AppUserModelId\$script:TestApp") { Unregister-GuiNotificationApp -Id $script:TestApp -Confirm:$false }
        if (-not $script:OwnWasThere -and (Test-Path -LiteralPath $script:OwnKey)) { Unregister-GuiNotificationApp -Id 'PWRS-GUI' -Confirm:$false }
    }
    if ($script:OverBus -and $env:DBUS_SESSION_BUS_ADDRESS) {
        Import-Module $script:StandinModule
        Stop-GuiTestBus
    }
    Stop-Watchdog $script:Watchdog
}

Describe 'A notification, in <_> mode' -ForEach $modes -Skip:$noBus {
    BeforeAll {
        Reset-GuiSession $_
        if ($script:OverBus) {
            Import-Module $script:StandinModule
            Start-GuiTestBus
        }
        # A quiet notification, shown and taken away, finds whether macOS
        # refuses PWRS-GUI's notifications here; any other failure fails the
        # suite.
        $script:Refused = $null
        if ($IsMacOS) {
            try {
                Show-GuiNotification 'PWRS-GUI test permission' @script:Show | Remove-GuiNotification
            } catch {
                if ($_.FullyQualifiedErrorId -notlike 'GuiNotificationDenied*') { throw }
                $script:Refused = $_.Exception.Message
            }
        }
    }

    # The identity is a Windows registry key; macOS shows notifications
    # under PWRS-GUI.app's bundle, and Linux and FreeBSD under the app's name.
    It 'registers PWRS-GUI''s own identity, with its icon copied for its toasts' -Skip:(-not $windows) {
        $n = Show-GuiNotification 'PWRS-GUI test identity' 'Checks the identity' @script:Show
        $key = Get-ItemProperty -LiteralPath $script:OwnKey
        $key.DisplayName | Should -Be 'PWRS-GUI'
        $key.IconUri | Should -Be (Join-Path $env:LOCALAPPDATA 'PWRS-GUI\Notifications\PWRS-GUI.png')
        Test-Path -LiteralPath $key.IconUri | Should -BeTrue
        Remove-GuiNotification $n.Id
    }

    It 'answers a reply: the input''s Change, the button''s Click with the inputs'' state, then Close' {
        Skip-WhenRefused
        $n = Show-GuiNotification 'PWRS-GUI test reply' 'Ann: lunch?' -Input @{ Name = 'Reply'; Placeholder = 'Type a reply' } -Button @{ Text = 'Send'; Input = 'Reply' }, 'Later' @script:Show
        $n.PSObject.TypeNames[0] | Should -Be 'PwrsGui.Notification'
        $n.Names | Should -Be 'Reply', 'Send', 'Later'
        @(Get-GuiNotification | Where-Object Id -EQ $n.Id).Title | Should -Be 'PWRS-GUI test reply'
        Invoke-GuiTestEvent -Notification $n.Id -Name Send -Event Click -Value @{ Reply = 'On my way' }
        $changed = Wait-GuiEvent -Window $n.Id -Timeout 20
        $changed.Kind | Should -Be 'Change'
        $changed.Name | Should -Be 'Reply'
        $changed.Value | Should -Be 'On my way'
        $sent = Wait-GuiEvent -Window $n.Id -Timeout 20
        $sent.Kind | Should -Be 'Click'
        $sent.Name | Should -Be 'Send'
        $sent.State.Reply | Should -Be 'On my way'
        (Wait-GuiEvent -Window $n.Id -Timeout 20).Kind | Should -Be 'Close'
        @(Get-GuiNotification | Where-Object Id -EQ $n.Id).Count | Should -Be 0
    }

    It 'activates on a click on itself, and a timeout dismisses it without ending it' {
        Skip-WhenRefused
        $n = Show-GuiNotification 'PWRS-GUI test activate' @script:Show
        Invoke-GuiTestEvent -Notification $n.Id -Event Dismiss -Value Timeout
        $timedOut = Wait-GuiEvent -Window $n.Id -Timeout 20
        $timedOut.Kind | Should -Be 'Dismiss'
        $timedOut.Value | Should -Be 'Timeout'
        @(Get-GuiNotification | Where-Object Id -EQ $n.Id).Count | Should -Be 1 -Because 'a timed-out notification stays in the system''s list'
        Invoke-GuiTestEvent -Notification $n.Id -Event Activate
        (Wait-GuiEvent -Window $n.Id -Timeout 20).Kind | Should -Be 'Activate'
        (Wait-GuiEvent -Window $n.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'runs -OnClick, -OnActivate and -OnDismiss as engine-event actions' {
        Skip-WhenRefused
        $global:PwrsGuiNotifyHeard = [System.Collections.Generic.List[string]]::new()
        $n = Show-GuiNotification 'PWRS-GUI test handlers' -Button 'Go' -OnClick { param($e) $global:PwrsGuiNotifyHeard.Add("click $($e.Name)") } -OnDismiss { param($e) $global:PwrsGuiNotifyHeard.Add("dismiss $($e.Value)") } @script:Show
        $n.SourceIdentifier | Should -Not -BeNullOrEmpty
        Invoke-GuiTestEvent -Notification $n.Id -Event Dismiss -Value Timeout
        Invoke-GuiTestEvent -Notification $n.Id -Name Go -Event Click
        Wait-Until { $global:PwrsGuiNotifyHeard.Count -eq 2 } 'both handlers running'
        $global:PwrsGuiNotifyHeard | Should -Be 'dismiss Timeout', 'click Go'
        Unregister-Event -SourceIdentifier $n.SourceIdentifier
    }

    It 'moves its progress bar and changes its title in place' {
        Skip-WhenRefused
        $n = Show-GuiNotification 'PWRS-GUI test progress' -Progress @{ Value = 0; Status = 'Starting' } @script:Show
        $n | Update-GuiNotification -Progress @{ Value = 0.5; Status = 'Copying' } -Title 'PWRS-GUI test progress half'
        Wait-Until { (Get-GuiSnapshot -Window $n.Id).Tree.Properties.progress.value -eq 0.5 } 'the progress reaching the renderer'
        (Get-GuiSnapshot -Window $n.Id).Tree.Properties.title | Should -Be 'PWRS-GUI test progress half'
        { Update-GuiNotification -Notification $n.Id -Progress 2 } | Should -Throw -ErrorId 'GuiBadNotification*'
        Remove-GuiNotification $n.Id
        (Wait-GuiEvent -Window $n.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    # A toast's own XML is Windows' alone: Linux and FreeBSD refuse it, and
    # macOS shows a title and a body, which this notification has none of.
    It 'shows a toast of the caller''s own XML, and Remove-GuiNotification takes it away' -Skip:(-not $windows) {
        $xml = '<toast><visual><binding template="ToastGeneric"><text>PWRS-GUI test XML</text><text>From the caller''s own document</text></binding></visual><audio silent="true"/></toast>'
        $n = Show-GuiNotification -Xml $xml
        (Get-GuiSnapshot -Window $n.Id).Tree.Properties.xml | Should -Be $xml
        Remove-GuiNotification $n.Id
        @(Get-GuiNotification | Where-Object Id -EQ $n.Id).Count | Should -Be 0
        Remove-GuiNotification $n.Id -ErrorAction SilentlyContinue -ErrorVariable gone
        $gone[0].FullyQualifiedErrorId | Should -BeLike 'GuiNoNotification*'
    }

    It 'waits with -Wait and writes the answer' {
        Skip-WhenRefused
        $job = Start-Background "Show-GuiNotification 'PWRS-GUI test wait' -Button 'Yes', 'No' -Audio @{ Silent = `$true }$script:Quiet -Wait"
        Wait-Until { @(Get-GuiNotification | Where-Object Title -EQ 'PWRS-GUI test wait').Count -eq 1 } 'the notification showing'
        $n = Get-GuiNotification | Where-Object Title -EQ 'PWRS-GUI test wait'
        Invoke-GuiTestEvent -Notification $n.Id -Name No -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output.Count | Should -Be 1
        $done.Output[0].Kind | Should -Be 'Click'
        $done.Output[0].Name | Should -Be 'No'
    }

    It 'refuses more inputs than the platform shows, and parts it does not take' {
        $inputs = 1..6 | ForEach-Object { "Field$_" }
        { Show-GuiNotification 'PWRS-GUI test inputs' -Input $inputs @script:Show } | Should -Throw -ErrorId 'GuiNotificationInput*'
        { Show-GuiNotification 'PWRS-GUI test sound' -Audio 'Chime' } | Should -Throw -ErrorId 'GuiBadNotification*'
        { Show-GuiNotification 'PWRS-GUI test button' -Button @{ Text = 'Send'; Input = 'Nowhere' } } | Should -Throw -ErrorId 'GuiBadNotification*'
        { Show-GuiNotification } | Should -Throw -ErrorId 'GuiBadNotification*'
    }

    It 'gives GuiNotificationDenied where macOS refuses PWRS-GUI''s notifications' -Skip:(-not $IsMacOS) {
        if (-not $script:Refused) { Set-ItResult -Skipped -Because 'this Mac allows PWRS-GUI''s notifications' }
        { Show-GuiNotification 'PWRS-GUI test refused' @script:Show } | Should -Throw -ErrorId 'GuiNotificationDenied*'
    }

    # The notification server on the session bus is Linux's and FreeBSD's
    # alone: there the suite reads what reached it and answers through it.
    It 'reaches the notification service with its parts and updates, and an action there comes back as a Click' -Skip:(-not $overBus) {
        $n = Show-GuiNotification 'PWRS-GUI test bus' 'Ready to deploy' -Button 'Deploy', 'Later' -Progress @{ Value = 0.25; Status = 'Copying' } @script:Show
        $call = Get-BusNotify 'PWRS-GUI test bus'
        $call | Should -Not -BeNullOrEmpty -Because 'Show-GuiNotification returns once the service has the notification'
        $call.Body | Should -Be 'Ready to deploy'
        $call.App | Should -Be 'PWRS-GUI'
        $call.ActionLabels | Should -Contain 'Deploy'
        $call.ActionLabels | Should -Contain 'Later'
        $call.Hints | Should -Contain 'value=25'
        $call.Hints | Should -Contain 'suppress-sound=true'
        $n | Update-GuiNotification -Progress @{ Value = 0.5; Status = 'Copying' }
        Wait-Until { @(Get-GuiTestBusCall | Where-Object { $_.Call -eq 'Notify' -and $_.ReplacesId -eq $call.Id }).Count -ge 1 } 'the update reaching the service'
        $update = @(Get-GuiTestBusCall | Where-Object { $_.Call -eq 'Notify' -and $_.ReplacesId -eq $call.Id })[-1]
        $update.Id | Should -Be $call.Id -Because 'an update replaces the notification it updates'
        $update.Hints | Should -Contain 'value=50'
        $key = $call.ActionKeys[[array]::IndexOf($call.ActionLabels, 'Deploy')]
        Invoke-GuiTestBusNotification -Notification $call.Id -Action $key
        $clicked = Wait-GuiEvent -Window $n.Id -Timeout 20
        $clicked.Kind | Should -Be 'Click'
        $clicked.Name | Should -Be 'Deploy'
        (Wait-GuiEvent -Window $n.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'takes a reply and a close from the notification service, and takes a notification away there' -Skip:(-not $overBus) {
        $n = Show-GuiNotification 'PWRS-GUI test bus reply' 'Ann: lunch?' -Input @{ Name = 'Reply'; Placeholder = 'Type a reply' } -Button @{ Text = 'Send'; Input = 'Reply' } @script:Show
        $call = Get-BusNotify 'PWRS-GUI test bus reply'
        $call.ActionKeys | Should -Contain 'inline-reply'
        Invoke-GuiTestBusNotification -Notification $call.Id -Reply 'On my way'
        $changed = Wait-GuiEvent -Window $n.Id -Timeout 20
        $changed.Kind | Should -Be 'Change'
        $changed.Value | Should -Be 'On my way'
        $sent = Wait-GuiEvent -Window $n.Id -Timeout 20
        $sent.Kind | Should -Be 'Click'
        $sent.Name | Should -Be 'Send'
        $sent.State.Reply | Should -Be 'On my way'
        (Wait-GuiEvent -Window $n.Id -Timeout 20).Kind | Should -Be 'Close'

        $d = Show-GuiNotification 'PWRS-GUI test bus dismiss' @script:Show
        $dismissed = Get-BusNotify 'PWRS-GUI test bus dismiss'
        Invoke-GuiTestBusNotification -Notification $dismissed.Id -Closed 2
        $closed = Wait-GuiEvent -Window $d.Id -Timeout 20
        $closed.Kind | Should -Be 'Dismiss'
        $closed.Value | Should -Be 'User'
        (Wait-GuiEvent -Window $d.Id -Timeout 20).Kind | Should -Be 'Close'

        $r = Show-GuiNotification 'PWRS-GUI test bus remove' @script:Show
        $removed = Get-BusNotify 'PWRS-GUI test bus remove'
        Remove-GuiNotification $r.Id
        @(Get-GuiTestBusCall | Where-Object { $_.Call -eq 'CloseNotification' -and $_.Id -eq $removed.Id }).Count | Should -Be 1
    }
}

Describe 'A notification identity of the caller''s own' -Skip:(-not ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop')) {
    BeforeAll {
        Reset-GuiSession Helper
    }

    It 'is registered with its name and icon, shows notifications, and is removed in full' {
        $icon = Join-Path $TestDrive 'tests.svg'
        Set-Content -LiteralPath $icon -Value '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#2a6fb5"/></svg>'
        Register-GuiNotificationApp -Id $script:TestApp -Name 'PWRS-GUI Tests' -Icon $icon
        $key = "HKCU:\Software\Classes\AppUserModelId\$script:TestApp"
        (Get-ItemProperty -LiteralPath $key).DisplayName | Should -Be 'PWRS-GUI Tests'
        (Get-ItemProperty -LiteralPath $key).IconUri | Should -Be (Resolve-Path -LiteralPath $icon).Path
        $n = Show-GuiNotification 'PWRS-GUI test own app' 'The one audible toast of the run' -App $script:TestApp
        Remove-GuiNotification $n.Id
        Unregister-GuiNotificationApp -Id $script:TestApp -Confirm:$false
        Test-Path -LiteralPath $key | Should -BeFalse
        Test-Path -LiteralPath "HKCU:\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\$script:TestApp" | Should -BeFalse
        Unregister-GuiNotificationApp -Id $script:TestApp -Confirm:$false -ErrorAction SilentlyContinue -ErrorVariable gone
        $gone[0].FullyQualifiedErrorId | Should -BeLike 'GuiNoNotificationApp*'
        { Register-GuiNotificationApp -Id 'has space' -Name 'x' } | Should -Throw -ErrorId 'GuiBadAppId*'
    }
}
