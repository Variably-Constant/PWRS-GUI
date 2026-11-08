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
    # the gap on every Windows host. On Linux and the BSDs both renderers'
    # gap is a ProRes clip, which only GStreamer's libav plugin decodes;
    # H.264 is in no list there, since OpenH264 and faad play it where
    # installed.
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
            Egui = @{ Plays = @('vp8-vorbis.webm', 'vp9-opus.webm', 'theora-vorbis.ogv', 'mjpeg-pcm.avi', 'tone.wav', 'tone.flac', 'tone.ogg', 'tone.opus', 'tone.mp3'); Gaps = @('prores.mov') }
            Html = @{ Plays = @('vp8-vorbis.webm', 'vp9-opus.webm', 'theora-vorbis.ogv', 'tone.wav', 'tone.flac', 'tone.ogg', 'tone.opus', 'tone.mp3'); Gaps = @('prores.mov') }
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
        # Whether the renderer measures the sound a player sends the
        # system: egui windows through each backend's own tap, on every
        # platform here, and HTML windows through Web Audio on WebView2 and
        # WebKit. On WebKitGTK an HTML window measures none, for the reason
        # `Unmeasured` gives, so each player's sound reads null there.
        $c.Measured = $c.Renderer -eq 'Egui' -or $windows -or $IsMacOS
        $c.Unmeasured = if (-not $c.Measured) { 'an HTML window on WebKitGTK measures no player''s sound, since a player routed through Web Audio there plays ahead of real time' }
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
    # of it once its picture there is drawn. A seek's end comes before its
    # picture, and a picture from before the seek is as lit as the new one,
    # so the time goes first. A renderer may draw no picture for longer, so
    # the snapshot then waits for the test picture to fill the player.
    function Get-Frame($Window, [double] $Seconds) {
        Set-GuiMediaPlayer P -Window $Window.Id -Position $Seconds
        Wait-Media $Window 'Position' | Out-Null
        Start-Sleep -Milliseconds 500
        $frame = @{}
        Wait-Until { $frame.Shot = Get-GuiSnapshot -Window $Window.Id -Name P -Pixels; (Get-Lit $frame.Shot) -gt ($frame.Shot.Width * $frame.Shot.Height / 2) } 'the test picture filling the player' | Out-Null
        $frame.Shot
    }

    # Whether player P of window `$Window` shows a cue over the test picture:
    # a snapshot that differs from `$Bare` with the picture filling the
    # player. A snapshot that came back with no picture differs from any
    # picture, so a change counts only with the picture there.
    function Test-CueShown($Window, $Bare) {
        $shot = Get-GuiSnapshot -Window $Window.Id -Name P -Pixels
        (Get-Lit $shot) -gt ($shot.Width * $shot.Height / 2) -and (Get-Changed $Bare $shot) -gt 150
    }

    # Watches player P of window `$Window` start its clip over twice, a
    # Position below the last being a start over, and gives how many times it
    # raised Playing. Two starts over are the fewest that show a repeat late
    # in a pass. A looping player's clip never ends for whoever follows it.
    function Get-LoopPlaying($Window) {
        $seen = @{ Playing = 0; Over = 0; Position = 0.0 }
        Wait-Until {
            $e = Wait-GuiEvent -Window $Window.Id -Timeout 1
            if ($e.Kind -eq 'Ended') { throw 'a looping player raised Ended' }
            if ($e.Kind -eq 'Playing') { $seen.Playing++ }
            if ($e.Kind -eq 'Position') {
                if ($e.Value -lt $seen.Position) { $seen.Over++ }
                $seen.Position = $e.Value
            }
            $seen.Over -ge 2
        } 'the clip starting over twice' | Out-Null
        $seen.Playing
    }

    # On Windows, the render sessions of a process and every process below
    # it, on every active endpoint, each muted or unmuted in the mixer: each
    # one's process, program, state (1 active) and the peak its meter reads
    # now, which it reads before the session's mute. A session spanning
    # processes answers AUDCLNT_S_NO_SINGLE_PROCESS, a success code, with
    # the process that made it.
    $script:SessionsSource = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace PwrsGuiTest {
    public class Session {
        public uint ProcessId;
        public string Program;
        public int State;
        public float Peak;
    }

    public static class AudioSessions {
        [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
        class MMDeviceEnumerator { }

        [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IMMDeviceEnumerator {
            [PreserveSig] int EnumAudioEndpoints(int dataFlow, int stateMask, out IMMDeviceCollection devices);
        }

        [ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IMMDeviceCollection {
            [PreserveSig] int GetCount(out int count);
            [PreserveSig] int Item(int index, out IMMDevice device);
        }

        [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IMMDevice {
            [PreserveSig] int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
        }

        [ComImport, Guid("77AA99A0-1BD6-484F-8BC7-2C654C9A9B6F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IAudioSessionManager2 {
            [PreserveSig] int GetAudioSessionControl(IntPtr sessionGuid, int flags, out IntPtr control);
            [PreserveSig] int GetSimpleAudioVolume(IntPtr sessionGuid, int flags, out IntPtr volume);
            [PreserveSig] int GetSessionEnumerator(out IAudioSessionEnumerator sessions);
        }

        [ComImport, Guid("E2F5BB11-0570-40CA-ACDD-3AA01277DEE8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IAudioSessionEnumerator {
            [PreserveSig] int GetCount(out int count);
            [PreserveSig] int GetSession(int index, out IAudioSessionControl2 session);
        }

        [ComImport, Guid("bfb7ff88-7239-4fc9-8fa2-07c950be9c6d"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IAudioSessionControl2 {
            [PreserveSig] int GetState(out int state);
            [PreserveSig] int GetDisplayName(out IntPtr name);
            [PreserveSig] int SetDisplayName(IntPtr value, IntPtr context);
            [PreserveSig] int GetIconPath(out IntPtr path);
            [PreserveSig] int SetIconPath(IntPtr value, IntPtr context);
            [PreserveSig] int GetGroupingParam(out Guid param);
            [PreserveSig] int SetGroupingParam(IntPtr param, IntPtr context);
            [PreserveSig] int RegisterAudioSessionNotification(IntPtr client);
            [PreserveSig] int UnregisterAudioSessionNotification(IntPtr client);
            [PreserveSig] int GetSessionIdentifier(out IntPtr id);
            [PreserveSig] int GetSessionInstanceIdentifier(out IntPtr id);
            [PreserveSig] int GetProcessId(out uint pid);
        }

        [ComImport, Guid("C02216F6-8C67-4B5B-9D00-D008E73E0064"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IAudioMeterInformation {
            [PreserveSig] int GetPeakValue(out float peak);
        }

        [ComImport, Guid("87CE5498-68D6-44E5-9215-6DA47EF883D8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface ISimpleAudioVolume {
            [PreserveSig] int SetMasterVolume(float level, IntPtr context);
            [PreserveSig] int GetMasterVolume(out float level);
            [PreserveSig] int SetMute([MarshalAs(UnmanagedType.Bool)] bool mute, IntPtr context);
            [PreserveSig] int GetMute([MarshalAs(UnmanagedType.Bool)] out bool mute);
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct ProcessEntry {
            public uint Size;
            public uint Usage;
            public uint ProcessId;
            public IntPtr DefaultHeapId;
            public uint ModuleId;
            public uint Threads;
            public uint ParentProcessId;
            public int PriorityBase;
            public uint Flags;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string ExeFile;
        }

        [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint processId);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool Process32FirstW(IntPtr snapshot, ref ProcessEntry entry);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool Process32NextW(IntPtr snapshot, ref ProcessEntry entry);
        [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
        [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint processId);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetProcessTimes(IntPtr process, out long creation, out long exit, out long kernel, out long user);

        // When pid started, as a FILETIME, or -1 where that cannot be read.
        static long StartOf(uint pid) {
            IntPtr handle = OpenProcess(0x1000, false, pid);
            if (handle == IntPtr.Zero) return -1;
            try {
                long start, exit, kernel, user;
                return GetProcessTimes(handle, out start, out exit, out kernel, out user) ? start : -1;
            } finally {
                CloseHandle(handle);
            }
        }

        static void Check(int hr, string call) {
            if (hr < 0) throw new InvalidOperationException(call + " failed with 0x" + hr.ToString("X8"));
        }

        static Dictionary<uint, KeyValuePair<uint, string>> Processes() {
            var all = new Dictionary<uint, KeyValuePair<uint, string>>();
            IntPtr snapshot = CreateToolhelp32Snapshot(2, 0);
            if (snapshot == new IntPtr(-1)) throw new InvalidOperationException("CreateToolhelp32Snapshot failed with " + Marshal.GetLastWin32Error());
            try {
                var entry = new ProcessEntry { Size = (uint)Marshal.SizeOf(typeof(ProcessEntry)) };
                for (bool more = Process32FirstW(snapshot, ref entry); more; more = Process32NextW(snapshot, ref entry)) {
                    all[entry.ProcessId] = new KeyValuePair<uint, string>(entry.ParentProcessId, entry.ExeFile);
                }
            } finally {
                CloseHandle(snapshot);
            }
            return all;
        }

        // Each render session of root or a process below it, muted first,
        // or unmuted, as mute says. A process counts as below its parent only
        // when it started after it: Windows reuses process ids, so another
        // program's process can name one of ours as its parent, and its
        // sessions are left alone.
        public static List<Session> SetMuteBelow(uint root, bool mute) {
            var all = Processes();
            var below = new HashSet<uint> { root };
            var starts = new Dictionary<uint, long>();
            for (bool grew = true; grew; ) {
                grew = false;
                foreach (var p in all) {
                    if (below.Contains(p.Key) || !below.Contains(p.Value.Key) || p.Key == p.Value.Key) continue;
                    if (!starts.ContainsKey(p.Key)) starts[p.Key] = StartOf(p.Key);
                    if (!starts.ContainsKey(p.Value.Key)) starts[p.Value.Key] = StartOf(p.Value.Key);
                    long child = starts[p.Key], parent = starts[p.Value.Key];
                    if (child < 0 || parent < 0 || child < parent) continue;
                    below.Add(p.Key);
                    grew = true;
                }
            }
            var found = new List<Session>();
            var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumerator();
            IMMDeviceCollection devices;
            Check(enumerator.EnumAudioEndpoints(0, 1, out devices), "EnumAudioEndpoints");
            int count;
            Check(devices.GetCount(out count), "GetCount");
            var iid = typeof(IAudioSessionManager2).GUID;
            for (int d = 0; d < count; d++) {
                IMMDevice device;
                Check(devices.Item(d, out device), "Item");
                object manager;
                Check(device.Activate(ref iid, 23, IntPtr.Zero, out manager), "Activate");
                IAudioSessionEnumerator sessions;
                Check(((IAudioSessionManager2)manager).GetSessionEnumerator(out sessions), "GetSessionEnumerator");
                int n;
                Check(sessions.GetCount(out n), "GetCount");
                for (int i = 0; i < n; i++) {
                    IAudioSessionControl2 control;
                    Check(sessions.GetSession(i, out control), "GetSession");
                    uint pid;
                    if (control.GetProcessId(out pid) < 0 || !below.Contains(pid)) continue;
                    Check(((ISimpleAudioVolume)control).SetMute(mute, IntPtr.Zero), "SetMute");
                    int state;
                    Check(control.GetState(out state), "GetState");
                    float peak;
                    Check(((IAudioMeterInformation)control).GetPeakValue(out peak), "GetPeakValue");
                    found.Add(new Session { ProcessId = pid, Program = all.ContainsKey(pid) ? all[pid].Value : "?", State = state, Peak = peak });
                }
            }
            return found;
        }
    }
}
'@

    # What a host of its own runs for Get-SilenceArm, its arm given as JSON
    # in PWRS_GUI_TEST_SILENCE, which it inherits.
    $script:SilenceArm = @'
$ErrorActionPreference = 'Stop'
$arm = $env:PWRS_GUI_TEST_SILENCE | ConvertFrom-Json
$env:PWRS_GUI_NO_SOUND = $arm.Value
$windows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
if ($windows) { Add-Type -TypeDefinition $arm.Sessions }
Import-Module (Join-Path $env:PWRS_MODULE 'PWRS-GUI.psd1')
Set-GuiOption -Mode $arm.Mode
if ($arm.Renderer -eq 'Html') { Set-GuiOption -WebView PerProcess }
$clip = $arm.Clip
$read = @{ Measured = $false; Low = $null; Events = [System.Collections.Generic.List[string]]::new(); Sessions = @{} }
function Read-Sessions([bool] $Unmute) {
    if (-not $windows) { return }
    $listed = [PwrsGuiTest.AudioSessions]::SetMuteBelow([uint32]$PID, -not $Unmute)
    foreach ($s in $listed) {
        $key = "$($s.ProcessId) $($s.Program)"
        if (-not $read.Sessions.ContainsKey($key)) { $read.Sessions[$key] = @{ Program = $s.Program; Active = $false; Peak = 0.0 } }
        if ($s.State -eq 1) { $read.Sessions[$key].Active = $true }
        if ($s.Peak -gt $read.Sessions[$key].Peak) { $read.Sessions[$key].Peak = [double]$s.Peak }
    }
    $listed
}
$w = Show-GuiWindow -NoWait -Renderer $arm.Renderer -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $clip -Name P -AutoPlay -AudioOnly }
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $ended = $false
    while (-not $ended -and [DateTime]::UtcNow -lt $deadline) {
        $e = Wait-GuiEvent -Window $w.Id -Timeout $arm.Interval
        if ($e) {
            $read.Events.Add([string]$e.Kind)
            $ended = [string]$e.Kind -in 'Ended', 'Error'
        }
        $heard = if (-not $ended) { (Get-GuiSnapshot -Window $w.Id).Sound.P }
        if ($heard) {
            $read.Measured = $true
            if ($heard.Peak -gt 0.05 -and $heard.Seconds -gt 0.2 -and ($null -eq $read.Low -or $heard.Peak -lt $read.Low)) { $read.Low = [double]$heard.Peak }
        }
        $null = Read-Sessions $false
    }
    # Once the tone has ended, the sessions are unmuted when every one reads
    # silent or inactive, and always before the window closes, so Windows
    # stores no mute for its program. An HTML window's session stays active
    # while its page's audio runs, so there its silence ends the wait.
    while ([DateTime]::UtcNow -lt $deadline -and @(Read-Sessions $false | Where-Object { $_.State -eq 1 -and $_.Peak -gt 0 }).Count -gt 0) { Start-Sleep -Milliseconds ([int]($arm.Interval * 1000)) }
} finally {
    try {
        $null = Read-Sessions $true
    } finally {
        Close-GuiWindow -Window $w.Id
    }
}
$read | ConvertTo-Json -Depth 4 -Compress
'@

    # Plays tone.wav to its end in a host of its own, under PWRS_GUI_NO_SOUND
    # $Value, in a $Renderer window in $Mode, and gives what it read every
    # $Interval seconds while the tone played: whether the player's Sound was
    # measured, the lowest peak it read before Ended with the tone above
    # 0.05 for more than 0.2 s, as the plays case waits for it, the player's
    # events, and on Windows each render session of that host and every
    # process below it, by process and program, with whether it was active
    # and the highest peak its meter read. The clip's abrupt start and end
    # ring above the steady tone's peak, and a read catches them or misses
    # them as its timing falls, so in an egui window, whose tap measures
    # every block, that lowest peak is the steady tone's in every run. An
    # HTML window's page samples its analyzer every 40 ms instead, so no
    # read of its peak repeats exactly from run to run. An HTML window's web
    # data is that host's own, so its WebView2 processes are below it. On
    # Windows that host mutes its sessions in the mixer as they appear,
    # their meters reading through the mute, so nothing is heard whatever
    # the variable does, and after the tone has ended unmutes each once it
    # reads silent or inactive, before the window closes, so Windows stores
    # no mute.
    # How far apart two runs' lowest peaks of the tone may read in an HTML
    # window: its page samples the analyzer every 40 ms, and the steady
    # tone's peak drifts by float steps between those samples.
    $script:PageDrift = 1e-6

    function Get-SilenceArm([string] $Value, [string] $Renderer, [string] $Mode, [double] $Interval) {
        $env:PWRS_GUI_TEST_SILENCE = @{ Value = $Value; Renderer = $Renderer; Mode = $Mode; Clip = (Join-Path $script:Clips 'tone.wav'); Sessions = $script:SessionsSource; Interval = $Interval } | ConvertTo-Json -Compress
        try {
            $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script:SilenceArm))
            $out = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -EncodedCommand $encoded 2>&1)
        } finally {
            Remove-Item -LiteralPath Env:\PWRS_GUI_TEST_SILENCE
        }
        $json = @($out | Where-Object { "$_".StartsWith('{') }) | Select-Object -Last 1
        if (-not $json) { throw "the host playing PWRS_GUI_NO_SOUND $Value wrote no reading: $($out -join "`n")" }
        "$json" | ConvertFrom-Json
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
            # The phase is read as the clip starts, since a clip of two
            # seconds may end during the waits below.
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Playing'
            if ($Measured) {
                # A renderer's measure may hear a clip's sound a while after
                # the clip plays, so the case waits for the tone, which plays
                # at an eighth of full scale, and for the seconds it counts.
                # The peak looks back half a second and a clip's last picture
                # stays, so the tone comes first.
                Wait-Until { $heard = (Get-GuiSnapshot -Window $w.Id).Sound.P; $heard.Peak -gt 0.05 -and $heard.Seconds -gt 0.2 } 'the tone heard above 0.05 for more than 0.2 s' | Out-Null
            } else {
                $heard = (Get-GuiSnapshot -Window $w.Id).Sound
                $heard.Contains('P') | Should -BeTrue -Because 'a snapshot gives each media player''s sound, measured or not'
                $heard.P | Should -BeNullOrEmpty -Because $Unmeasured
            }
            if (-not $sound) {
                # A renderer may draw a clip's first picture a while after the
                # clip plays, so the case waits for the test picture to fill
                # the player.
                Wait-Until { $shot = Get-GuiSnapshot -Window $w.Id -Name P -Pixels; (Get-Lit $shot) -gt ($shot.Width * $shot.Height / 2) } 'the test picture filling the player' | Out-Null
            }
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
        { Invoke-GuiMedia $address -Renderer $script:Renderer } | Should -Throw -ErrorId 'GuiMediaFailed*'
    }

    # Under PWRS_GUI_NO_SOUND 1, which Common.ps1 sets for every suite, the
    # player measures the tone it plays as under 0, and on Windows hands the
    # system silence: each of its render sessions reads a zero peak while
    # active, where under 0 the same sessions read the tone. The steady
    # tone's peak is the same exactly in an egui window, and within the
    # page's drift in an HTML one. Each arm plays in a host of its own,
    # which mutes its sessions in the mixer, their meters reading through
    # the mute, so nothing is heard under 0, nor under 1 should it fail.
    It 'measures its sound under PWRS_GUI_NO_SOUND 1 as under 0 and hands the system silence' {
        $windows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
        $on = Get-SilenceArm '1' $Renderer $Mode 0.025
        $off = Get-SilenceArm '0' $Renderer $Mode 0.025
        foreach ($arm in $on, $off) { $arm.Events | Should -Contain 'Ended' }
        if ($Measured) {
            foreach ($arm in $on, $off) { $arm.Low | Should -Not -BeNullOrEmpty -Because 'the tone plays at an eighth of full scale for 2 s' }
            if ($Renderer -eq 'Egui') {
                $on.Low | Should -Be $off.Low -Because 'the player measures its sound before it hands the system silence'
            } else {
                [Math]::Abs([double]$on.Low - [double]$off.Low) | Should -BeLessOrEqual $script:PageDrift -Because "the page measures its sound before it hands the system silence: $($on.Low) under 1, $($off.Low) under 0"
            }
        } else {
            $on.Measured | Should -BeFalse -Because $Unmeasured
            $off.Measured | Should -BeFalse -Because $Unmeasured
        }
        if ($windows) {
            $heard = @($off.Sessions.PSObject.Properties.Value | Where-Object Active)
            $heard.Count | Should -BeGreaterThan 0 -Because 'the tone plays through a session of the host'
            ($heard | Measure-Object Peak -Maximum).Maximum | Should -BeGreaterThan 0 -Because 'a session''s meter reads the tone under 0'
            $silent = @($on.Sessions.PSObject.Properties.Value | Where-Object Active)
            $silent.Count | Should -BeGreaterThan 0 -Because 'the tone plays through a session of the host under 1 too'
            foreach ($s in $silent) { $s.Peak | Should -Be 0 -Because "the session of $($s.Program) is handed silence" }
        }
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
                (Get-GuiSnapshot -Window $w.Id).Sound.P.Peak | Should -Be 0 -Because 'a muted player sends the system silence'
            } else {
                (Get-GuiSnapshot -Window $w.Id).Sound.P | Should -BeNullOrEmpty -Because $Unmeasured
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

    # On WebKitGTK the page restarts every looping player itself. The next
    # two cases loop a clip alone and with cues.
    It 'plays its clip again at each end with -Loop' {
        $clip = Join-Path $script:Clips ($Plays.Clip | Where-Object { $_ -notlike 'tone.*' } | Select-Object -First 1)
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $clip -Name P -NoControls -AutoPlay -Loop -Muted -OnPosition {} -PositionInterval 0.25 }
        try {
            Get-LoopPlaying $w | Should -Be 1
            $state = Get-GuiMediaPlayer P -Window $w.Id
            $state.Phase | Should -Be 'Playing'
            $state.Loop | Should -BeTrue
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'plays its clip again at each end with -Loop and -Subtitles' {
        $clip = Join-Path $script:Clips ($Plays.Clip | Where-Object { $_ -notlike 'tone.*' } | Select-Object -First 1)
        $cues = Join-Path $script:Clips 'cues.srt'
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $clip -Name P -NoControls -AutoPlay -Loop -Muted -Subtitles $cues -OnPosition {} -PositionInterval 0.25 }
        try {
            Get-LoopPlaying $w | Should -Be 1
            (Get-GuiMediaPlayer P -Window $w.Id).Loop | Should -BeTrue
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
            Wait-Until { Test-CueShown $w $bare } 'the cue showing over the picture at 1 s'
            Get-Changed $early (Get-Frame $w 0.2) | Should -BeLessThan 50 -Because 'no cue shows before 0.5 s'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        { New-GuiMediaPlayer -Path $clip -Subtitles (Join-Path $script:Clips 'tone.wav') } | Should -Throw -ErrorId 'GuiBadProperty*'
    }

    # A renderer whose player reports no clip's own text track here has no
    # case, which Pester refuses as an empty list. A clip's own cue comes
    # from its engine some time after the seek, so the case waits for it.
    if (@($InBand).Count -gt 0) {
        It 'shows the cue of <Clip>''s own text track over the picture at 1 s' -ForEach $InBand {
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips $Plain) -Name P -NoControls -OnPosition {} }
            try {
                Wait-Media $w 'Loaded' | Out-Null
                $bare = Get-Frame $w 1
            } finally {
                Close-GuiWindow -Window $w.Id
            }
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips $Clip) -Name P -NoControls -OnPosition {} }
            try {
                Wait-Media $w 'Loaded' | Out-Null
                Get-Frame $w 1 | Out-Null
                Wait-Until { Test-CueShown $w $bare } "the cue of $Clip's own track over the picture $Plain shares with it"
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }

        # On WebKitGTK an HTML window plays a muted player at volume 0 with its
        # element unmuted, and the player reads as muted at its own volume.
        It 'plays <Clip> with -Muted from its start, muted' -ForEach $InBand {
            $path = Join-Path $script:Clips $Clip
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -NoControls -AutoPlay -Muted -OnPosition {} }
            try {
                Wait-Media $w 'Playing' | Out-Null
                $state = Get-GuiMediaPlayer P -Window $w.Id
                $state.Phase | Should -Be 'Playing'
                $state.Muted | Should -BeTrue
                $state.Volume | Should -Be 1
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }

        # On WebKitGTK an HTML window plays such a clip briefly and silently
        # before its first paused seek, and the player says nothing of it.
        It 'seeks <Clip> paused raising Position alone, its volume and mute as set' -ForEach $InBand {
            $path = Join-Path $script:Clips $Clip
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -NoControls -Volume 0.5 -OnPosition {} }
            try {
                Wait-Media $w 'Loaded' | Out-Null
                $state = Set-GuiMediaPlayer P -Window $w.Id -Position 1 -PassThru
                $state.Position | Should -BeGreaterOrEqual 0.9 -Because 'the reply gives the place asked for'
                $state.Phase | Should -Be 'Paused'
                $kinds = [System.Collections.Generic.List[string]]::new()
                Wait-Until { $e = Wait-GuiEvent -Window $w.Id -Timeout 1; if ($e) { $kinds.Add([string]$e.Kind) }; $kinds -contains 'Position' } 'Position after the seek' | Out-Null
                $kinds | Should -Not -Contain 'Playing'
                $kinds | Should -Not -Contain 'Paused'
                $kinds | Should -Not -Contain 'Ended'
                $after = Get-GuiMediaPlayer P -Window $w.Id
                $after.Phase | Should -Be 'Paused'
                $after.Volume | Should -Be 0.5
                $after.Muted | Should -BeFalse
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }

        It 'plays <Clip> again at each end with -Loop, its own text track followed' -ForEach $InBand {
            $path = Join-Path $script:Clips $Clip
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -NoControls -AutoPlay -Loop -Muted -OnPosition {} -PositionInterval 0.25 }
            try {
                Get-LoopPlaying $w | Should -Be 1
                (Get-GuiMediaPlayer P -Window $w.Id).Loop | Should -BeTrue
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }
    }

    # Without -Wait the command writes the window once the clip is playing,
    # so the line after it reads Playing; a clip that cannot be played ends
    # it with the same error as with -Wait.
    It 'is played by Invoke-GuiMedia in a window of its own' {
        $clip = if ($Plays.Clip -contains 'h264-aac.mp4') { 'h264-aac.mp4' } else { 'vp9-opus.webm' }
        $w = Invoke-GuiMedia (Join-Path $script:Clips $clip) -Renderer $script:Renderer
        try {
            (Get-GuiMediaPlayer Player -Window $w.Id).Phase | Should -Be 'Playing'
            $w.Title | Should -Be $clip
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        $took = Measure-Command { Invoke-GuiMedia (Join-Path $script:Clips $clip) -Wait -Renderer $script:Renderer }
        $took.TotalSeconds | Should -BeGreaterThan 1.5
        $took.TotalSeconds | Should -BeLessThan 15
        { Invoke-GuiMedia (Join-Path $script:Clips $Gaps[0].Clip) -Wait -Renderer $script:Renderer } | Should -Throw -ErrorId 'GuiMediaUnplayable*'
        { Invoke-GuiMedia (Join-Path $script:Clips $Gaps[0].Clip) -Renderer $script:Renderer } | Should -Throw -ErrorId 'GuiMediaUnplayable*'
    }

    It 'plays a clip''s sound with Invoke-GuiMedia -AudioOnly and no window shown' {
        $w = Invoke-GuiMedia (Join-Path $script:Clips 'tone.wav') -AudioOnly -Renderer $script:Renderer
        try {
            (Get-GuiMediaPlayer Player -Window $w.Id).Phase | Should -Be 'Playing'
            $w.Title | Should -Be 'tone.wav'
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
