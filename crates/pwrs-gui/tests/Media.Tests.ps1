# The media player, in each renderer and mode the platform has. Each
# renderer plays the clips in media/ that its player claims to their end,
# with their picture and their sound, and raises the designed error for its
# player's most common gap: Media Foundation and WebView2 on Windows,
# AVFoundation and WKWebView on macOS, GStreamer and WebKitGTK elsewhere.
# Then the commands that drive a player. See Common.ps1 for the deadlines.

BeforeDiscovery {
    $windows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
    $modes = if ($windows) { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }

    # The clips each player claims, and its most common gap. On Windows the
    # HEVC clip plays where the HEVC Video Extensions are installed and is
    # the gap where they are not; AIFF, which no Windows player reads, is
    # the gap on every Windows host.
    $hevc = $windows -and [bool](Get-ChildItem -Path 'HKLM:\SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\PackageRepository\Packages' -ErrorAction SilentlyContinue | Where-Object PSChildName -like 'Microsoft.HEVCVideoExtension*')
    $plan = if ($windows) {
        @{
            Egui = @{
                Plays = @('h264-aac.mp4', 'h264-aac.mov', 'h264-aac.mkv', 'h264-aac.ts', 'mpeg4-aac.3gp', 'wmv2-wmav2.wmv', 'tone.mp3', 'tone.m4a', 'tone.aac', 'tone.wav', 'tone.flac', 'tone.wma', 'tone.amr') + @(if ($hevc) { 'hevc-aac.mp4' })
                Gaps = @('tone.aiff') + @(if (-not $hevc) { 'hevc-aac.mp4' })
            }
            Html = @{
                Plays = @('h264-aac.mp4', 'h264-aac.mov', 'h264-aac.mkv', 'vp8-vorbis.webm', 'vp9-opus.webm', 'av1-opus.webm', 'tone.mp3', 'tone.m4a', 'tone.aac', 'tone.wav', 'tone.flac', 'tone.ogg', 'tone.opus')
                Gaps = @('tone.aiff')
            }
        }
    } elseif ($IsMacOS) {
        @{
            Egui = @{ Plays = @('h264-aac.mp4', 'h264-aac.mov', 'hevc-aac.mp4', 'mpeg4-aac.3gp', 'tone.m4a', 'tone.mp3', 'tone.wav', 'tone.aiff', 'tone.caf', 'tone.aac', 'tone.flac', 'tone.ac3'); Gaps = @('vp9-opus.webm') }
            Html = @{ Plays = @('h264-aac.mp4', 'h264-aac.mov', 'hevc-aac.mp4', 'tone.m4a', 'tone.mp3', 'tone.wav', 'tone.aac'); Gaps = @('theora-vorbis.ogv') }
        }
    } else {
        @{
            Egui = @{ Plays = @('vp8-vorbis.webm', 'vp9-opus.webm', 'theora-vorbis.ogv', 'mjpeg-pcm.avi', 'tone.wav', 'tone.flac', 'tone.ogg', 'tone.opus', 'tone.mp3'); Gaps = @('h264-aac.mp4') }
            Html = @{ Plays = @('vp8-vorbis.webm', 'vp9-opus.webm', 'theora-vorbis.ogv', 'tone.wav', 'tone.flac', 'tone.ogg', 'tone.opus', 'tone.mp3'); Gaps = @('h264-aac.mp4') }
        }
    }
    # The clips whose own text track each renderer's player reports, and the
    # same clip without one, whose picture is the same: Media Foundation
    # reports a SubRip track in Matroska, where the VP9 Video Extensions
    # play it; GStreamer and WebKitGTK report it too; WebKit reports MP4's
    # timed text, and AVFoundation's legible output does in egui windows.
    # WebView2 reports none.
    $vp9 = $windows -and [bool](Get-ChildItem -Path 'HKLM:\SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\PackageRepository\Packages' -ErrorAction SilentlyContinue | Where-Object PSChildName -like 'Microsoft.VP9VideoExtensions*')
    $inBand = if ($windows) {
        @{ Egui = @(if ($vp9) { 'vp9-opus-subs.mkv' }); Html = @() }
    } elseif ($IsMacOS) {
        @{ Egui = @('h264-aac-subs.mp4'); Html = @('h264-aac-subs.mp4') }
    } else {
        @{ Egui = @('vp9-opus-subs.mkv'); Html = @('vp9-opus-subs.mkv') }
    }
    $plain = @{ 'vp9-opus-subs.mkv' = 'vp9-opus.webm'; 'h264-aac-subs.mp4' = 'h264-aac.mp4' }
    foreach ($c in $cases) {
        $c.Plays = @($plan[$c.Renderer].Plays | ForEach-Object { @{ Clip = $_ } })
        $c.Gaps = @($plan[$c.Renderer].Gaps | ForEach-Object { @{ Clip = $_ } })
        # Whether the renderer measures the sound a player hands the
        # system: HTML windows through Web Audio, and egui windows through
        # each backend's own tap, on every platform here.
        $c.Measured = $true
        $c.InBand = @($inBand[$c.Renderer] | ForEach-Object { @{ Clip = $_; Plain = $plain[$_] } })
    }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Clips = Join-Path $PSScriptRoot 'media'
    $script:Watchdog = Start-Watchdog 2400

    # The window's events until one of the kinds `$Kinds` names, which is
    # returned; the test fails when none comes in `$Seconds`.
    function Wait-Media($Window, [string[]] $Kinds, [int] $Seconds = 20) {
        $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
        $seen = [System.Collections.Generic.List[string]]::new()
        while ([DateTime]::UtcNow -lt $deadline) {
            $e = Wait-GuiEvent -Window $Window.Id -Timeout 1
            if (-not $e) { continue }
            $seen.Add([string]$e.Kind)
            if ($Kinds -contains [string]$e.Kind) { return $e }
        }
        throw "no $($Kinds -join ' or ') came within $Seconds s; the window raised $($seen -join ', ')"
    }

    # How many of a snapshot's pixels are not black.
    function Get-Lit($Shot) {
        $lit = 0
        for ($i = 0; $i -lt $Shot.Pixels.Length; $i += 4) { if ($Shot.Pixels[$i] -gt 16 -or $Shot.Pixels[$i + 1] -gt 16 -or $Shot.Pixels[$i + 2] -gt 16) { $lit++ } }
        $lit
    }

    # How many pixels in the lower half of two snapshots of one widget
    # differ by more than a sixth of the range in a channel.
    function Get-Changed($Before, $After) {
        $changed = 0
        $end = [Math]::Min($Before.Pixels.Length, $After.Pixels.Length)
        for ($i = [int]($After.Height / 2) * $After.Width * 4; $i -lt $end; $i += 4) {
            for ($c = 0; $c -lt 3; $c++) {
                if ([Math]::Abs([int]$Before.Pixels[$i + $c] - [int]$After.Pixels[$i + $c]) -gt 42) { $changed++; break }
            }
        }
        $changed
    }

    # Seeks player P of window `$Window` to `$Seconds` and gives a snapshot
    # of it once its picture there is drawn.
    function Get-Frame($Window, [double] $Seconds) {
        Set-GuiMediaPlayer P -Window $Window.Id -Position $Seconds
        Wait-Media $Window 'Position' | Out-Null
        Start-Sleep -Milliseconds 500
        Get-GuiSnapshot -Window $Window.Id -Name P -Pixels
    }
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'A MediaPlayer in <Renderer> windows in <Mode> mode' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
        $script:Renderer = $Renderer
    }

    It 'plays <Clip> to its end with its picture and its sound' -ForEach $Plays {
        $path = Join-Path $script:Clips $Clip
        $sound = $Clip -like 'tone.*'
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -AutoPlay -AudioOnly:$sound -OnPosition {} -PositionInterval 0.25 }
        try {
            $loaded = Wait-Media $w 'Loaded', 'Error'
            $loaded.Kind | Should -Be 'Loaded' -Because ($loaded.Value | ConvertTo-Json -Compress)
            $loaded.Value.Duration | Should -BeGreaterThan 1.5
            $loaded.Value.HasAudio | Should -BeTrue
            $loaded.Value.HasVideo | Should -Be (-not $sound)
            if (-not $sound) {
                $loaded.Value.Width | Should -Be 160
                $loaded.Value.Height | Should -Be 120
            }
            Wait-Media $w 'Playing' | Out-Null
            Start-Sleep -Milliseconds 600
            if (-not $sound) {
                $shot = Get-GuiSnapshot -Window $w.Id -Name P -Pixels
                Get-Lit $shot | Should -BeGreaterThan ($shot.Width * $shot.Height / 2) -Because 'the test picture fills the player'
            }
            $heard = (Get-GuiSnapshot -Window $w.Id).Sound.P
            if ($Measured) {
                $heard.Peak | Should -BeGreaterThan 0.05 -Because 'the tone plays at an eighth of full scale'
                $heard.Seconds | Should -BeGreaterThan 0.2
            } else {
                $heard | Should -BeNullOrEmpty -Because 'this renderer does not measure the sound it hands the system'
            }
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Playing'
            $ended = Wait-Media $w 'Ended' -Seconds 15
            $ended.Value | Should -BeGreaterThan 1.5
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'raises the designed error for <Clip>, naming its format' -ForEach $Gaps {
        $path = Join-Path $script:Clips $Clip
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -AutoPlay }
        try {
            $e = Wait-Media $w 'Error', 'Ended'
            $e.Kind | Should -Be 'Error'
            $e.Value.Cause | Should -Be 'FormatNotSupported'
            $e.Value.Address | Should -Be $path
            $e.Value.Format | Should -Not -BeNullOrEmpty
            $e.Value.Message | Should -Be "$path could not be played: its format is not supported here; it holds $($e.Value.Format)"
            $remedy = if ($script:Renderer -eq 'Html') { '-Renderer Egui' } else { 'Install-Module PWRS-GUI.FFmpeg.*' }
            $e.Value.Remedy | Should -BeLike $remedy
            $e.Value.Detail | Should -Not -BeNullOrEmpty
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Failed'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'raises Error for an https clip it cannot fetch, for the network''s cause' {
        $address = 'https://127.0.0.1:9/gone.mp4'
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $address -Name P -AutoPlay }
        try {
            $e = Wait-Media $w 'Error', 'Loaded'
            $e.Kind | Should -Be 'Error'
            $e.Value.Cause | Should -Be 'Network' -Because ($e.Value | ConvertTo-Json -Compress)
            $e.Value.Address | Should -Be $address
            $e.Value.Format | Should -BeNullOrEmpty
            $e.Value.Detail | Should -Not -BeNullOrEmpty
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Failed'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        { Invoke-GuiMedia $address -Wait -Renderer $script:Renderer } | Should -Throw -ErrorId 'GuiMediaFailed*'
    }

    It 'is moved, slowed, quietened and read by Set-GuiMediaPlayer and Get-GuiMediaPlayer' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips 'tone.wav') -Name P -AudioOnly -OnPosition {} }
        try {
            Wait-Media $w 'Loaded' | Out-Null
            $state = Set-GuiMediaPlayer P -Window $w.Id -Position 1 -Rate 1.5 -Volume 0.5 -Muted -Loop:$false -PassThru
            $state.GetType().FullName | Should -Be 'PwrsGui.MediaPlayer'
            $state.Position | Should -BeGreaterOrEqual 0.9
            $state.Rate | Should -Be 1.5
            $state.Volume | Should -Be 0.5
            $state.Muted | Should -BeTrue
            (Wait-Media $w 'Position').Value | Should -BeGreaterOrEqual 0.9 -Because 'a seek raises Position where it went'
            Set-GuiMediaPlayer P -Window $w.Id -Play
            Start-Sleep -Milliseconds 400
            if ($Measured) {
                (Get-GuiSnapshot -Window $w.Id).Sound.P.Peak | Should -Be 0 -Because 'a muted player hands the system silence'
            }
            Wait-Media $w 'Ended' | Out-Null
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Ended'
            { Set-GuiMediaPlayer P -Window $w.Id -Rate 5 } | Should -Throw -ErrorId 'GuiBadMedia*'
            { Set-GuiMediaPlayer P -Window $w.Id -Play -Pause } | Should -Throw -ErrorId 'GuiBadMedia*'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'raises Position every -PositionInterval while it plays' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips 'tone.wav') -Name P -AudioOnly -AutoPlay -OnPosition {} -PositionInterval 0.5 }
        try {
            $positions = 0
            $deadline = [DateTime]::UtcNow.AddSeconds(15)
            while ([DateTime]::UtcNow -lt $deadline) {
                $e = Wait-GuiEvent -Window $w.Id -Timeout 1
                if ($e.Kind -eq 'Position') { $positions++ }
                if ($e.Kind -eq 'Ended') { break }
            }
            $positions | Should -BeGreaterOrEqual 3 -Because 'two seconds at a half-second interval, and the end'
            $positions | Should -BeLessOrEqual 7
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'shows the cues of -Subtitles <Sidecar> over the picture at their times' -ForEach @(@{ Sidecar = 'cues.srt' }, @{ Sidecar = 'cues.vtt' }) {
        $clip = Join-Path $script:Clips ($Plays.Clip | Where-Object { $_ -notlike 'tone.*' } | Select-Object -First 1)
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $clip -Name P -NoControls -OnPosition {} }
        try {
            Wait-Media $w 'Loaded' | Out-Null
            $early = Get-Frame $w 0.2
            $bare = Get-Frame $w 1
            Update-GuiWidget -Window $w.Id -Name P -Property @{ Subtitles = (Join-Path $script:Clips $Sidecar) }
            Wait-Until { (Get-Changed $bare (Get-GuiSnapshot -Window $w.Id -Name P -Pixels)) -gt 150 } 'the cue showing over the picture at 1 s'
            Get-Changed $early (Get-Frame $w 0.2) | Should -BeLessThan 50 -Because 'no cue shows before 0.5 s'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        { New-GuiMediaPlayer -Path $clip -Subtitles (Join-Path $script:Clips 'tone.wav') } | Should -Throw -ErrorId 'GuiBadProperty*'
    }

    # A renderer whose player reports no clip's own text track here has no
    # case, which Pester refuses as an empty list.
    if (@($InBand).Count -gt 0) {
        It 'shows the cue of <Clip>''s own text track over the picture at 1 s' -ForEach $InBand {
            $frames = foreach ($file in $Clip, $Plain) {
                $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips $file) -Name P -NoControls -OnPosition {} }
                try {
                    Wait-Media $w 'Loaded' | Out-Null
                    Get-Frame $w 1
                } finally {
                    Close-GuiWindow -Window $w.Id
                }
            }
            Get-Changed $frames[1] $frames[0] | Should -BeGreaterThan 150 -Because "the cue of $Clip's own track shows over the picture $Plain shares with it"
        }
    }

    It 'is played by Invoke-GuiMedia in a window of its own' {
        $clip = if ($Plays.Clip -contains 'h264-aac.mp4') { 'h264-aac.mp4' } else { 'vp9-opus.webm' }
        $w = Invoke-GuiMedia (Join-Path $script:Clips $clip) -Renderer $script:Renderer
        try {
            $w.Title | Should -Be $clip
            (Get-GuiMediaPlayer Player -Window $w.Id).Phase | Should -BeIn 'Paused', 'Playing'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        $took = Measure-Command { Invoke-GuiMedia (Join-Path $script:Clips $clip) -Wait -Renderer $script:Renderer }
        $took.TotalSeconds | Should -BeGreaterThan 1.5
        $took.TotalSeconds | Should -BeLessThan 15
        { Invoke-GuiMedia (Join-Path $script:Clips $Gaps[0].Clip) -Wait -Renderer $script:Renderer } | Should -Throw -ErrorId 'GuiMediaUnplayable*'
    }

    It 'plays a clip''s sound with Invoke-GuiMedia -AudioOnly and no window shown' {
        $w = Invoke-GuiMedia (Join-Path $script:Clips 'tone.wav') -AudioOnly -Renderer $script:Renderer
        try {
            $w.Title | Should -Be 'tone.wav'
            Wait-Until { (Get-GuiMediaPlayer Player -Window $w.Id).Phase -eq 'Playing' } 'the sound playing' | Out-Null
            (Get-GuiMediaPlayer Player -Window $w.Id).HasAudio | Should -BeTrue
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        $took = Measure-Command { Invoke-GuiMedia (Join-Path $script:Clips 'tone.wav') -AudioOnly -Wait -Renderer $script:Renderer }
        $took.TotalSeconds | Should -BeGreaterThan 1.5
        $took.TotalSeconds | Should -BeLessThan 15
    }

    It 'refuses Get-GuiMediaPlayer on a widget of another kind' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' { Text 'Hi' -Name T }
        try {
            { Get-GuiMediaPlayer T -Window $w.Id } | Should -Throw -ErrorId 'GuiNotAMediaPlayer*'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }
}
