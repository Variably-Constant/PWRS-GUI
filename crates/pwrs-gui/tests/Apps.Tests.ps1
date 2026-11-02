# What a page of the caller's own may reach and do as an application, in
# every mode the platform has. The page's policy: without -Connect or
# -ContentSecurityPolicy it reaches only its own origin; -Connect adds the
# origins it may fetch from and nothing else; -ContentSecurityPolicy
# replaces the policy whole, and -AllowNavigation keeps a policy given; a
# source or a policy the page cannot carry is refused before any window
# opens, and the binder refuses the two together. A fetch to a .invalid
# address fails without leaving the machine, so what the tests read is
# whether the policy refused it. A page's window.pwrs.call: what -Call
# allows runs, a command by its name and a script block by the name given
# it, an object splatted and an array positional; an error rejects it as
# -CallErrors says; anything else, a call from an origin the window does
# not trust and a call in a window that allows nothing are refused with an
# Error; calls are numbered with sends, and a -NoWait window runs them as
# engine-event actions. A page from another origin is served from a
# loopback socket of the test's own. A page's drops of files: its ondrop
# handlers get the full paths, then -OnDrop where the page leaves the drop,
# as an engine-event action with -NoWait; text is refused. A test's drop
# gives paths that need not exist. A page's own window: it takes its
# document's title without -Title, is named, resized, moved and filled
# from script, the page told of each, and its onclose handlers hold its
# person's close but never the session's, for as long as the page keeps it,
# while a close the page gives no answer in 10 seconds closes the window. An
# app of several windows: its
# pages open windows of its folder and of HTML text, which the command
# answers as it answers its first and waits for, its result the first one's;
# -EndWithFirstWindow closes the rest with the first; a page lists its app's
# windows and acts on them but on no other app's; what an app cannot open is
# refused; and with -NoWait, Get-GuiWindow lists a later window, whose sends
# reach -OnMessage with it as the second argument. A page's channels: text
# messages both ways over rings a script made, a long one in parts; plain
# frames over a stream New-GuiChannel made, whose folder goes once the app
# has ended; a channel ended by its process's exit; and what is refused.
# The backend's side of a channel takes SubEtha's module, which
# SUBETHA_MODULE names. A page without the system's frame:
# maximized, restored and closed from script; a move its window cannot make
# reported; a press on its title bar or its edges reported under a test's
# input as the move it starts, which is not made; and the box of the
# system's buttons over it.
# A widget window's own CSS: -StylePath's file, then -Style's text, after
# the runtime's own, selecting widgets by their kind and name. See
# Common.ps1 for the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
    # A page that closes with what its policy did to a fetch to an address
    # that names no host and to an eval: "refused <directive>" or "allowed"
    # for the fetch, then "eval" or "no eval".
    $script:Probe = @'
<script>
var done = false;
function evals() { try { return eval('1') === 1 ? 'eval' : 'no eval'; } catch (e) { return 'no eval'; } }
function finish(fetched) { if (!done) { done = true; window.pwrs.close(fetched + ' ' + evals()); } }
document.addEventListener('securitypolicyviolation', function (e) { if (e.effectiveDirective !== 'script-src') { finish('refused ' + (e.effectiveDirective || e.violatedDirective)); } });
fetch('https://connect.invalid/').then(function () { finish('allowed'); }, function () { setTimeout(function () { finish('allowed'); }, 500); });
</script>
'@
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'A page''s policy in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
    }

    It 'reaches only its own origin without -Connect or -ContentSecurityPolicy' {
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test own origin' -Html @'`n$script:Probe`n'@"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -BeLike 'refused *src eval' -Because 'the default policy keeps the page on its own origin and lets it eval'
    }

    It 'adds the origins -Connect names to what the page may fetch, and changes nothing else' {
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test connect' -Connect 'https://connect.invalid', 'wss://events.invalid:8443' -Html @'`n$script:Probe`n'@"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'allowed eval' -Because '-Connect adds to connect-src alone'
    }

    It 'gives a folder''s files the policy -Connect makes' {
        $site = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-connect-$PID"
        New-Item -ItemType Directory -Path $site -Force | Out-Null
        try {
            Set-Content -LiteralPath (Join-Path $site 'index.html') -Value $script:Probe
            $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test connect folder' -Path '$site' -Connect 'https:'"
            $done = Complete-Background $job
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[0].Result | Should -Be 'allowed eval'
        } finally {
            Remove-Item -LiteralPath $site -Recurse -Force
        }
    }

    It 'replaces the policy whole with -ContentSecurityPolicy' {
        $policy = "default-src 'self' 'unsafe-inline'; connect-src https://connect.invalid"
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test policy' -ContentSecurityPolicy `"$policy`" -Html @'`n$script:Probe`n'@"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'allowed no eval' -Because 'the policy given allows the fetch and, lacking unsafe-eval, no eval'
    }

    It 'keeps a policy given on the page''s own files with -AllowNavigation, which alone drops it' {
        $policy = "default-src 'self' 'unsafe-inline'"
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test policy navigate' -AllowNavigation -ContentSecurityPolicy `"$policy`" -Html @'`n$script:Probe`n'@"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -BeLike 'refused *src no eval'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test navigate' -AllowNavigation -Html @'`n$script:Probe`n'@"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'allowed eval' -Because 'a page that may navigate carries no policy of the module''s'
    }

    It 'refuses a source that would change more than connect-src, and a policy a header cannot carry, before any window opens' {
        foreach ($source in "https://a.invalid; script-src *", "'unsafe-eval'", 'https://a.invalid,https://b.invalid', 'https://[::1]:8080') {
            { Show-GuiWindow -Html '<p/>' -Connect $source } | Should -Throw -ErrorId 'GuiBadPolicy*' -Because "$source is no source"
        }
        { Show-GuiWindow -Html '<p/>' -ContentSecurityPolicy "default-src 'self'`nX-Other: 1" } | Should -Throw -ErrorId 'GuiBadPolicy*'
        { Show-GuiWindow -Html '<p/>' -ContentSecurityPolicy '  ' } | Should -Throw -ErrorId 'GuiBadPolicy*'
        Get-GuiWindow | Should -BeNullOrEmpty -Because 'no window opened'
    }

    It 'takes -Connect and -ContentSecurityPolicy in parameter sets of their own, so the binder refuses the two together' {
        { Show-GuiWindow -Html '<p/>' -Connect 'https:' -ContentSecurityPolicy "default-src 'self'" } | Should -Throw -ErrorId 'AmbiguousParameterSet*'
        { Show-GuiWindow -Path '.' -Connect 'https:' -ContentSecurityPolicy "default-src 'self'" } | Should -Throw -ErrorId 'AmbiguousParameterSet*'
        $sets = (Get-Command Show-GuiWindow).ParameterSets
        foreach ($name in 'Html', 'Path') {
            @($sets | Where-Object Name -eq $name).Parameters.Name | Should -Not -Contain 'Connect'
            (@($sets | Where-Object Name -eq "${name}Connect").Parameters | Where-Object Name -eq Connect).IsMandatory | Should -BeTrue
            (@($sets | Where-Object Name -eq "${name}Policy").Parameters | Where-Object Name -eq ContentSecurityPolicy).IsMandatory | Should -BeTrue
        }
        Get-GuiWindow | Should -BeNullOrEmpty -Because 'no window opened'
    }
}

Describe 'A page''s calls in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
        # What a page that waits for its window's command writes: the page
        # $Page, in a window given $Arguments too.
        function Invoke-CallPage([string] $Title, [string] $Page, [string] $Arguments) {
            $job = Start-Background ("`$page = @'`n" + $Page + "`n'@`n" + "Show-GuiWindow -Title '$Title' -Html `$page $Arguments")
            Complete-Background $job
        }
        # A page that closes with what its call of $Call gave: the output's
        # JSON, or the rejection's Error as JSON, its message, errorId,
        # category and output and whether it is an Error.
        function Get-CallPage([string] $Call) {
            "<script>$Call.then(function (r) { window.pwrs.close('resolved ' + JSON.stringify(r)); }, function (e) { window.pwrs.close(JSON.stringify({ message: e.message, errorId: e.errorId, category: e.category, output: e.output, isError: e instanceof Error })); });</script>"
        }
        # Serves $Html at / of a loopback port of its own, as another origin,
        # until Stop-PageServer; anything else it is asked for is not found.
        function Start-PageServer([string] $Html) {
            $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
            $listener.Start()
            $ps = [powershell]::Create()
            [void]$ps.AddScript({
                param($Listener, [string] $Html)
                $body = [System.Text.Encoding]::UTF8.GetBytes($Html)
                while ($true) {
                    $client = $Listener.AcceptTcpClient()
                    try {
                        $stream = $client.GetStream()
                        $reader = [System.IO.StreamReader]::new($stream, [System.Text.Encoding]::ASCII)
                        $first = $reader.ReadLine()
                        do { $line = $reader.ReadLine() } while ($line)
                        $page = $first -like 'GET / *'
                        $content = if ($page) { $body } else { [byte[]]::new(0) }
                        $status = if ($page) { '200 OK' } else { '404 Not Found' }
                        $head = [System.Text.Encoding]::ASCII.GetBytes("HTTP/1.1 $status`r`nContent-Type: text/html; charset=utf-8`r`nContent-Length: $($content.Length)`r`nConnection: close`r`n`r`n")
                        $stream.Write($head, 0, $head.Length)
                        $stream.Write($content, 0, $content.Length)
                        $stream.Flush()
                    } finally {
                        $client.Close()
                    }
                }
            }).AddArgument($listener).AddArgument($Html)
            $async = $ps.BeginInvoke()
            [pscustomobject]@{ Port = $listener.LocalEndpoint.Port; Listener = $listener; PS = $ps; Async = $async }
        }
        # Stops the listener, which ends the server's wait for a connection,
        # and with it the server.
        function Stop-PageServer($Server) {
            $Server.Listener.Stop()
            $Server.PS.Stop()
            $Server.PS.Dispose()
        }
    }

    It 'runs a command -Call names, called by its name whatever its case, an object splatted as its named arguments' {
        $page = Get-CallPage 'window.pwrs.call("get-date", { Date: "2026-01-02T03:04:05", Format: "yyyy-MM-dd" })'
        $done = Invoke-CallPage 'PWRS-GUI test call command' $page "-Call 'Get-Date'"
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'resolved "2026-01-02"'
    }

    It 'runs the script block -Call gives a name to, an array positional, another value one argument, and output as -OnMessage''s' {
        $page = Get-CallPage 'Promise.all([window.pwrs.call("Join", ["x", "y"]), window.pwrs.call("Twice", 21), window.pwrs.call("Three"), window.pwrs.call("Nothing")])'
        $done = Invoke-CallPage 'PWRS-GUI test call blocks' $page '-Call @{ Join = { param($a, $b) "$a+$b" }; Twice = { param($n) $n * 2 }; Three = { 1; 2; 3 }; Nothing = { } }'
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'resolved ["x+y",42,[1,2,3],null]'
    }

    It 'rejects a call at its first error with the error''s message, id and category and the output written before it' {
        $page = Get-CallPage 'window.pwrs.call("Fail")'
        $done = Invoke-CallPage 'PWRS-GUI test call fails' $page '-Call @{ Fail = { ''before''; Write-Error ''it broke'' -ErrorId Broke -Category InvalidData; ''after'' } }'
        $done.Errors | Should -BeNullOrEmpty -Because 'the page is given the error'
        $done.Output[0].Result | Should -Be '{"message":"it broke","errorId":"Broke","category":"InvalidData","output":"before","isError":true}'
    }

    It 'with -CallErrors Terminating resolves a call past an error it writes, which the command writes as its own' {
        $page = Get-CallPage 'window.pwrs.call("Fail")'
        $done = Invoke-CallPage 'PWRS-GUI test call terminating' $page '-CallErrors Terminating -Call @{ Fail = { ''before''; Write-Error ''it broke'' -ErrorId Broke -Category InvalidData; ''after'' } }'
        $done.Output[0].Result | Should -Be 'resolved ["before","after"]'
        @($done.Errors).Count | Should -Be 1
        $done.Errors[0].Exception.Message | Should -Be 'it broke'
        $done.Errors[0].FullyQualifiedErrorId | Should -BeLike 'Broke*'
    }

    It 'rejects a call that throws with what it threw and the output before it, with -CallErrors <Errors>' -ForEach @(@{ Errors = 'Any' }, @{ Errors = 'Terminating' }) {
        $page = Get-CallPage 'window.pwrs.call("Throw")'
        $done = Invoke-CallPage "PWRS-GUI test call throws $Errors" $page "-CallErrors $Errors -Call @{ Throw = { 'out'; throw 'boom' } }"
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be '{"message":"boom","errorId":"boom","category":"OperationStopped","output":"out","isError":true}'
    }

    It 'refuses a call of a name -Call does not give, naming it' {
        $page = Get-CallPage 'window.pwrs.call("Remove-Item", { Path: "C:/" })'
        $done = Invoke-CallPage 'PWRS-GUI test call not allowed' $page "-Call 'Get-Date'"
        $done.Errors | Should -BeNullOrEmpty
        ($done.Output[0].Result | ConvertFrom-Json).message | Should -Be 'this window allows no call named Remove-Item'
    }

    It 'runs a -NoWait window''s calls as engine-event actions once the engine is idle' {
        $script = @'
$global:said = $null
$page = '<script>window.pwrs.call("Add", [2, 3]).then(function (r) { window.pwrs.send(r); });</script>'
$w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test call nowait' -Html $page -Call @{ Add = { param($a, $b) $a + $b } } -OnMessage { param($m) $global:said = $m }
$deadline = [DateTime]::UtcNow.AddSeconds(15)
while ($null -eq $global:said -and [DateTime]::UtcNow -lt $deadline) { $null = Wait-Event -Timeout 1 }
Close-GuiWindow -Window $w.Id
"said $global:said"
'@
        $done = Complete-Background (Start-Background $script)
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1] | Should -Be 'said 5'
    }

    It 'refuses a call from an origin -AllowNavigation admits, and runs it with -TrustNavigation' {
        $server = Start-PageServer (Get-CallPage 'window.pwrs.call("Get-Date", { Format: "yyyy" })')
        try {
            $leave = "<script>location.href = 'http://127.0.0.1:$($server.Port)/';</script>"
            $done = Invoke-CallPage 'PWRS-GUI test call visitor' $leave "-AllowNavigation -Call 'Get-Date'"
            $done.Errors | Should -BeNullOrEmpty
            ($done.Output[0].Result | ConvertFrom-Json).message | Should -BeLike "http://127.0.0.1:$($server.Port)/*not the page's own origin*-TrustNavigation*"
            $done = Invoke-CallPage 'PWRS-GUI test call trusted visitor' $leave "-AllowNavigation -TrustNavigation -Call 'Get-Date'"
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[0].Result | Should -Be "resolved `"$((Get-Date).Year)`""
        } finally {
            Stop-PageServer $server
        }
    }

    It 'gives a page from an origin the window does not trust no dropped paths, while -OnDrop still runs, and a trusted one the paths' {
        $served = @'
<title>PWRS-GUI test drop visitor</title>
<script>
var got = 'none';
window.pwrs.ondrop(function (d) { got = d.paths.length + ' paths'; });
window.pwrs.on(function (v) { window.pwrs.close(v + ', page given ' + got); });
</script>
'@
        $server = Start-PageServer $served
        try {
            $leave = "<script>location.href = 'http://127.0.0.1:$($server.Port)/';</script>"
            foreach ($run in @{ Switches = '-AllowNavigation'; Expected = 'command ran, page given none' }, @{ Switches = '-AllowNavigation -TrustNavigation'; Expected = 'command ran, page given 2 paths' }) {
                $job = Start-Background ("`$leave = @'`n" + $leave + "`n'@`nShow-GuiWindow -Html `$leave $($run.Switches) -OnDrop { Send-GuiMessage -Window `$_.Window -Message 'command ran' }")
                $w = Wait-GuiWindow 'PWRS-GUI test drop visitor' $job
                Invoke-GuiTestEvent -Window $w.Id -Drop 'first.txt', 'second.txt'
                $done = Complete-Background $job
                $done.Errors | Should -BeNullOrEmpty
                $done.Output[0].Result | Should -Be $run.Expected -Because $run.Switches
            }
        } finally {
            Stop-PageServer $server
        }
    }

    It 'refuses a -Call value that names no command or gives no script block, a name twice, and -TrustNavigation alone, before any window opens' {
        foreach ($bad in 'No-SuchCommandAnywhere', 'Get-*', '', 42, @{ X = 'not a block' }, @{ '' = { } }) {
            { Show-GuiWindow -Html '<p/>' -Call $bad } | Should -Throw -ErrorId 'GuiBadCall*' -Because "$bad allows nothing"
        }
        { Show-GuiWindow -Html '<p/>' -Call 'Get-Date', @{ 'get-date' = { } } } | Should -Throw -ErrorId 'GuiBadCall*'
        { Show-GuiWindow -Html '<p/>' -TrustNavigation } | Should -Throw -ErrorId 'GuiBadNavigation*'
    }

    It 'rejects a call in a window that allows nothing to be called, with an Error naming it' {
        $page = '<script>window.pwrs.call("Get-Date", { Format: "o" }).then(function () { window.pwrs.close("resolved"); }, function (e) { window.pwrs.close(e instanceof Error ? e.message : "not an Error"); });</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test call refused' -Html '$page'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -BeLike '*allows nothing*Get-Date*'
    }

    It 'rejects a call that names nothing to run without asking the session' {
        $page = '<script>window.pwrs.call("").then(function () { window.pwrs.close("resolved"); }, function (e) { window.pwrs.close(e.name); });</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test call nameless' -Html '$page'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'TypeError'
    }

    It 'numbers sends and calls from one count, so each answer reaches its own Promise' {
        $page = '<script>Promise.all([window.pwrs.send(1), window.pwrs.call("X").catch(function (e) { return "refused"; }), window.pwrs.send(2)]).then(function (r) { window.pwrs.close(JSON.stringify(r)); });</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test call and send' -Html '$page' -OnMessage { param(`$m) `$m * 10 }"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be '[10,"refused",20]'
    }
}

Describe 'A page''s drops in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
        # Two files' full paths, which a test's drop gives without their
        # being there.
        $script:Dropped = (Join-Path ([IO.Path]::GetTempPath()) 'pwrs-gui-report.txt'), (Join-Path ([IO.Path]::GetTempPath()) 'pwrs-gui-data.csv')
        # Runs $Script, which shows a page titled $Title, in the background,
        # drops $script:Dropped on the page once it is open, and returns what
        # the command wrote.
        function Invoke-DropPage([string] $Title, [string] $Script) {
            $job = Start-Background $Script
            $w = Wait-GuiWindow $Title $job
            Invoke-GuiTestEvent -Window $w.Id -Drop $script:Dropped
            Complete-Background $job
        }
    }

    It 'gives the page''s ondrop handlers the full paths of files dropped, then -OnDrop them where the page leaves the drop' {
        $script = @'
$page = @"
<script>
var seen = [];
window.pwrs.ondrop(function (d) { seen.push('page ' + d.paths.join(';') + ' taken ' + d.taken); });
window.pwrs.on(function (v) { seen.push(v); window.pwrs.close(seen.join(' | ')); });
</script>
"@
Show-GuiWindow -Title 'PWRS-GUI test drop' -Html $page -OnDrop { Send-GuiMessage -Window $_.Window -Message "command $($_.Kind) $($_.Value -join ';')" }
'@
        $done = Invoke-DropPage 'PWRS-GUI test drop' $script
        $done.Errors | Should -BeNullOrEmpty
        $joined = $script:Dropped -join ';'
        $done.Output[0].Result | Should -Be "page $joined taken false | command Drop $joined"
    }

    It 'leaves -OnDrop out of a drop the page takes with preventDefault, whose ondrop handlers still get the paths' {
        $script = @'
$page = @"
<script>
document.addEventListener('drop', function (e) { e.preventDefault(); });
window.pwrs.ondrop(function (d) { setTimeout(function () { window.pwrs.close('page ' + d.paths.length + ' taken ' + d.taken); }, 300); });
window.pwrs.on(function (v) { window.pwrs.close(v); });
</script>
"@
Show-GuiWindow -Title 'PWRS-GUI test drop taken' -Html $page -OnDrop { Send-GuiMessage -Window $_.Window -Message 'command ran' }
'@
        $done = Invoke-DropPage 'PWRS-GUI test drop taken' $script
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'page 2 taken true'
    }

    It 'runs a -NoWait page''s -OnDrop as an engine-event action once the engine is idle' {
        $script = @'
$global:said = $null
$w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test drop nowait' -Html '<p>Drop here</p>' -OnDrop { $global:said = "$($_.Kind) $(@($_.Value).Count)" }
Invoke-GuiTestEvent -Window $w.Id -Drop @('first.txt', 'second.txt')
$deadline = [DateTime]::UtcNow.AddSeconds(15)
while ($null -eq $global:said -and [DateTime]::UtcNow -lt $deadline) { $null = Wait-Event -Timeout 1 }
Close-GuiWindow -Window $w.Id
"said $global:said"
'@
        $done = Complete-Background (Start-Background $script)
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1] | Should -Be 'said Drop 2'
    }

    It 'refuses text dropped on a page, which takes files' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test drop text' -Html '<p>Files only</p>'
        try {
            Invoke-GuiTestEvent -Window $w.Id -Drop 'some text' -ErrorVariable failed -ErrorAction SilentlyContinue
            $failed[0].FullyQualifiedErrorId | Should -BeLike 'GuiRefused*'
            $failed[0].Exception.Message | Should -BeLike '*list of their paths*'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }
}

Describe 'A page without the system''s frame in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
    }

    # A window manager maximizes a window on X11, and the X servers the
    # suites run under on Linux and FreeBSD run none.
    It 'maximizes and restores its window from script, and closes it as its person would' -Skip:($IsLinux -or $PSVersionTable.OS -match 'FreeBSD') {
        $page = @'
<script>
var first = [innerWidth, innerHeight], seen = [];
function sized() { return innerWidth + 'x' + innerHeight; }
function settled(moved, then) {
  var start = Date.now(), last = null, since = 0;
  (function look() {
    var now = sized();
    if (now !== last) { last = now; since = Date.now(); }
    if ((moved() && Date.now() - since >= 300) || Date.now() - start > 5000) { then(); return; }
    setTimeout(look, 50);
  })();
}
window.pwrs.window.maximize();
settled(function () { return innerWidth > first[0] && innerHeight > first[1]; }, function () {
  seen.push(innerWidth > first[0] && innerHeight > first[1] ? 'larger' : 'not larger at ' + sized());
  window.pwrs.window.restore();
  settled(function () { return innerWidth === first[0] && innerHeight === first[1]; }, function () {
    seen.push(innerWidth === first[0] && innerHeight === first[1] ? 'back' : 'not back at ' + sized());
    window.pwrs.send(seen.join(' ')).then(function () { window.pwrs.window.close(); });
  });
});
</script>
'@
        $script = @'
$global:seen = $null
$r = Show-GuiWindow -Title 'PWRS-GUI test frameless' -Frameless -Width 400 -Height 300 -OnMessage { param($m) $global:seen = $m } -Html $page
"$global:seen | $($r.Result)"
'@
        $job = Start-Background ("`$page = @'`n" + $page + "`n'@`n" + $script)
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0] | Should -Be 'larger back | Closed' -Because 'a frameless page''s close is its person''s close, whose result is Closed'
    }

    It 'reports a move its window cannot make to the window''s command' {
        $page = '<script>window.pwrs.window.startResize("middle"); setTimeout(function () { window.pwrs.close("done"); }, 300);</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test frameless refused' -Frameless -Html '$page'"
        $done = Complete-Background $job
        $done.Output[-1].Result | Should -Be 'done'
        @($done.Warnings | Where-Object { $_ -like '*resizes from n, ne*middle*' }).Count | Should -Be 1 -Because 'the page asked for an edge no window has'
    }

    It 'reports a press on a title bar marked data-pwrs-drag as the move it starts under a test''s input, and makes none, but for what keeps its presses' {
        $page = @'
<style>
  body { margin: 0; }
  #bar { display: flex; height: 40px; }
  #bar span { flex: 1; }
  #bar button, #bar i { width: 60px; }
</style>
<div id="bar" data-pwrs-drag><span>Title</span><button>Close</button><i data-pwrs-no-drag>Menu</i></div>
<p>Body</p>
'@
        $w = Show-GuiWindow -NoWait -Frameless -Title 'PWRS-GUI test drag region' -Width 400 -Height 300 -Html $page
        try {
            $moving = [ordered]@{}
            foreach ($at in @{ What = 'title'; X = 100; Y = 20 }, @{ What = 'button'; X = 310; Y = 20 }, @{ What = 'no-drag'; X = 370; Y = 20 }, @{ What = 'body'; X = 100; Y = 120 }) {
                Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $at.X; Y = $at.Y }, @{ Release = 'Left'; X = $at.X; Y = $at.Y }
                $moving[$at.What] = "$((Get-GuiSnapshot -Window $w.Id).Moving)"
            }
            $moving.title | Should -Be 'drag' -Because 'a press on the bar starts the system''s move, which a test does not make'
            $moving.button | Should -Be '' -Because 'a button inside the bar keeps its presses'
            $moving['no-drag'] | Should -Be '' -Because 'what is marked data-pwrs-no-drag keeps its presses'
            $moving.body | Should -Be '' -Because 'the page outside the bar moves nothing'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    # A window manager maximizes a window on X11, and the X servers the
    # suites run under on Linux and FreeBSD run none.
    It 'maximizes its window at a double-click on its title bar, and restores it at the next' -Skip:($IsLinux -or $PSVersionTable.OS -match 'FreeBSD') {
        $w = Show-GuiWindow -NoWait -Frameless -Title 'PWRS-GUI test drag double' -Width 400 -Height 300 -Html '<body style="margin: 0"><div data-pwrs-drag style="height: 40px"></div></body>'
        try {
            # A second's wait first, so the double-click starts a count of
            # clicks of its own.
            $double = @{ Wait = 1 }, @{ Press = 'Left'; X = 100; Y = 20 }, @{ Release = 'Left'; X = 100; Y = 20 }, @{ Press = 'Left'; X = 100; Y = 20 }, @{ Release = 'Left'; X = 100; Y = 20 }
            $before = (Get-GuiSnapshot -Window $w.Id -Pixels).Width
            Invoke-GuiTestEvent -Window $w.Id -Input $double
            Wait-Until { (Get-GuiSnapshot -Window $w.Id -Pixels).Width -gt $before } 'the window''s maximize at a double-click on its title bar'
            Invoke-GuiTestEvent -Window $w.Id -Input $double
            Wait-Until { (Get-GuiSnapshot -Window $w.Id -Pixels).Width -eq $before } 'the window''s restore at the next double-click'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'starts the system''s resize from the edges the bridge places over the page, under a test''s input as the resize it names' {
        $w = Show-GuiWindow -NoWait -Frameless -Title 'PWRS-GUI test edges' -Width 400 -Height 300 -Html '<body style="margin: 0"><p>Body</p></body>'
        try {
            $seen = foreach ($at in @{ X = 200; Y = 2 }, @{ X = 2; Y = 2 }, @{ X = 2; Y = 150 }, @{ X = 397; Y = 297 }) {
                Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $at.X; Y = $at.Y }, @{ Release = 'Left'; X = $at.X; Y = $at.Y }
                "$((Get-GuiSnapshot -Window $w.Id).Moving)"
            }
            # The system's frame resizes a window on macOS, and on Windows
            # the system's shadow does at every edge but the top, which lies
            # inside the page.
            $expected = if ($IsMacOS) {
                '', '', '', ''
            } elseif ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
                'resize n', 'resize nw', '', ''
            } else {
                'resize n', 'resize nw', 'resize w', 'resize se'
            }
            $seen | Should -Be $expected
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'gives the box of the system''s own buttons over the page on macOS, and none elsewhere, once hidden or with the system''s frame' {
        $page = '<script>window.pwrs.close(JSON.stringify(window.pwrs.window.buttons));</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test buttons' -Frameless -Html '$page'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        if ($IsMacOS) {
            $box = $done.Output[0].Result | ConvertFrom-Json
            $box.x | Should -BeGreaterOrEqual 0
            $box.y | Should -BeGreaterOrEqual 0
            $box.width | Should -BeGreaterThan 0
            $box.height | Should -BeGreaterThan 0
        } else {
            $done.Output[0].Result | Should -Be 'null'
        }
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test buttons hidden' -Frameless -HideWindowButtons -Html '$page'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'null' -Because '-HideWindowButtons hides them'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test buttons framed' -Html '$page'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'null' -Because 'a window with the system''s frame has its buttons in the frame, not over the page'
    }

    It 'refuses -HideWindowButtons without -Frameless' {
        { Show-GuiWindow -Html '<p/>' -HideWindowButtons } | Should -Throw -ErrorId 'GuiBadFrame*'
    }
}

Describe 'A page''s own window in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
        # Waits until the size of the page's inside settles once `moved()`
        # holds, or 5 s pass, then runs `then`.
        $script:Settled = @'
function sized() { return innerWidth + 'x' + innerHeight; }
function settled(moved, then) {
  var start = Date.now(), last = null, since = 0;
  (function look() {
    var now = sized();
    if (now !== last) { last = now; since = Date.now(); }
    if ((moved() && Date.now() - since >= 300) || Date.now() - start > 5000) { then(); return; }
    setTimeout(look, 50);
  })();
}
'@
    }

    It 'takes its document''s title without -Title, and keeps a -Title given' {
        $w = Show-GuiWindow -NoWait -Html '<script>document.title = "PWRS-GUI test document title";</script>'
        try {
            Wait-Until { Get-GuiWindow -Title 'PWRS-GUI test document title' } 'the window taking its document''s title'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test own title' -Html '<script>document.title = "PWRS-GUI test passed over";</script>'
        try {
            Start-Sleep -Milliseconds 500
            (Get-GuiWindow -Title 'PWRS-GUI test own title').Id | Should -Be $w.Id
            Get-GuiWindow -Title 'PWRS-GUI test passed over' | Should -BeNullOrEmpty
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'names its window from script, which Get-GuiWindow finds it by' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test set title' -Html '<script>window.pwrs.window.setTitle("PWRS-GUI test title set");</script>'
        try {
            Wait-Until { Get-GuiWindow -Title 'PWRS-GUI test title set' } 'the title the page set'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'resizes and moves its window from script, and is told of each' {
        $page = @'
<script>
var seen = {};
window.pwrs.window.on('resize', function (d) { seen.resize = d; });
window.pwrs.window.on('move', function (d) { seen.move = d; });
window.pwrs.window.resize(420, 300);
setTimeout(function () {
  window.pwrs.window.move(screenX + 10, screenY + 10);
  setTimeout(function () { window.pwrs.close({ width: innerWidth, height: innerHeight, resized: seen.resize || null, moved: !!seen.move }); }, 800);
}, 800);
</script>
'@
        $done = Complete-Background (Start-Background ("`$page = @'`n" + $page + "`n'@`nShow-GuiWindow -Title 'PWRS-GUI test move' -Html `$page"))
        $done.Errors | Should -BeNullOrEmpty
        $r = $done.Output[0].Result
        "$($r.width)x$($r.height)" | Should -Be '420x300'
        "$($r.resized.width)x$($r.resized.height)" | Should -Be '420x300' -Because 'the page is told of the resize'
        $r.moved | Should -BeTrue -Because 'the page is told of the move'
    }

    It 'keeps its window open at a close its onclose handler holds, closes at one it lets go, and never holds a close of the session''s' {
        $script = @'
$global:said = [System.Collections.Generic.List[string]]::new()
$page = @"
<script>
var asked = 0;
window.pwrs.window.onclose(function (e) { asked++; if (asked === 1) { e.preventDefault(); window.pwrs.send('kept'); } });
window.pwrs.window.close();
setTimeout(function () { window.pwrs.window.close(); }, 500);
</script>
"@
$r = Show-GuiWindow -Title 'PWRS-GUI test onclose' -Html $page -OnMessage { param($m) $global:said.Add($m) }
"$($global:said -join ',') | $($r.Result)"
'@
        $done = Complete-Background (Start-Background $script)
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1] | Should -Be 'kept | Closed'
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test onclose session' -Html '<script>window.pwrs.window.onclose(function (e) { e.preventDefault(); });</script>'
        Close-GuiWindow -Window $w.Id
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI test onclose session') } 'the session''s close'
    }

    It 'closes a window whose page gives no answer to its person''s close in 10 seconds, and says so' {
        $page = '<script>window.pwrs.window.onclose(function () { throw new Error("broken"); }); window.pwrs.window.close();</script>'
        $started = [DateTime]::UtcNow
        $done = Complete-Background (Start-Background "Show-GuiWindow -Title 'PWRS-GUI test onclose silent' -Html '$page'")
        $waited = ([DateTime]::UtcNow - $started).TotalSeconds
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'Closed' -Because 'a close the page does not answer is its person''s'
        $waited | Should -BeGreaterOrEqual 9 -Because 'the page has 10 seconds to answer'
        $done.Warnings | Where-Object { $_ -like '*did not answer*in 10 s*' } | Should -Not -BeNullOrEmpty
    }

    It 'keeps a window open past 10 seconds when its page answers its person''s close by keeping it' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test onclose kept' -Html '<script>window.pwrs.window.onclose(function (e) { e.preventDefault(); }); window.pwrs.window.close();</script>'
        try {
            Start-Sleep -Seconds 11
            Get-GuiWindow -Title 'PWRS-GUI test onclose kept' | Should -Not -BeNullOrEmpty -Because 'a close the page kept is held as long as the page keeps it'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    # A window manager fills a monitor with a window on X11, and the X
    # servers the suites run under on Linux and FreeBSD run none.
    It 'fills its monitor from script and comes back' -Skip:($IsLinux -or $PSVersionTable.OS -match 'FreeBSD') {
        $page = "<script>`n$script:Settled`n" + @'
var first = [innerWidth, innerHeight], seen = [];
window.pwrs.window.fullscreen(true);
settled(function () { return innerWidth > first[0] && innerHeight > first[1]; }, function () {
  seen.push(innerWidth > first[0] && innerHeight > first[1] ? 'filled' : 'not filled at ' + sized());
  window.pwrs.window.fullscreen(false);
  settled(function () { return innerWidth === first[0] && innerHeight === first[1]; }, function () {
    seen.push(innerWidth === first[0] && innerHeight === first[1] ? 'back' : 'not back at ' + sized());
    window.pwrs.close(seen.join(' '));
  });
});
</script>
'@
        $done = Complete-Background (Start-Background ("`$page = @'`n" + $page + "`n'@`nShow-GuiWindow -Title 'PWRS-GUI test fullscreen' -Width 400 -Height 300 -Html `$page"))
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'filled back'
    }
}

Describe 'An app of several windows in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
        # What the command that shows the page $Page, titled $Title with
        # $Arguments too, wrote once it returned.
        function Invoke-AppPage([string] $Title, [string] $Page, [string] $Arguments) {
            Complete-Background (Start-Background ("`$page = @'`n" + $Page + "`n'@`n" + "Show-GuiWindow -Title '$Title' -Html `$page $Arguments"))
        }
    }

    It 'opens windows of its folder and of HTML text, answers each as it answers the first, and waits for the last, writing the first one''s result' {
        $folder = Join-Path $TestDrive 'several'
        $null = New-Item -ItemType Directory -Path $folder
        Set-Content -LiteralPath (Join-Path $folder 'index.html') -Encoding utf8 -Value @'
<script>
var third = '<scr' + 'ipt>window.pwrs.send("third").then(function () { window.pwrs.close(); });</scr' + 'ipt>';
window.pwrs.window.open({ path: 'second.html', title: 'PWRS-GUI test app second', width: 360, height: 240 }).then(function (second) {
  return window.pwrs.window.open({ html: third }).then(function () {
    return window.pwrs.send('opened ' + typeof second.id + ' ' + (second.id !== window.pwrs.window.id) + ' ' + second.title);
  });
}).then(function () { window.pwrs.close('first'); }, function (e) { window.pwrs.close('refused: ' + e.message); });
</script>
'@
        Set-Content -LiteralPath (Join-Path $folder 'second.html') -Encoding utf8 -Value @'
<script>
setTimeout(function () {
  window.pwrs.call('Twice', 21).then(function (n) { return window.pwrs.send('second ' + n); }).then(function () { window.pwrs.close('second'); });
}, 800);
</script>
'@
        $script = "`$folder = '$folder'`n" + @'
$global:said = [System.Collections.Generic.List[string]]::new()
$r = Show-GuiWindow -Path $folder -Title 'PWRS-GUI test app first' -Call @{ Twice = { param($n) $n * 2 } } -OnMessage { param($m, $sender) $global:said.Add("$m | $($sender.Title)") }
$global:said
"result $($r.Result)"
'@
        $done = Complete-Background (Start-Background $script)
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1] | Should -Be 'result first' -Because 'the app''s result is its first window''s, though a later one closed last'
        $said = @($done.Output | Select-Object -SkipLast 1)
        $said | Should -Contain 'opened string true PWRS-GUI test app second | PWRS-GUI test app first'
        $said | Should -Contain 'third | PWRS-GUI' -Because 'a window opened without a title opens as PWRS-GUI'
        $said | Should -Contain 'second 42 | PWRS-GUI test app second' -Because 'a later window calls what -Call allows, and sends after the first window closed'
    }

    It 'closes the app''s other windows with its first with -EndWithFirstWindow' {
        $page = @'
<script>
window.pwrs.window.open({ html: '<title>PWRS-GUI test app left</title><p>left</p>' }).then(function () {
  var tries = 0;
  (function look() {
    window.pwrs.window.list().then(function (all) {
      var seen = all.some(function (w) { return w.title === 'PWRS-GUI test app left'; });
      if (seen || ++tries > 40) { window.pwrs.close(seen ? 'done' : 'not seen'); return; }
      setTimeout(look, 100);
    });
  })();
});
</script>
'@
        $done = Invoke-AppPage 'PWRS-GUI test app ending' $page '-EndWithFirstWindow'
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'done' -Because 'the later window took its document''s title'
        Get-GuiWindow -Title 'PWRS-GUI test app left' | Should -BeNullOrEmpty -Because 'the later window closed with the first'
    }

    It 'lists its app''s windows and acts on one of them, and is refused a window of another app' {
        $other = Show-GuiWindow -NoWait -Title 'PWRS-GUI test app other' -Html '<p>other</p>'
        try {
            $page = @'
<script>
window.pwrs.window.open({ html: '<p>two</p>', title: 'PWRS-GUI test app two' }).then(function (two) {
  return window.pwrs.window.list().then(function (all) {
    two.setTitle('PWRS-GUI test app renamed');
    window.pwrs.window.of(OTHER).setTitle('PWRS-GUI test app taken');
    setTimeout(function () {
      window.pwrs.window.list().then(function (now) {
        two.close();
        window.pwrs.close(all.length + ' ' + now.map(function (w) { return w.title; }).sort().join(','));
      });
    }, 500);
  });
});
</script>
'@ -replace 'OTHER', "'$($other.Id)'"
            $done = Invoke-AppPage 'PWRS-GUI test app lister' $page ''
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[0].Result | Should -Be '2 PWRS-GUI test app lister,PWRS-GUI test app renamed'
            $done.Warnings | Where-Object { $_ -like '*is no open window of this page''s app*' } | Should -Not -BeNullOrEmpty
            Get-GuiWindow -Title 'PWRS-GUI test app taken' | Should -BeNullOrEmpty -Because 'a window of another app is not the page''s to rename'
        } finally {
            Close-GuiWindow -Window $other.Id
        }
    }

    It 'refuses to open a path in an app of HTML text or out of its folder, an address it does not trust, and options it does not take' {
        $tries = @'
<script>
var said = [];
function tried(options) { return window.pwrs.window.open(options).then(function () { said.push('opened'); }, function (e) { said.push(e.message); }); }
var all = TRIES;
all.reduce(function (before, options) { return before.then(function () { return tried(options); }); }, Promise.resolve()).then(function () { window.pwrs.close(said.join(' | ')); });
</script>
'@
        $page = $tries -replace 'TRIES', '[{ path: "other.html" }, { url: "https://example.invalid/" }, { html: "<p/>", width: 50 }, { html: "<p/>", color: "red" }]'
        $done = Invoke-AppPage 'PWRS-GUI test app refusals' $page ''
        $done.Errors | Should -BeNullOrEmpty
        $said = $done.Output[0].Result -split ' \| '
        $said[0] | Should -BeLike '*is a path*HTML text*'
        $said[1] | Should -BeLike '*-AllowNavigation and -TrustNavigation*'
        $said[2] | Should -BeLike '*from 100 to 10000 points*'
        $said[3] | Should -BeLike '*not color*'
        $folder = Join-Path $TestDrive 'refusing'
        $null = New-Item -ItemType Directory -Path $folder
        Set-Content -LiteralPath (Join-Path $folder 'index.html') -Encoding utf8 -Value ($tries -replace 'TRIES', '[{ path: "../outside.html" }, { path: "missing.html" }]')
        $done = Complete-Background (Start-Background "Show-GuiWindow -Title 'PWRS-GUI test app folder refusals' -Path '$folder'")
        $done.Errors | Should -BeNullOrEmpty
        $said = $done.Output[0].Result -split ' \| '
        $said[0] | Should -BeLike '*leaves the app''s folder*'
        $said[1] | Should -BeLike '*no file of the app''s folder*'
    }

    It 'with -NoWait, lets Get-GuiWindow list a later window and runs its sends through the same engine event, the window the handler''s second argument' {
        $page = @'
<script>
var later = '<scr' + 'ipt>window.pwrs.send("hello");</scr' + 'ipt>';
window.pwrs.window.open({ html: later, title: 'PWRS-GUI test app later' });
</script>
'@
        $script = @'
$global:said = $null
$w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test app nowait' -Html $page -OnMessage { param($m, $sender) $global:said = "$m from $($sender.Title)" }
$deadline = [DateTime]::UtcNow.AddSeconds(15)
while ($null -eq $global:said -and [DateTime]::UtcNow -lt $deadline) { $null = Wait-Event -Timeout 1 }
$later = @(Get-GuiWindow -Title 'PWRS-GUI test app later')
Close-GuiWindow -Window $w.Id
foreach ($l in $later) { Close-GuiWindow -Window $l.Id }
"said $global:said; listed $($later.Count)"
'@
        $done = Complete-Background (Start-Background ("`$page = @'`n" + $page + "`n'@`n" + $script))
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1] | Should -Be 'said hello from PWRS-GUI test app later; listed 1'
    }
}

Describe 'A page''s channels in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
        $script:Mode = $_
        # A folder of the test's own for the rings it makes as a backend.
        function New-RingFolder([string] $Name) {
            $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-$Name-$PID-$script:Mode"
            $null = New-Item -ItemType Directory -Path $base -Force
            $base
        }
        # The next frame of $Ring for $Consumer, as bytes, within 20 s.
        function Receive-Frame($Ring, $Consumer) {
            $deadline = [DateTime]::UtcNow.AddSeconds(20)
            while ([DateTime]::UtcNow -lt $deadline) {
                $frame = $Ring.RecvFrame($Consumer)
                if ($null -ne $frame) { return , [byte[]] $frame }
                Start-Sleep -Milliseconds 20
            }
            throw 'no frame came within 20 s'
        }
    }

    It 'carries text messages both ways over rings a script made, a message longer than a frame in parts' {
        Import-Module (Join-Path $env:SUBETHA_MODULE 'SubEtha.psd1') -ErrorAction Stop
        $base = New-RingFolder 'channel'
        $at = @{ ToPage = Join-Path $base 'to-page'; FromPage = Join-Path $base 'from-page' }
        $toPage = New-SubEthaRing -Path $at.ToPage -Capacity 64
        $fromPage = New-SubEthaRing -Path $at.FromPage -Capacity 64
        try {
            $c = $fromPage.RegisterConsumer()
            $p = $toPage.RegisterProducer()
            $page = @'
<script>
var ch = window.pwrs.channel('Link'), got = [];
ch.onopen = function () { ch.send('hello').then(function () { return ch.send('x'.repeat(20000)); }); };
ch.onmessage = function (e) { got.push(typeof e.data + ' ' + e.data.length); if (got.length === 2) window.pwrs.close(got.join(', ')); };
</script>
'@
            $script = "Import-Module (Join-Path '$env:SUBETHA_MODULE' 'SubEtha.psd1')`n`$rings = @((Open-SubEthaRing -Path '$($at.ToPage)' -Capacity 64), (Open-SubEthaRing -Path '$($at.FromPage)' -Capacity 64))`n`$page = @'`n" + $page + "`n'@`n" + @'
try {
    Show-GuiWindow -Title 'PWRS-GUI test channel' -Html $page -Channel @{ Link = @{ ToPage = $rings[0]; FromPage = $rings[1]; Text = $true } }
} finally {
    foreach ($r in $rings) { $r.Dispose() }
}
'@
            $job = Start-Background $script
            $hello = Receive-Frame $fromPage $c
            $hello[0] | Should -Be 0 -Because 'a message in one frame is whole'
            [Text.Encoding]::UTF8.GetString($hello, 1, $hello.Length - 1) | Should -Be 'hello'
            $parts = @(1..3 | ForEach-Object { , (Receive-Frame $fromPage $c) })
            ($parts | ForEach-Object { $_[0] }) -join ',' | Should -Be '1,1,2' -Because 'a message longer than a frame goes as parts, the last marked so'
            ($parts | ForEach-Object { $_.Length - 1 } | Measure-Object -Sum).Sum | Should -Be 20000
            $toPage.SendFrame($p, [byte[]](@(0) + [Text.Encoding]::UTF8.GetBytes('pong'))) | Should -BeTrue
            $long = [Text.Encoding]::UTF8.GetBytes('y' * 9000)
            $toPage.SendFrame($p, [byte[]](@(1) + $long[0..8190])) | Should -BeTrue
            $toPage.SendFrame($p, [byte[]](@(2) + $long[8191..8999])) | Should -BeTrue
            $done = Complete-Background $job
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[-1].Result | Should -Be 'string 4, string 9000' -Because 'the page is given each message whole, as text'
        } finally {
            foreach ($r in $toPage, $fromPage) { $r.Dispose() }
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Wait-Until { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction Ignore; -not (Test-Path -LiteralPath $base) } 'the rings'' folder being removed once the window let go of them'
        }
    }

    It 'carries plain frames over a stream New-GuiChannel made, whose rings the backend opens by their paths, and removes them once the app has ended' {
        Import-Module (Join-Path $env:SUBETHA_MODULE 'SubEtha.psd1') -ErrorAction Stop
        $ch = New-GuiChannel -Name Lsp -Stream -MaxFrames 8
        $ch.Capacity | Should -Be 8
        $folder = Split-Path -Parent $ch.ToPage
        $toPage = Open-SubEthaRing -Path $ch.ToPage -Capacity $ch.Capacity
        $fromPage = Open-SubEthaRing -Path $ch.FromPage -Capacity $ch.Capacity
        $c = $fromPage.RegisterConsumer()
        $p = $toPage.RegisterProducer()
        $page = '<script>var ch = window.pwrs.channel("Lsp"); ch.onopen = function () { ch.send(new Uint8Array([1, 2, 3])); }; ch.onmessage = function (e) { document.title = "PWRS-GUI test stream " + Array.from(new Uint8Array(e.data)).join(","); };</script>'
        $w = Show-GuiWindow -NoWait -Html $page -Channel $ch
        try {
            (Receive-Frame $fromPage $c) -join ',' | Should -Be '1,2,3' -Because 'a stream sends plain frames'
            $toPage.SendFrame($p, [byte[]](9, 8)) | Should -BeTrue
            Wait-Until { Get-GuiWindow -Title 'PWRS-GUI test stream 9,8' } 'the page given the backend''s frame'
        } finally {
            foreach ($r in $toPage, $fromPage) { $r.Dispose() }
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Close-GuiWindow -Window $w.Id
        }
        Wait-Until { -not (Test-Path -LiteralPath $folder) } 'the channel''s folder going once the app ended'
    }

    It 'ends a channel as the process New-GuiChannel -Process ties it to exits, and the page''s onclose runs' {
        $arguments = @{ FilePath = (Get-Process -Id $PID).Path; ArgumentList = '-NoProfile', '-Command', 'Start-Sleep -Seconds 2'; PassThru = $true }
        if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { $arguments.WindowStyle = 'Hidden' }
        $sleeper = Start-Process @arguments
        $ch = New-GuiChannel -Name Tied -Process $sleeper
        $w = Show-GuiWindow -NoWait -Html '<script>var ch = window.pwrs.channel("Tied"); ch.onclose = function (e) { document.title = "PWRS-GUI test tied: " + e.reason; };</script>' -Channel $ch
        try {
            Wait-Until { Get-GuiWindow -Title 'PWRS-GUI test tied: the process the channel was tied to has exited' } 'the channel ending with its process' 30
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'refuses a channel the window does not offer, and -Text with -Stream' {
        { New-GuiChannel -Name Both -Text -Stream } | Should -Throw -ErrorId 'GuiBadChannel*'
        $page = '<script>var ch = window.pwrs.channel("Nope"); ch.onerror = function (e) { window.pwrs.close(e.message); };</script>'
        $done = Complete-Background (Start-Background "Show-GuiWindow -Title 'PWRS-GUI test channel none' -Html '$page'")
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -BeLike '*no channel named Nope*'
    }
}

Describe 'A widget window''s own CSS in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_ -Renderer Html
        # Where the button named Wide is drawn, with Show-GuiWindow given
        # $Arguments too.
        function Get-ButtonBounds([string]$Title, [string]$Arguments) {
            $job = Start-Background "Show-GuiWindow -Title '$Title' -Renderer Html $Arguments -Content { Text 'Before' ; Button 'Wide' -Name Wide -Result Ok }"
            $w = Wait-GuiWindow $Title $job
            $bounds = (Get-GuiSnapshot -Window $w.Id).Bounds.Wide
            Invoke-GuiTestEvent -Window $w.Id -Name Wide -Event Click
            $done = Complete-Background $job
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[0].Result | Should -Be 'Ok'
            $bounds
        }
    }

    It 'applies -Style after the runtime''s own styles, so a rule of the same weight wins' {
        $plain = Get-ButtonBounds 'PWRS-GUI test style plain' ''
        $styled = Get-ButtonBounds 'PWRS-GUI test style' "-Style 'button { margin-left: 100px; }'"
        ($styled.X - $plain.X) | Should -BeGreaterOrEqual 99 -Because 'the runtime''s rule for buttons sets their margin to 0, and the window''s own rule of the same weight comes after it'
        ($styled.X - $plain.X) | Should -BeLessOrEqual 101
        $styled.Width | Should -Be $plain.Width
    }

    It 'applies -StylePath''s file before -Style''s text, each selecting widgets by their kind and name' {
        $sheet = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-style-$PID.css"
        Set-Content -LiteralPath $sheet -Value '[data-kind="Button"] { margin-left: 50px; }'
        try {
            $plain = Get-ButtonBounds 'PWRS-GUI test style file plain' ''
            $file = Get-ButtonBounds 'PWRS-GUI test style file' "-StylePath '$sheet'"
            $both = Get-ButtonBounds 'PWRS-GUI test style file and text' "-StylePath '$sheet' -Style '[data-name=Wide] { margin-left: 100px; }'"
        } finally {
            Remove-Item -LiteralPath $sheet
        }
        ($file.X - $plain.X) | Should -BeGreaterOrEqual 49 -Because 'the file''s rule selects every button by its kind'
        ($file.X - $plain.X) | Should -BeLessOrEqual 51
        ($both.X - $plain.X) | Should -BeGreaterOrEqual 99 -Because '-Style''s rule, as strong as the file''s, comes after it and selects the button by its name'
        ($both.X - $plain.X) | Should -BeLessOrEqual 101
    }

    It 'refuses a -StylePath that names no file' {
        $missing = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-no-such-style-$PID.css"
        { Show-GuiWindow -Renderer Html -StylePath $missing -Content { Button 'Go' } } | Should -Throw -ErrorId 'GuiNoFile*'
    }
}
