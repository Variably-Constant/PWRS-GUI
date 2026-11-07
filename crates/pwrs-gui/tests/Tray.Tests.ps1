# Tray icons: New-, Update-, Remove- and Get-GuiTrayIcon, their menus and
# handlers, in every mode the platform has. Clicks and menu choices come
# from Invoke-GuiTestEvent -TrayIcon, as a person's do. On Linux and
# FreeBSD a tray icon goes over the session bus, so the suite runs where
# one is, with the stand-in module's test bus as the tray: its watcher
# takes the icon as a StatusNotifierItem, and the suite checks the item
# and its menu there, and clicks and chooses through it. See Common.ps1
# for the deadlines.

BeforeDiscovery {
    $windows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
    $overBus = -not $windows -and -not $IsMacOS
    $modes = if ($windows) { 'Helper', 'InProcess' } else { 'Helper' }
    $noBus = $overBus -and -not $env:DBUS_SESSION_BUS_ADDRESS
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
    $script:OverBus = -not ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') -and -not $IsMacOS
    $script:Icon = Join-Path $TestDrive 'tray.svg'
    Set-Content -LiteralPath $script:Icon -Value '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><circle cx="16" cy="16" r="14" fill="#b5532a"/></svg>'
}

AfterAll {
    Get-GuiTrayIcon | Remove-GuiTrayIcon
    if ($script:OverBus -and $env:DBUS_SESSION_BUS_ADDRESS) {
        Import-Module $script:StandinModule
        Stop-GuiTestBus
    }
    Stop-Watchdog $script:Watchdog
}

Describe 'A tray icon, in <_> mode' -ForEach $modes -Skip:$noBus {
    BeforeAll {
        Reset-GuiSession $_
        if ($script:OverBus) {
            Import-Module $script:StandinModule
            Start-GuiTestBus
        }
    }

    It 'shows with its tooltip and menu, and its clicks and choices wait for Wait-GuiEvent, its Close last' {
        $tray = New-GuiTrayIcon -Icon $script:Icon -Tooltip 'PWRS-GUI test tray' -Menu { MenuItem 'Open' -Name Open; Separator; Menu 'More' { MenuItem 'Quit' -Name Quit } }
        $tray.PSObject.TypeNames[0] | Should -Be 'PwrsGui.TrayIcon'
        $tray.Tooltip | Should -Be 'PWRS-GUI test tray'
        $tray.Names | Should -Be 'Open', 'Quit'
        $listed = @(Get-GuiTrayIcon | Where-Object Id -EQ $tray.Id)
        $listed.Count | Should -Be 1
        $listed[0].Tooltip | Should -Be 'PWRS-GUI test tray'
        @(Get-GuiWindow | Where-Object Id -EQ $tray.Id).Count | Should -Be 0 -Because 'a tray icon is no window'
        Invoke-GuiTestEvent -TrayIcon $tray.Id -Event Click
        $click = Wait-GuiEvent -Window $tray.Id -Timeout 20
        $click.Kind | Should -Be 'Click'
        $click.Name | Should -BeNullOrEmpty
        Invoke-GuiTestEvent -TrayIcon $tray.Id -Name Quit -Event Click
        $chosen = Wait-GuiEvent -Window $tray.Id -Timeout 20
        $chosen.Kind | Should -Be 'Click'
        $chosen.Name | Should -Be 'Quit'
        Remove-GuiTrayIcon -TrayIcon $tray.Id
        @(Get-GuiTrayIcon | Where-Object Id -EQ $tray.Id).Count | Should -Be 0
        (Wait-GuiEvent -Window $tray.Id -Timeout 20).Kind | Should -Be 'Close'
        Remove-GuiTrayIcon -TrayIcon $tray.Id -ErrorAction SilentlyContinue -ErrorVariable gone
        $gone[0].FullyQualifiedErrorId | Should -BeLike 'GuiNoTrayIcon*'
    }

    It 'runs -OnClick and a menu item''s -OnClick as engine-event actions' {
        $global:PwrsGuiTrayHeard = [System.Collections.Generic.List[string]]::new()
        $tray = New-GuiTrayIcon $script:Icon 'PWRS-GUI test tray handlers' -OnClick { $global:PwrsGuiTrayHeard.Add('icon') } -Menu { MenuItem 'Run' -Name Run -OnClick { param($e) $global:PwrsGuiTrayHeard.Add($e.Name) } }
        $tray.SourceIdentifier | Should -Not -BeNullOrEmpty
        Invoke-GuiTestEvent -TrayIcon $tray.Id -Event Click
        Invoke-GuiTestEvent -TrayIcon $tray.Id -Name Run -Event Click
        Wait-Until { $global:PwrsGuiTrayHeard.Count -eq 2 } 'both handlers running'
        $global:PwrsGuiTrayHeard | Should -Be 'icon', 'Run'
        Remove-GuiTrayIcon $tray.Id
        Unregister-Event -SourceIdentifier $tray.SourceIdentifier
    }

    It 'takes a new tooltip and a new menu, whose items'' handlers replace the old ones' {
        $global:PwrsGuiTrayHeard = [System.Collections.Generic.List[string]]::new()
        $tray = New-GuiTrayIcon $script:Icon 'before' -Menu { MenuItem 'Old' -Name Old -OnClick { $global:PwrsGuiTrayHeard.Add('old') } }
        $tray | Update-GuiTrayIcon -Tooltip 'after' -Menu { MenuItem 'New' -Name New -OnClick { $global:PwrsGuiTrayHeard.Add('new') } }
        Wait-Until { (Get-GuiTrayIcon | Where-Object Id -EQ $tray.Id).Tooltip -eq 'after' } 'the new tooltip reaching the renderer'
        (Get-GuiTrayIcon | Where-Object Id -EQ $tray.Id).Names | Should -Be 'New'
        { Invoke-GuiTestEvent -TrayIcon $tray.Id -Name Old -Event Click -ErrorAction Stop } | Should -Throw '*no menu item named Old*'
        Invoke-GuiTestEvent -TrayIcon $tray.Id -Name New -Event Click
        Wait-Until { $global:PwrsGuiTrayHeard.Count -eq 1 } 'the new item''s handler running'
        $global:PwrsGuiTrayHeard | Should -Be 'new'
        Update-GuiWidget -Window $tray.Id -Name New -Property @{ Enabled = $false }
        Wait-Until {
            try { Invoke-GuiTestEvent -TrayIcon $tray.Id -Name New -Event Click -ErrorAction Stop; $false } catch { "$_" -like '*disabled*' }
        } 'the item turning disabled'
        Remove-GuiTrayIcon $tray.Id
        Unregister-Event -SourceIdentifier $tray.SourceIdentifier
    }

    It 'refuses a picture it cannot read and a menu of anything but items, menus and separators' {
        $text = Join-Path $TestDrive 'not-a-picture.png'
        Set-Content -LiteralPath $text -Value 'plain text'
        { New-GuiTrayIcon -Icon $text } | Should -Throw -ErrorId 'GuiBadIcon*'
        { New-GuiTrayIcon -Icon $script:Icon -Menu { Button 'Go' } } | Should -Throw -ErrorId 'GuiNotAllowedHere*'
        { Update-GuiTrayIcon -TrayIcon 999999 -Tooltip 'x' } | Should -Throw -ErrorId 'GuiNoTrayIcon*'
    }

    # The session bus's tray reads the item and its menu as a panel does,
    # and clicks and chooses as a person does, on Linux and FreeBSD alone,
    # where tray icons go over D-Bus.
    It 'reaches the session bus as an item with its tooltip and menu, which a click and a choice there answer' -Skip:(-not $overBus) {
        $tray = New-GuiTrayIcon $script:Icon 'PWRS-GUI test bus tray' -Menu { MenuItem 'Open' -Name Open; Separator; Menu 'More' { MenuItem 'Quit' -Name Quit -Enabled:$false } }
        Wait-Until { @(Get-GuiTestBusItem -Item $tray.Id).Count -eq 1 } 'the item reaching the session bus'
        $item = Get-GuiTestBusItem -Item $tray.Id
        $item.Tooltip | Should -Be 'PWRS-GUI test bus tray'
        $item.Menu | Should -Be 'Open', '-', 'More', 'More/Quit'
        $item.Disabled | Should -Be 'More/Quit'
        $item.IconSizes | Should -Not -BeNullOrEmpty
        Invoke-GuiTestBusItem -Item $tray.Id
        $click = Wait-GuiEvent -Window $tray.Id -Timeout 20
        $click.Kind | Should -Be 'Click'
        $click.Name | Should -BeNullOrEmpty
        Invoke-GuiTestBusItem -Item $tray.Id -MenuItem 'Open'
        $chosen = Wait-GuiEvent -Window $tray.Id -Timeout 20
        $chosen.Kind | Should -Be 'Click'
        $chosen.Name | Should -Be 'Open'
        $tray | Update-GuiTrayIcon -Tooltip 'PWRS-GUI test bus tray changed' -Menu { MenuItem 'Stop' -Name Stop }
        Wait-Until { (Get-GuiTestBusItem -Item $tray.Id).Tooltip -eq 'PWRS-GUI test bus tray changed' } 'the new tooltip reaching the session bus'
        Wait-Until { ((Get-GuiTestBusItem -Item $tray.Id).Menu -join ',') -eq 'Stop' } 'the new menu reaching the session bus'
        Remove-GuiTrayIcon $tray.Id
        Wait-Until { @(Get-GuiTestBusItem -Item $tray.Id).Count -eq 0 } 'the item leaving the session bus'
        (Wait-GuiEvent -Window $tray.Id -Timeout 20).Kind | Should -Be 'Close'
    }
}
